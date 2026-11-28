//! Live geometry path for the camera's two haze emitter programs.

use bevy::core_pipeline::FullscreenShader;
use bevy::ecs::system::SystemParam;
use bevy::mesh::VertexBufferLayout;
use bevy::prelude::*;
use bevy::render::mesh::{RenderMesh, RenderMeshBufferInfo, allocator::MeshAllocator};
use bevy::render::render_asset::RenderAssets;
use bevy::render::render_resource::binding_types::{
    sampler, storage_buffer_read_only_sized, texture_2d, uniform_buffer,
};
use bevy::render::render_resource::*;
use bevy::render::renderer::{RenderDevice, RenderQueue};
use bevy::render::storage::GpuShaderBuffer;
use bevy::render::texture::{CachedTexture, GpuImage, TextureCache};
use bevy::render::view::{ViewUniform, ViewUniformOffset, ViewUniforms};
use bevy::shader::ShaderDefVal;

use super::overlay::{self, LiveBases};
use crate::effects::{DrawParams, HazeDraws, HazeStage, PARTICLE_SHADER};

const MASK: TextureFormat = TextureFormat::R16Float;
const HDR: TextureFormat = TextureFormat::Rgba16Float;

#[derive(Resource)]
pub(super) struct LiveHaze {
    view: BindGroupLayoutDescriptor,
    emitter: BindGroupLayoutDescriptor,
    composite: BindGroupLayoutDescriptor,
    mask: CachedRenderPipelineId,
    effect: CachedRenderPipelineId,
    compose: CachedRenderPipelineId,
    linear: Sampler,
}

#[derive(SystemParam)]
pub(super) struct Inputs<'w> {
    draws: Option<Res<'w, HazeDraws>>,
    meshes: Res<'w, RenderAssets<RenderMesh>>,
    allocator: Res<'w, MeshAllocator>,
    buffers: Res<'w, RenderAssets<GpuShaderBuffer>>,
    views: Res<'w, ViewUniforms>,
}

pub(super) fn init(
    mut commands: Commands,
    device: Res<RenderDevice>,
    fullscreen: Res<FullscreenShader>,
    cache: Res<PipelineCache>,
) {
    if std::env::var_os("BOTW_HAZE_LIVE").is_none() {
        return;
    }
    let float = |filterable| texture_2d(TextureSampleType::Float { filterable });
    let view = BindGroupLayoutDescriptor::new(
        "live_haze_camera",
        &BindGroupLayoutEntries::single(ShaderStages::VERTEX, uniform_buffer::<ViewUniform>(true)),
    );
    let emitter = BindGroupLayoutDescriptor::new(
        "live_haze_emitter",
        &BindGroupLayoutEntries::with_indices(
            ShaderStages::VERTEX_FRAGMENT,
            (
                (0, storage_buffer_read_only_sized(false, None)),
                (1, storage_buffer_read_only_sized(false, None)),
                (2, storage_buffer_read_only_sized(false, None)),
                (3, uniform_buffer::<DrawParams>(false)),
                (4, float(true)),
                (5, sampler(SamplerBindingType::Filtering)),
                (10, float(false)),
                (12, float(true)),
                (13, sampler(SamplerBindingType::Filtering)),
                (14, uniform_buffer::<Vec4>(false)),
            ),
        ),
    );
    let composite = BindGroupLayoutDescriptor::new(
        "live_haze_composite",
        &BindGroupLayoutEntries::sequential(
            ShaderStages::FRAGMENT,
            (
                float(false),
                float(true),
                float(false),
                float(false),
                sampler(SamplerBindingType::Filtering),
                uniform_buffer::<Vec4>(false),
            ),
        ),
    );
    let linear = device.create_sampler(&SamplerDescriptor {
        mag_filter: FilterMode::Linear,
        min_filter: FilterMode::Linear,
        ..default()
    });
    let pipeline = |label: &'static str, format, cull_mode| {
        let attributes = [
            Mesh::ATTRIBUTE_POSITION,
            Mesh::ATTRIBUTE_NORMAL,
            Mesh::ATTRIBUTE_UV_0,
            Mesh::ATTRIBUTE_UV_1,
            Mesh::ATTRIBUTE_COLOR,
        ];
        // Mesh orders its attributes by their IDs, not insertion order.
        let mut attrs = attributes
            .into_iter()
            .zip([0, 1, 2, 4, 3])
            .collect::<Vec<_>>();
        attrs.sort_by_key(|(attribute, _)| attribute.id);
        let mut offset = 0;
        let attributes = attrs
            .into_iter()
            .map(|(attribute, shader_location)| {
                let entry = VertexAttribute {
                    format: attribute.format,
                    offset,
                    shader_location,
                };
                offset += attribute.format.size();
                entry
            })
            .collect();
        let defs = vec![
            ShaderDefVal::Bool("BOTW_LIVE_HAZE".into(), true),
            ShaderDefVal::UInt("MATERIAL_BIND_GROUP".into(), 1),
        ];
        cache.queue_render_pipeline(RenderPipelineDescriptor {
            label: Some(label.into()),
            layout: vec![view.clone(), emitter.clone()],
            vertex: VertexState {
                shader: PARTICLE_SHADER,
                shader_defs: defs.clone(),
                entry_point: Some("vertex".into()),
                buffers: vec![VertexBufferLayout {
                    array_stride: offset,
                    step_mode: VertexStepMode::Vertex,
                    attributes,
                }],
            },
            fragment: Some(FragmentState {
                shader: PARTICLE_SHADER,
                shader_defs: defs,
                entry_point: Some(label.into()),
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
            }),
            primitive: PrimitiveState {
                cull_mode,
                ..default()
            },
            ..default()
        })
    };
    let mask = pipeline("live_haze_mask", MASK, None);
    let effect = pipeline("live_haze_effect", HDR, Some(Face::Front));
    let compose = cache.queue_render_pipeline(RenderPipelineDescriptor {
        label: Some("live_haze_composite".into()),
        layout: vec![composite.clone()],
        vertex: fullscreen.to_vertex_state(),
        fragment: Some(FragmentState {
            shader: overlay::SHADER,
            entry_point: Some("composite_overlay".into()),
            targets: vec![Some(ColorTargetState {
                format: HDR,
                blend: None,
                write_mask: ColorWrites::ALL,
            })],
            ..default()
        }),
        ..default()
    });
    commands.insert_resource(LiveHaze {
        view,
        emitter,
        composite,
        mask,
        effect,
        compose,
        linear,
    });
}

