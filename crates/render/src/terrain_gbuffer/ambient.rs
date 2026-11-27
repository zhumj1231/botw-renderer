//! View-wide visibility used by the field and its forward materials.
use bevy::asset::{load_internal_asset, uuid_handle};
use bevy::core_pipeline::FullscreenShader;
use bevy::prelude::*;
use bevy::render::render_resource::binding_types::{sampler, texture_2d};
use bevy::render::render_resource::*;
use bevy::render::renderer::{RenderContext, RenderDevice};
use bevy::render::texture::CachedTexture;
use bevy::render::{RenderApp, RenderStartup};
use bevy::shader::ShaderDefVal;

use crate::deferred_light::CUBE_MEAN_FORMAT;

use super::{color, target};

const SHADER: Handle<Shader> = uuid_handle!("160f36e1-497f-4159-ac43-8029d315ba6e");
const REDUCTION_FORMAT: TextureFormat = TextureFormat::Rgba16Float;
const ANALYZE_WIDTH_PERCENT: u32 = 80;
const REDUCTION_SCALE: f32 = 0.25;

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
    let width_scale = ANALYZE_WIDTH_PERCENT as f32 / 100.0;
    size.width = (size.width as f32 * width_scale * REDUCTION_SCALE) as u32;
    size.height = (size.height as f32 * REDUCTION_SCALE) as u32;
    let mut levels = Vec::new();
    loop {
        size.width = size.width.div_ceil(2).max(2);
        size.height = size.height.div_ceil(2).max(2);
        levels.push(get("light_analyzer_reduce", REDUCTION_FORMAT, size));
        if size.width == 2 && size.height == 2 {
            break;
        }
    }
    levels
}

#[derive(Resource)]
pub(super) struct LightAnalyzerPipelines {
    layout: BindGroupLayoutDescriptor,
    first: CachedRenderPipelineId,
    middle: CachedRenderPipelineId,
    last: CachedRenderPipelineId,
    only: CachedRenderPipelineId,
    resolve: CachedRenderPipelineId,
    sampler: Sampler,
}

fn init_pipelines(
    mut commands: Commands,
    device: Res<RenderDevice>,
    cache: Res<PipelineCache>,
    fullscreen: Res<FullscreenShader>,
) {
    let layout = BindGroupLayoutDescriptor::new(
        "light_analyzer",
        &BindGroupLayoutEntries::sequential(
            ShaderStages::FRAGMENT,
            (
                texture_2d(TextureSampleType::Float { filterable: true }),
                sampler(SamplerBindingType::Filtering),
                texture_2d(TextureSampleType::Float { filterable: false }),
            ),
        ),
    );
    let pipeline = |name: &'static str, entry: &'static str, color_target, shader_defs| {
        cache.queue_render_pipeline(RenderPipelineDescriptor {
            label: Some(format!("light_analyzer_{name}").into()),
            layout: vec![layout.clone()],
            vertex: fullscreen.to_vertex_state(),
            fragment: Some(FragmentState {
                shader: SHADER,
                shader_defs,
                entry_point: Some(entry.into()),
                targets: vec![color_target],
                ..default()
            }),
            ..default()
        })
    };
    let crop = ShaderDefVal::UInt("ANALYZE_WIDTH_PERCENT".into(), ANALYZE_WIDTH_PERCENT);
    let weather = ShaderDefVal::Bool("FINAL_REDUCTION".into(), true);
    let reduce = |name, defs| pipeline(name, "reduce", target(REDUCTION_FORMAT), defs);
    let first = reduce("reduce_first", vec![crop.clone()]);
    let middle = reduce("reduce", vec![]);
    let last = reduce("reduce_last", vec![weather.clone()]);
    let only = reduce("reduce_only", vec![crop, weather]);
    let resolve = pipeline(
        "resolve",
        "resolve",
        Some(ColorTargetState {
            format: CUBE_MEAN_FORMAT,
            blend: None,
            write_mask: ColorWrites::GREEN,
        }),
        vec![],
    );
    commands.insert_resource(LightAnalyzerPipelines {
        layout,
        first,
        middle,
        last,
        only,
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
    resolve: RenderPipeline,
    levels: Vec<(RenderPipeline, BindGroup)>,
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
        let resolve = cache.get_render_pipeline(self.resolve)?.clone();
        let layout = cache.get_bind_group_layout(&self.layout);
        let mut source = expand;
        let length = levels.len();
        let reductions = levels
            .iter()
            .enumerate()
            .map(|(index, level)| {
                let id = match (index == 0, index + 1 == length) {
                    (true, true) => self.only,
                    (true, false) => self.first,
                    (false, true) => self.last,
                    (false, false) => self.middle,
                };
                let pipeline = cache.get_render_pipeline(id)?.clone();
                let group = device.create_bind_group(
                    "light_analyzer_reduce",
                    &layout,
                    &BindGroupEntries::sequential((source, &self.sampler, look)),
                );
                source = &level.default_view;
                Some((pipeline, group))
            })
            .collect::<Option<Vec<_>>>()?;
        let resolve_inputs = device.create_bind_group(
            "light_analyzer_resolve",
            &layout,
            &BindGroupEntries::sequential((source, &self.sampler, look)),
        );
        Some(LightAnalysis {
            resolve,
            levels: reductions,
            resolve_inputs,
            means: means.clone(),
        })
    }
}

impl LightAnalysis {
    pub(super) fn draw(&self, render: &mut RenderContext, levels: &[CachedTexture]) {
        for (level, (pipeline, inputs)) in levels.iter().zip(&self.levels) {
            let mut pass = render.begin_tracked_render_pass(RenderPassDescriptor {
                label: Some("light analyzer reduce"),
                color_attachments: &[color(&level.default_view)],
                ..default()
            });
            pass.set_render_pipeline(pipeline);
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
