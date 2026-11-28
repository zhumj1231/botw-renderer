//! Independent raster replay of the Wii U haze mask and effect draws.

use std::{fs, path::Path};

use bevy::asset::{load_internal_asset, uuid_handle};
use bevy::mesh::VertexBufferLayout;
use bevy::prelude::*;
use bevy::render::render_resource::binding_types::{sampler, texture_2d, uniform_buffer};
use bevy::render::render_resource::*;
use bevy::render::renderer::{RenderDevice, RenderQueue};

const SHADER: Handle<Shader> = uuid_handle!("0d95f1ef-775f-4fac-a79e-8651c5dad374");
const MASK_SIZE: Extent3d = extent(160, 90);
const EFFECT_SIZE: Extent3d = extent(640, 360);

const fn extent(width: u32, height: u32) -> Extent3d {
    Extent3d {
        width,
        height,
        depth_or_array_layers: 1,
    }
}

pub(super) fn register_shader(app: &mut App) {
    load_internal_asset!(app, SHADER, "haze_raster.wgsl", Shader::from_wgsl);
}

#[derive(Clone, Copy, ShaderType, bytemuck::Pod, bytemuck::Zeroable)]
#[repr(C)]
struct Params {
    palette: [f32; 4],
    alpha_ref: f32,
    _padding: [f32; 3],
}

struct IndexedDraw {
    vertices: Buffer,
    indices: Buffer,
    bindings: BindGroup,
    count: u32,
}

struct RasterPass {
    pipeline: CachedRenderPipelineId,
    base: Texture,
    target: Texture,
    view: TextureView,
    draws: Vec<IndexedDraw>,
    label: &'static str,
}

pub(super) struct HazeRasterReplay {
    mask: RasterPass,
    effect: RasterPass,
}

