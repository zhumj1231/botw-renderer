//! View-wide visibility used by the field and its forward materials.
use bevy::asset::{load_internal_asset, uuid_handle};
use bevy::core_pipeline::FullscreenShader;
use bevy::prelude::*;
use bevy::render::render_resource::binding_types::{sampler, texture_2d};
use bevy::render::render_resource::*;
use bevy::render::renderer::{RenderContext, RenderDevice};
use bevy::render::texture::CachedTexture;
use bevy::render::{RenderApp, RenderStartup};

use crate::deferred_light::CUBE_MEAN_FORMAT;

use super::{color, target};

const SHADER: Handle<Shader> = uuid_handle!("160f36e1-497f-4159-ac43-8029d315ba6e");
const REDUCTION_FORMAT: TextureFormat = TextureFormat::Rgba16Float;

pub(super) struct LightAnalyzerPlugin;

impl Plugin for LightAnalyzerPlugin {
    fn build(&self, app: &mut App) {
        load_internal_asset!(app, SHADER, "ambient.wgsl", Shader::from_wgsl);
        if let Some(render_app) = app.get_sub_app_mut(RenderApp) {
            render_app.add_systems(RenderStartup, init_pipelines);
        }
    }
}

pub(super) fn allocate_reduction(
    mut size: Extent3d,
    get: &mut impl FnMut(&'static str, TextureFormat, Extent3d) -> CachedTexture,
) -> Vec<CachedTexture> {
    let mut levels = Vec::new();
    while size.width > 2 || size.height > 2 {
        size.width = size.width.div_ceil(2).max(2);
        size.height = size.height.div_ceil(2).max(2);
        levels.push(get("light_analyzer_reduce", REDUCTION_FORMAT, size));
    }
    levels
}

#[derive(Resource)]
pub(super) struct LightAnalyzerPipelines {
    reduce_layout: BindGroupLayoutDescriptor,
    resolve_layout: BindGroupLayoutDescriptor,
    reduce: CachedRenderPipelineId,
    resolve: CachedRenderPipelineId,
    sampler: Sampler,
}

fn init_pipelines(
    mut commands: Commands,
    device: Res<RenderDevice>,
    cache: Res<PipelineCache>,
    fullscreen: Res<FullscreenShader>,
) {
    let source = || texture_2d(TextureSampleType::Float { filterable: true });
    let filtering = || sampler(SamplerBindingType::Filtering);
    let reduce_layout = BindGroupLayoutDescriptor::new(
        "light_analyzer_reduce",
        &BindGroupLayoutEntries::sequential(ShaderStages::FRAGMENT, (source(), filtering())),
    );
    let resolve_layout = BindGroupLayoutDescriptor::new(
        "light_analyzer_resolve",
        &BindGroupLayoutEntries::sequential(
            ShaderStages::FRAGMENT,
            (
                source(),
                filtering(),
                texture_2d(TextureSampleType::Float { filterable: false }),
            ),
        ),
    );
    let pipeline = |name: &'static str, layout: &BindGroupLayoutDescriptor, color_target| {
        cache.queue_render_pipeline(RenderPipelineDescriptor {
            label: Some(format!("light_analyzer_{name}").into()),
            layout: vec![layout.clone()],
            vertex: fullscreen.to_vertex_state(),
            fragment: Some(FragmentState {
                shader: SHADER,
                entry_point: Some(name.into()),
                targets: vec![color_target],
                ..default()
            }),
            ..default()
        })
    };
    let reduce = pipeline("reduce", &reduce_layout, target(REDUCTION_FORMAT));
    let resolve = pipeline(
        "resolve",
        &resolve_layout,
        Some(ColorTargetState {
            format: CUBE_MEAN_FORMAT,
            blend: None,
            write_mask: ColorWrites::GREEN,
        }),
    );
    commands.insert_resource(LightAnalyzerPipelines {
        reduce_layout,
        resolve_layout,
        reduce,
        resolve,
        sampler: device.create_sampler(&SamplerDescriptor {
            label: Some("light_analyzer_linear"),
            mag_filter: FilterMode::Linear,
            min_filter: FilterMode::Linear,
            ..default()
        }),
    });
}

pub(super) struct LightAnalysis {
    reduce: RenderPipeline,
    resolve: RenderPipeline,
    levels: Vec<BindGroup>,
    resolve_inputs: BindGroup,
    means: TextureView,
}

impl LightAnalyzerPipelines {
    pub(super) fn prepare(
        &self,
        cache: &PipelineCache,
        device: &RenderDevice,
        expand: &TextureView,
        levels: &[CachedTexture],
        look: &TextureView,
        means: &TextureView,
    ) -> Option<LightAnalysis> {
        let reduce = cache.get_render_pipeline(self.reduce)?.clone();
        let resolve = cache.get_render_pipeline(self.resolve)?.clone();
        let layout = cache.get_bind_group_layout(&self.reduce_layout);
        let mut source = expand;
        let levels = levels
            .iter()
            .map(|level| {
                let group = device.create_bind_group(
                    "light_analyzer_reduce",
                    &layout,
                    &BindGroupEntries::sequential((source, &self.sampler)),
                );
                source = &level.default_view;
                group
            })
            .collect();
        let resolve_inputs = device.create_bind_group(
            "light_analyzer_resolve",
            &cache.get_bind_group_layout(&self.resolve_layout),
            &BindGroupEntries::sequential((source, &self.sampler, look)),
        );
        Some(LightAnalysis {
            reduce,
            resolve,
            levels,
            resolve_inputs,
            means: means.clone(),
        })
    }
}

impl LightAnalysis {
    pub(super) fn draw(&self, render: &mut RenderContext, levels: &[CachedTexture]) {
        for (level, inputs) in levels.iter().zip(&self.levels) {
            let mut pass = render.begin_tracked_render_pass(RenderPassDescriptor {
                label: Some("light analyzer reduce"),
                color_attachments: &[color(&level.default_view)],
                ..default()
            });
            pass.set_render_pipeline(&self.reduce);
            pass.set_bind_group(0, inputs, &[]);
            pass.draw(0..3, 0..1);
        }
        let mut pass = render.begin_tracked_render_pass(RenderPassDescriptor {
            label: Some("light analyzer resolve"),
            color_attachments: &[Some(RenderPassColorAttachment {
                view: &self.means,
                depth_slice: None,
                resolve_target: None,
                ops: Operations {
                    load: LoadOp::Load,
                    store: StoreOp::Store,
                },
            })],
            ..default()
        });
        pass.set_render_pipeline(&self.resolve);
        pass.set_bind_group(0, &self.resolve_inputs, &[]);
        pass.draw(0..3, 0..1);
    }
}