impl LiveHaze {
    #[allow(clippy::too_many_arguments)]
    pub(super) fn draw(
        &self,
        encoder: &mut CommandEncoder,
        device: &RenderDevice,
        queue: &RenderQueue,
        cache: &PipelineCache,
        texture_cache: &mut TextureCache,
        images: &RenderAssets<GpuImage>,
        inputs: &Inputs,
        view_offset: &ViewUniformOffset,
        bases: &LiveBases,
        frame: Extent3d,
        palette: Vec4,
    ) -> Option<TextureView> {
        let draws = &inputs.draws.as_ref()?.0;
        if !draws.iter().any(|draw| draw.0 == HazeStage::Effect) {
            return None;
        }
        let (mask_pipeline, effect_pipeline, compose) = (
            cache.get_render_pipeline(self.mask)?,
            cache.get_render_pipeline(self.effect)?,
            cache.get_render_pipeline(self.compose)?,
        );
        let target = |textures: &mut TextureCache, label, size, format, extra| {
            textures.get(
                device,
                TextureDescriptor {
                    label: Some(label),
                    size,
                    dimension: TextureDimension::D2,
                    format,
                    mip_level_count: 1,
                    sample_count: 1,
                    usage: TextureUsages::RENDER_ATTACHMENT
                        | TextureUsages::TEXTURE_BINDING
                        | extra,
                    view_formats: &[],
                },
            )
        };
        let mask = target(
            texture_cache,
            "live_haze_mask",
            bases.mask.size(),
            MASK,
            TextureUsages::COPY_DST,
        );
        let effect = target(
            texture_cache,
            "live_haze_effect",
            Extent3d {
                width: (frame.width / 2).max(1),
                height: (frame.height / 2).max(1),
                depth_or_array_layers: 1,
            },
            HDR,
            TextureUsages::empty(),
        );
        let hdr = target(
            texture_cache,
            "live_haze_hdr",
            frame,
            HDR,
            TextureUsages::empty(),
        );
        let view_group = device.create_bind_group(
            "live_haze_camera",
            &cache.get_bind_group_layout(&self.view),
            &BindGroupEntries::single(inputs.views.uniforms.binding()?),
        );
        let uniform = |value| {
            let mut u = UniformBuffer::from(value);
            u.write_buffer(device, queue);
            u
        };
        let palette = uniform(palette);
        let edge = uniform(bases.edge_adjust);
        let emitter_layout = cache.get_bind_group_layout(&self.emitter);
        // The earlier scene-fog pass retains its input. Modify a separate copy.
        encoder.copy_texture_to_texture(
            bases.mask.as_image_copy(),
            mask.texture.as_image_copy(),
            bases.mask.size(),
        );
        let mut draw_pass = |stage, pipeline: &RenderPipeline, target: &CachedTexture, load| {
            let mut pass = encoder.begin_render_pass(&RenderPassDescriptor {
                label: Some(if stage == HazeStage::Mask {
                    "live_haze_mask"
                } else {
                    "live_haze_effect"
                }),
                color_attachments: &[Some(RenderPassColorAttachment {
                    view: &target.default_view,
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
            pass.set_bind_group(0, &view_group, &[view_offset.offset]);
            for (_, handle, material) in draws.iter().filter(|draw| draw.0 == stage) {
                let id = handle.id();
                let (Some(mesh), Some(vertices), Some(indices)) = (
                    inputs.meshes.get(id),
                    inputs.allocator.mesh_vertex_slice(&id),
                    inputs.allocator.mesh_index_slice(&id),
                ) else {
                    continue;
                };
                let RenderMeshBufferInfo::Indexed {
                    index_format,
                    count,
                } = mesh.buffer_info
                else {
                    continue;
                };
                let (Some(stat), Some(particles), Some(dynamic), Some(tex), Some(look)) = (
                    inputs.buffers.get(&material.statics),
                    inputs.buffers.get(&material.particles),
                    inputs.buffers.get(&material.dynamic),
                    images.get(&material.tex0),
                    images.get(&material.look),
                ) else {
                    continue;
                };
                let mut draw = UniformBuffer::from(material.draw);
                draw.write_buffer(device, queue);
                let source = if stage == HazeStage::Mask {
                    &tex.texture_view
                } else {
                    &mask.default_view
                };
                let group = device.create_bind_group(
                    "live_haze_emitter",
                    &emitter_layout,
                    &BindGroupEntries::with_indices((
                        (0, stat.buffer.as_entire_binding()),
                        (1, particles.buffer.as_entire_binding()),
                        (2, dynamic.buffer.as_entire_binding()),
                        (3, draw.binding().unwrap()),
                        (4, &tex.texture_view),
                        (5, &tex.sampler),
                        (10, &look.texture_view),
                        (12, source),
                        (13, &self.linear),
                        (14, palette.binding().unwrap()),
                    )),
                );
                pass.set_bind_group(1, &group, &[]);
                pass.set_vertex_buffer(0, *vertices.buffer.slice(..));
                pass.set_index_buffer(*indices.buffer.slice(..), index_format);
                pass.draw_indexed(
                    indices.range.start..indices.range.start + count,
                    vertices.range.start as i32,
                    0..1,
                );
            }
        };
        draw_pass(HazeStage::Mask, mask_pipeline, &mask, LoadOp::Load);
        draw_pass(
            HazeStage::Effect,
            effect_pipeline,
            &effect,
            LoadOp::Clear(LinearRgba::BLACK.into()),
        );
        let group = device.create_bind_group(
            "live_haze_composite",
            &cache.get_bind_group_layout(&self.composite),
            &BindGroupEntries::sequential((
                bases.scene,
                &effect.default_view,
                bases.full_depth,
                bases.half_depth,
                &self.linear,
                edge.binding()?,
            )),
        );
        let mut pass = encoder.begin_render_pass(&RenderPassDescriptor {
            label: Some("live_haze_composite"),
            color_attachments: &[Some(RenderPassColorAttachment {
                view: &hdr.default_view,
                depth_slice: None,
                resolve_target: None,
                ops: Operations {
                    load: LoadOp::Clear(LinearRgba::BLACK.into()),
                    store: StoreOp::Store,
                },
            })],
            ..default()
        });
        pass.set_pipeline(compose);
        pass.set_bind_group(0, &group, &[]);
        pass.draw(0..3, 0..1);
        drop(pass);
        Some(hdr.default_view)
    }
}