impl HazeRasterReplay {
    pub(super) fn load(
        path: &Path,
        device: &RenderDevice,
        queue: &RenderQueue,
        cache: &PipelineCache,
    ) -> Result<Self, String> {
        let layout = BindGroupLayoutDescriptor::new(
            "haze_raster_inputs",
            &BindGroupLayoutEntries::sequential(
                ShaderStages::FRAGMENT,
                (
                    texture_2d(TextureSampleType::Float { filterable: true }),
                    sampler(SamplerBindingType::Filtering),
                    uniform_buffer::<Params>(false),
                ),
            ),
        );
        let binding_layout = cache.get_bind_group_layout(&layout);
        let noise = texture(
            device,
            "haze_noise_bc4",
            extent(512, 512),
            TextureFormat::Bc4RUnorm,
            3,
            TextureUsages::COPY_DST | TextureUsages::TEXTURE_BINDING,
        );
        for mip in 0..3 {
            let side = 512 >> mip;
            let data = read_exact(
                path,
                &format!("noise_bc4-mip{mip}.bin"),
                (side * side / 2) as usize,
            )?;
            upload(
                queue,
                &noise,
                mip,
                &data,
                (side / 4) * 8,
                extent(side, side),
            );
        }
        let noise = noise.create_view(&default());
        let make_pass = |is_mask: bool, input: &TextureView| -> Result<RasterPass, String> {
            let (view_size, storage_size, format, bytes, attrs, label, vert, frag, snapshot) =
                if is_mask {
                    (
                        MASK_SIZE,
                        extent(192, 96),
                        TextureFormat::R16Float,
                        2,
                        6,
                        "haze_mask",
                        "mask_vertex",
                        "haze_mask",
                        "mask_base.bin",
                    )
                } else {
                    (
                        EFFECT_SIZE,
                        extent(640, 368),
                        TextureFormat::Rgba16Float,
                        8,
                        5,
                        "haze_effect",
                        "haze_vertex",
                        "haze_effect",
                        "effect_base.bin",
                    )
                };
            let base = texture(
                device,
                snapshot,
                view_size,
                format,
                1,
                TextureUsages::COPY_DST | TextureUsages::COPY_SRC,
            );
            let data = read_exact(
                path,
                snapshot,
                (storage_size.width * storage_size.height * bytes) as usize,
            )?;
            upload(
                queue,
                &base,
                0,
                &data,
                storage_size.width * bytes,
                view_size,
            );
            let target = texture(
                device,
                label,
                view_size,
                format,
                1,
                TextureUsages::COPY_DST
                    | TextureUsages::RENDER_ATTACHMENT
                    | TextureUsages::TEXTURE_BINDING,
            );
            let view = target.create_view(&default());
            let address_mode = if is_mask {
                AddressMode::Repeat
            } else {
                AddressMode::ClampToEdge
            };
            let filtering = device.create_sampler(&SamplerDescriptor {
                address_mode_u: address_mode,
                address_mode_v: address_mode,
                mag_filter: FilterMode::Linear,
                min_filter: FilterMode::Linear,
                lod_max_clamp: if is_mask { 2.0 } else { 0.0 },
                ..default()
            });
            let vertex_stride = attrs * 16;
            let events: Vec<_> = if is_mask {
                (10745..=10819).step_by(2).collect()
            } else {
                vec![13765]
            };
            let draws = events
                .into_iter()
                .map(|event| {
                    let (vertex_count, index_count) = if is_mask { (33, 144) } else { (102, 480) };
                    let vertices = read_exact(
                        path,
                        &format!("vertices-{event}.bin"),
                        vertex_count * vertex_stride,
                    )?;
                    let indices =
                        read_exact(path, &format!("indices-{event}.bin"), 4 * index_count)?;
                    let uniform_size = if is_mask { 32 } else { 48 };
                    let uniform =
                        read_exact(path, &format!("fragment-uniform-{event}.bin"), uniform_size)?;
                    let get = |i: usize| {
                        f32::from_le_bytes(uniform[i * 4..i * 4 + 4].try_into().unwrap())
                    };
                    let params = Params {
                        palette: if is_mask {
                            [0.0; 4]
                        } else {
                            [get(0), get(1), get(2), get(3)]
                        },
                        alpha_ref: get(if is_mask { 0 } else { 4 }),
                        _padding: [0.0; 3],
                    };
                    let buffer = |contents, usage| {
                        device.create_buffer_with_data(&BufferInitDescriptor {
                            label: Some(label),
                            contents,
                            usage,
                        })
                    };
                    let uniform = buffer(bytemuck::bytes_of(&params), BufferUsages::UNIFORM);
                    Ok(IndexedDraw {
                        vertices: buffer(&vertices, BufferUsages::VERTEX),
                        indices: buffer(&indices, BufferUsages::INDEX),
                        bindings: device.create_bind_group(
                            label,
                            &binding_layout,
                            &BindGroupEntries::sequential((
                                input,
                                &filtering,
                                uniform.as_entire_binding(),
                            )),
                        ),
                        count: index_count as u32,
                    })
                })
                .collect::<Result<Vec<_>, String>>()?;
            let pipeline = cache.queue_render_pipeline(RenderPipelineDescriptor {
                label: Some(label.into()),
                layout: vec![layout.clone()],
                vertex: VertexState {
                    shader: SHADER,
                    entry_point: Some(vert.into()),
                    buffers: vec![VertexBufferLayout {
                        array_stride: vertex_stride as u64,
                        step_mode: VertexStepMode::Vertex,
                        attributes: (0..attrs as u32)
                            .map(|location| VertexAttribute {
                                format: VertexFormat::Float32x4,
                                offset: u64::from(location) * 16,
                                shader_location: location,
                            })
                            .collect(),
                    }],
                    ..default()
                },
                fragment: Some(FragmentState {
                    shader: SHADER,
                    entry_point: Some(frag.into()),
                    targets: vec![Some(ColorTargetState {
                        format,
                        blend: Some(BlendState {
                            color: BlendComponent {
                                src_factor: BlendFactor::SrcAlpha,
                                dst_factor: BlendFactor::OneMinusSrcAlpha,
                                operation: BlendOperation::Add,
                            },
                            alpha: BlendComponent {
                                src_factor: BlendFactor::Zero,
                                dst_factor: BlendFactor::OneMinusSrcAlpha,
                                operation: BlendOperation::Add,
                            },
                        }),
                        write_mask: ColorWrites::ALL,
                    })],
                    ..default()
                }),
                primitive: PrimitiveState {
                    cull_mode: (!is_mask).then_some(Face::Front),
                    ..default()
                },
                ..default()
            });
            Ok(RasterPass {
                pipeline,
                base,
                target,
                view,
                draws,
                label,
            })
        };
        let mask = make_pass(true, &noise)?;
        let effect = make_pass(false, &mask.view)?;
        Ok(Self { mask, effect })
    }

