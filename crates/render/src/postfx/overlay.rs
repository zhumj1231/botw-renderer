//! Run the recovered HDR composite against the local frame 3830 readbacks.
//! This replay supplies the scene and effect inputs for a GPU comparison.

use bevy::asset::{load_internal_asset, uuid_handle};
use bevy::core_pipeline::FullscreenShader;
use bevy::prelude::*;
use bevy::render::render_resource::binding_types::{sampler, texture_2d};
use bevy::render::render_resource::*;
use bevy::render::renderer::{RenderDevice, RenderQueue};

const SHADER: Handle<Shader> = uuid_handle!("c7bbb279-02d8-431d-95fc-ee17763f8fef");
const FRAME: Extent3d = Extent3d {
    width: 1280,
    height: 720,
    depth_or_array_layers: 1,
};
const HALF: Extent3d = Extent3d {
    width: 640,
    height: 360,
    depth_or_array_layers: 1,
};
const FORMAT: TextureFormat = TextureFormat::Rgba16Float;

pub(super) fn register_shader(app: &mut App) {
    load_internal_asset!(app, SHADER, "overlay.wgsl", Shader::from_wgsl);
}

#[derive(Resource)]
pub(super) struct OverlayReplay {
    pipeline: CachedRenderPipelineId,
    inputs: BindGroup,
    output: TextureView,
}

pub(super) fn init(
    mut commands: Commands,
    device: Res<RenderDevice>,
    queue: Res<RenderQueue>,
    fullscreen: Res<FullscreenShader>,
    cache: Res<PipelineCache>,
) {
    let Some(path) = std::env::var_os("BOTW_OVERLAY_REPLAY") else {
        return;
    };
    let directory = std::path::Path::new(&path);
    let views = [
        ("hdr-pre", FRAME, TextureFormat::Rg11b10Ufloat, 4),
        ("effect-post", HALF, FORMAT, 8),
        ("depth-full", FRAME, TextureFormat::R16Float, 2),
        ("depth-half", HALF, TextureFormat::R32Float, 4),
    ]
    .map(|(name, size, format, pixel_bytes)| {
        let path = directory.join(format!("overlay-ab-{name}.bin"));
        let data = match std::fs::read(&path) {
            Ok(data) if data.len() == (size.width * size.height * pixel_bytes) as usize => data,
            Ok(data) => {
                error!(
                    "{}: unexpected readback size of {} bytes",
                    path.display(),
                    data.len()
                );
                return None;
            }
            Err(error) => {
                error!("{}: {error}", path.display());
                return None;
            }
        };
        let input = texture(
            &device,
            name,
            size,
            format,
            TextureUsages::COPY_DST | TextureUsages::TEXTURE_BINDING,
        );
        queue.write_texture(
            TexelCopyTextureInfo {
                texture: &input,
                mip_level: 0,
                origin: Origin3d::ZERO,
                aspect: TextureAspect::All,
            },
            &data,
            TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(size.width * pixel_bytes),
                rows_per_image: None,
            },
            size,
        );
        Some(input.create_view(&TextureViewDescriptor::default()))
    });
    let [
        Some(scene),
        Some(effect),
        Some(full_depth),
        Some(half_depth),
    ] = views
    else {
        return;
    };
    let unfiltered = TextureSampleType::Float { filterable: false };
    let layout = BindGroupLayoutDescriptor::new(
        "hdr_overlay_inputs",
        &BindGroupLayoutEntries::sequential(
            ShaderStages::FRAGMENT,
            (
                texture_2d(unfiltered),
                texture_2d(TextureSampleType::Float { filterable: true }),
                texture_2d(unfiltered),
                texture_2d(unfiltered),
                sampler(SamplerBindingType::Filtering),
            ),
        ),
    );
    let linear_clamp = device.create_sampler(&SamplerDescriptor {
        mag_filter: FilterMode::Linear,
        min_filter: FilterMode::Linear,
        ..default()
    });
    let inputs = device.create_bind_group(
        "hdr_overlay_inputs",
        &cache.get_bind_group_layout(&layout),
        &BindGroupEntries::sequential((&scene, &effect, &full_depth, &half_depth, &linear_clamp)),
    );
    let output = texture(
        &device,
        "hdr_overlay_composite",
        FRAME,
        FORMAT,
        TextureUsages::RENDER_ATTACHMENT | TextureUsages::TEXTURE_BINDING,
    )
    .create_view(&TextureViewDescriptor::default());
    let pipeline = cache.queue_render_pipeline(RenderPipelineDescriptor {
        label: Some("hdr_overlay_composite".into()),
        layout: vec![layout],
        vertex: fullscreen.to_vertex_state(),
        fragment: Some(FragmentState {
            shader: SHADER,
            entry_point: Some("composite_overlay".into()),
            targets: vec![Some(ColorTargetState {
                format: FORMAT,
                blend: None,
                write_mask: ColorWrites::ALL,
            })],
            ..default()
        }),
        ..default()
    });
    commands.insert_resource(OverlayReplay {
        pipeline,
        inputs,
        output,
    });
}

impl OverlayReplay {
    pub(super) fn draw<'a>(
        &'a self,
        encoder: &mut CommandEncoder,
        cache: &PipelineCache,
        size: Extent3d,
    ) -> Option<&'a TextureView> {
        if size != FRAME {
            return None;
        }
        let pipeline = cache.get_render_pipeline(self.pipeline)?;
        let mut pass = encoder.begin_render_pass(&RenderPassDescriptor {
            label: Some("hdr_overlay_composite"),
            color_attachments: &[Some(RenderPassColorAttachment {
                view: &self.output,
                depth_slice: None,
                resolve_target: None,
                ops: Operations {
                    load: LoadOp::Clear(Default::default()),
                    store: StoreOp::Store,
                },
            })],
            ..default()
        });
        pass.set_pipeline(pipeline);
        pass.set_bind_group(0, &self.inputs, &[]);
        pass.draw(0..3, 0..1);
        Some(&self.output)
    }
}

fn texture(
    device: &RenderDevice,
    label: &'static str,
    size: Extent3d,
    format: TextureFormat,
    usage: TextureUsages,
) -> Texture {
    device.create_texture(&TextureDescriptor {
        label: Some(label),
        size,
        format,
        usage,
        dimension: TextureDimension::D2,
        mip_level_count: 1,
        sample_count: 1,
        view_formats: &[],
    })
}