    pub(super) fn effect(&self) -> &TextureView {
        &self.effect.view
    }

    pub(super) fn draw(&self, encoder: &mut CommandEncoder, cache: &PipelineCache) -> Option<()> {
        self.draw_from(encoder, cache, None)
    }

    pub(super) fn draw_from(
        &self,
        encoder: &mut CommandEncoder,
        cache: &PipelineCache,
        live_mask: Option<&Texture>,
    ) -> Option<()> {
        let mask_pipeline = cache.get_render_pipeline(self.mask.pipeline)?;
        let effect_pipeline = cache.get_render_pipeline(self.effect.pipeline)?;
        let mask = live_mask.unwrap_or(&self.mask.base);
        if mask.size() != self.mask.target.size() {
            return None;
        }
        self.mask.draw(encoder, mask_pipeline, Some(mask));
        self.effect.draw(
            encoder,
            effect_pipeline,
            live_mask.is_none().then_some(&self.effect.base),
        );
        Some(())
    }
}

impl RasterPass {
    fn draw(
        &self,
        encoder: &mut CommandEncoder,
        pipeline: &RenderPipeline,
        base: Option<&Texture>,
    ) {
        let load = if let Some(base) = base {
            encoder.copy_texture_to_texture(
                base.as_image_copy(),
                self.target.as_image_copy(),
                self.target.size(),
            );
            LoadOp::Load
        } else {
            LoadOp::Clear(LinearRgba::BLACK.into())
        };
        let mut pass = encoder.begin_render_pass(&RenderPassDescriptor {
            label: Some(self.label),
            color_attachments: &[Some(RenderPassColorAttachment {
                view: &self.view,
                depth_slice: None,
                resolve_target: None,
                ops: Operations {
                    load,
                    store: StoreOp::Store,
                },
            })],
            ..default()
        });
        pass.set_pipeline(pipeline);
        for draw in &self.draws {
            pass.set_bind_group(0, &draw.bindings, &[]);
            pass.set_vertex_buffer(0, *draw.vertices.slice(..));
            pass.set_index_buffer(*draw.indices.slice(..), IndexFormat::Uint32);
            pass.draw_indexed(0..draw.count, 0, 0..1);
        }
    }
}

fn read_exact(directory: &Path, name: &str, expected: usize) -> Result<Vec<u8>, String> {
    let path = directory.join(name);
    let bytes = fs::read(&path).map_err(|error| format!("{}: {error}", path.display()))?;
    if bytes.len() != expected {
        return Err(format!(
            "{}: expected {expected} bytes, read {}",
            path.display(),
            bytes.len()
        ));
    }
    Ok(bytes)
}

fn texture(
    device: &RenderDevice,
    label: &str,
    size: Extent3d,
    format: TextureFormat,
    mips: u32,
    usage: TextureUsages,
) -> Texture {
    device.create_texture(&TextureDescriptor {
        label: Some(label),
        dimension: TextureDimension::D2,
        size,
        mip_level_count: mips,
        sample_count: 1,
        format,
        usage,
        view_formats: &[],
    })
}

fn upload(
    queue: &RenderQueue,
    texture: &Texture,
    mip_level: u32,
    bytes: &[u8],
    pitch: u32,
    size: Extent3d,
) {
    queue.write_texture(
        TexelCopyTextureInfo {
            texture,
            mip_level,
            origin: Origin3d::ZERO,
            aspect: TextureAspect::All,
        },
        bytes,
        TexelCopyBufferLayout {
            offset: 0,
            bytes_per_row: Some(pitch),
            rows_per_image: None,
        },
        size,
    );
}
