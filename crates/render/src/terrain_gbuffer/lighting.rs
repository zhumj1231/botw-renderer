//! Half-resolution field lighting and full-resolution HDR composition.
use std::collections::HashMap;

use bevy::asset::{load_internal_asset, uuid_handle};
use bevy::core_pipeline::core_3d::{Opaque3d, main_opaque_pass_3d};
use bevy::core_pipeline::schedule::{Core3d, Core3dSystems};
use bevy::ecs::system::{SystemParamItem, lifetimeless::SRes};
use bevy::pbr::deferred::DeferredLightingLayout;
use bevy::pbr::{DrawMaterial, MeshPipelineKey, MeshViewBindGroup, ViewKeyCache};
use bevy::prelude::*;
use bevy::render::extract_resource::ExtractResourcePlugin;
use bevy::render::render_asset::RenderAssets;
use bevy::render::render_phase::{
    DrawFunctions, PhaseItem, RenderCommand, RenderCommandResult, RenderCommandState,
    TrackedRenderPass,
};
use bevy::render::render_resource::binding_types::{sampler, texture_2d, uniform_buffer};
use bevy::render::render_resource::*;
use bevy::render::renderer::{RenderContext, RenderDevice, RenderQueue, ViewQuery};
use bevy::render::texture::{CachedTexture, GpuImage};
use bevy::render::view::{ExtractedView, ViewTarget};
use bevy::render::{RenderApp, RenderStartup};

use crate::clouds::{CloudParams, CloudShadows};
use crate::deferred_light::{CubeMean, SsaoNoise};
use crate::look::LookTexture;

use super::{CLASS_DEPTH, FIELD_CLASS, Stages, Targets, class_group, classify, color, target};
use super::geometry::{TerrainMeshes, draw_geometry};

const SHADER: Handle<Shader> = uuid_handle!("e9b6a2f4-b244-46a5-bf90-36303dcd704a");
const SHADOW: TextureFormat = TextureFormat::Rgba8Unorm;
const INDIRECT: TextureFormat = TextureFormat::Rg11b10Ufloat;
const NORMAL_GUIDE: TextureFormat = TextureFormat::R8Unorm;
const DEPTH_GUIDE: TextureFormat = TextureFormat::R32Float;

pub(super) struct TerrainLightingPlugin;

impl Plugin for TerrainLightingPlugin {
    fn build(&self, app: &mut App) {
        load_internal_asset!(app, SHADER, "lighting.wgsl", Shader::from_wgsl);
        app.add_plugins((
            ExtractResourcePlugin::<CloudShadows>::default(),
            ExtractResourcePlugin::<SsaoNoise>::default(),
        ));
        let Some(render_app) = app.get_sub_app_mut(RenderApp) else {
            return;
        };
        render_app
            .add_systems(RenderStartup, (init_layouts, register_opaque_draw))
            .add_systems(
                Core3d,
                (
                    prepare_lighting.after(draw_geometry).before(draw_lighting),
                    // Deformed grass skips the prepass and draws over the lit terrain.
                    draw_lighting.before(main_opaque_pass_3d),
                )
                    .in_set(Core3dSystems::MainPass),
            );
    }
}

fn register_opaque_draw(world: &mut World) {
    // Use the normal material draw for other batches and views. Prepass and
    // shadow phases have their own draw functions.
    let draw = RenderCommandState::<Opaque3d, (SkipDeferredTerrain, DrawMaterial)>::new(world);
    world
        .resource::<DrawFunctions<Opaque3d>>()
        .write()
        .add_with::<DrawMaterial, _>(draw);
}

struct SkipDeferredTerrain;

impl RenderCommand<Opaque3d> for SkipDeferredTerrain {
    type Param = (SRes<TerrainMeshes>, Local<'static, (bool, bool)>);
    type ViewQuery = Option<&'static Targets>;
    type ItemQuery = ();

    fn render<'w>(
        item: &Opaque3d,
        targets: Option<&'w Targets>,
        _: Option<()>,
        (terrain, mut reported): SystemParamItem<'w, '_, Self::Param>,
        _: &mut TrackedRenderPass<'w>,
    ) -> RenderCommandResult {
        let Some(targets) = targets.filter(|targets| targets.lighting.is_some()) else {
            return RenderCommandResult::Success;
        };
        if !terrain.contains(item.main_entity()) {
            return RenderCommandResult::Success;
        }
        if targets.lighting_passes.is_some() {
            if !reported.0 {
                debug!("Terrain forward opaque draw skipped: deferred geometry and lighting ready");
                reported.0 = true;
            }
            return RenderCommandResult::Skip;
        }
        if !reported.1 {
            debug!("Terrain forward opaque fallback selected: deferred geometry or lighting pending");
            reported.1 = true;
        }
        RenderCommandResult::Success
    }
}

pub(super) struct LightingTargets {
    shadow: CachedTexture,
    fog: CachedTexture,
    diffuse: CachedTexture,
    normal: CachedTexture,
    depth: CachedTexture,
}

pub(super) fn allocate_targets(
    size: Extent3d,
    mut get: impl FnMut(&'static str, TextureFormat, Extent3d) -> CachedTexture,
) -> LightingTargets {
    LightingTargets {
        shadow: get("field_half_shadow", SHADOW, size),
        fog: get("field_half_fog", INDIRECT, size),
        diffuse: get("field_half_diffuse", INDIRECT, size),
        normal: get("field_half_normal_guide", NORMAL_GUIDE, size),
        depth: get("field_half_depth_guide", DEPTH_GUIDE, size),
    }
}

#[derive(Resource)]
struct LightingLayouts {
    pre: BindGroupLayoutDescriptor,
    compose: BindGroupLayoutDescriptor,
}

fn init_layouts(mut commands: Commands) {
    let float = || texture_2d(TextureSampleType::Float { filterable: false });
    let base = BindGroupLayoutEntries::sequential(
        ShaderStages::FRAGMENT,
        (
            float(),
            float(),
            float(),
            float(),
            float(),
            texture_2d(TextureSampleType::Float { filterable: true }),
            sampler(SamplerBindingType::Filtering),
            uniform_buffer::<CloudParams>(false),
        ),
    );
    let mut compose = base.to_vec();
    compose.extend(
        BindGroupLayoutEntries::with_indices(
            ShaderStages::FRAGMENT,
            (
                (8, float()),
                (9, float()),
                (10, float()),
                (11, float()),
                (12, float()),
            ),
        )
        .iter()
        .cloned(),
    );
    commands.insert_resource(LightingLayouts {
        pre: BindGroupLayoutDescriptor::new("field_preshading", &base),
        compose: BindGroupLayoutDescriptor::new("field_compose", &compose),
    });
}

fn queue_pipelines(
    cache: &PipelineCache,
    pbr: &DeferredLightingLayout,
    layouts: &LightingLayouts,
    key: MeshPipelineKey,
) -> [CachedRenderPipelineId; 2] {
    // Reuse Bevy's view specialization for the camera, cube, shadows and clustered lights.
    let mut base = pbr.specialize(key);
    let defs = &mut base.fragment.as_mut().unwrap().shader_defs;
    defs.retain(|def| {
        !matches!(def, bevy::shader::ShaderDefVal::Bool(name, _)
        if name == "DEFERRED_PREPASS" || name == "DEFERRED_LIGHTING_PIPELINE")
    });
    let shader_defs = defs.clone();
    let queue = |name: &str, layout: &BindGroupLayoutDescriptor, targets| {
        cache.queue_render_pipeline(RenderPipelineDescriptor {
            label: Some(format!("field_{name}").into()),
            layout: vec![
                base.layout[0].clone(),
                base.layout[1].clone(),
                layout.clone(),
            ],
            vertex: VertexState {
                shader: SHADER,
                entry_point: Some("field_vertex".into()),
                ..default()
            },
            fragment: Some(FragmentState {
                shader: SHADER,
                entry_point: Some(name.to_owned().into()),
                shader_defs: shader_defs.clone(),
                targets,
                ..default()
            }),
            depth_stencil: Some(DepthStencilState {
                format: CLASS_DEPTH,
                depth_write_enabled: Some(false),
                depth_compare: Some(CompareFunction::Equal),
                stencil: default(),
                bias: default(),
            }),
            ..default()
        })
    };
    [
        queue(
            "preshade",
            &layouts.pre,
            vec![
                target(SHADOW),
                target(INDIRECT),
                target(INDIRECT),
                None,
                target(NORMAL_GUIDE),
                target(DEPTH_GUIDE),
            ],
        ),
        queue(
            "compose",
            &layouts.compose,
            vec![target(key.target_format())],
        ),
    ]
}

#[derive(Default)]
struct LightingState {
    pipelines: HashMap<MeshPipelineKey, [CachedRenderPipelineId; 2]>,
    clouds: UniformBuffer<CloudParams>,
}

pub(super) struct LightingPasses {
    half: RenderPipeline,
    full: RenderPipeline,
    pre: RenderPipeline,
    compose: RenderPipeline,
    classes: BindGroup,
    pre_inputs: BindGroup,
    compose_inputs: BindGroup,
}

fn prepare_lighting(
    view: ViewQuery<(&ViewTarget, &ExtractedView, &MeshViewBindGroup, &mut Targets)>,
    stages: Res<Stages>,
    pbr: Res<DeferredLightingLayout>,
    keys: Res<ViewKeyCache>,
    layouts: Res<LightingLayouts>,
    cache: Res<PipelineCache>,
    device: Res<RenderDevice>,
    queue: Res<RenderQueue>,
    images: Res<RenderAssets<GpuImage>>,
    scene: (
        Option<Res<LookTexture>>,
        Option<Res<SsaoNoise>>,
        Option<Res<CubeMean>>,
        Option<Res<CloudShadows>>,
    ),
    mut state: Local<LightingState>,
) {
    let (_, view, _, mut out) = view.into_inner();
    if !out.geometry_ready {
        return;
    }
    let (Some(out_lit), Some(&key)) = (&out.lighting, keys.get(&view.retained_view_entity)) else {
        return;
    };
    let (Some(look), Some(noise), Some(cube), Some(clouds)) = scene else {
        return;
    };
    let (Some(look), Some(noise), Some(cube), Some(cloud_map)) = (
        images.get(&look.0),
        images.get(&noise.0),
        images.get(&cube.0),
        images.get(&clouds.shadow_map),
    ) else {
        return;
    };
    let [pre, compose] = *state
        .pipelines
        .entry(key)
        .or_insert_with(|| queue_pipelines(&cache, &pbr, &layouts, key));
    let [Some(half), Some(pre), Some(full), Some(compose)] =
        [stages.half.pipeline, pre, stages.full.pipeline, compose]
            .map(|id| cache.get_render_pipeline(id))
    else {
        return;
    };
    state.clouds.set(clouds.params.clone());
    state.clouds.write_buffer(&device, &queue);
    let base = BindGroupEntries::sequential((
        &out.albedo.default_view,
        &out.normal.default_view,
        &look.texture_view,
        &noise.texture_view,
        &cube.texture_view,
        &cloud_map.texture_view,
        &cloud_map.sampler,
        state.clouds.binding().unwrap(),
    ));
    let pre_inputs = device.create_bind_group(
        "field_preshade_inputs",
        &cache.get_bind_group_layout(&layouts.pre),
        &base,
    );
    let mut compose_inputs = base.to_vec();
    compose_inputs.extend(
        BindGroupEntries::with_indices((
            (8, &out_lit.shadow.default_view),
            (9, &out_lit.fog.default_view),
            (10, &out_lit.diffuse.default_view),
            (11, &out_lit.normal.default_view),
            (12, &out_lit.depth.default_view),
        ))
        .iter()
        .cloned(),
    );
    let compose_inputs = device.create_bind_group(
        "field_compose_inputs",
        &cache.get_bind_group_layout(&layouts.compose),
        &compose_inputs,
    );

    let classes = class_group(&cache, &device, &stages, &out);
    out.lighting_passes = Some(LightingPasses {
        half: half.clone(),
        full: full.clone(),
        pre: pre.clone(),
        compose: compose.clone(),
        classes,
        pre_inputs,
        compose_inputs,
    });
}

fn draw_lighting(
    view: ViewQuery<(&ViewTarget, &MeshViewBindGroup, &Targets)>,
    mut render: RenderContext,
    mut logged: Local<bool>,
) {
    let (view_target, bindings, out) = view.into_inner();
    let (Some(out_lit), Some(passes)) = (&out.lighting, &out.lighting_passes) else {
        return;
    };
    let LightingPasses {
        half,
        full,
        pre,
        compose,
        classes,
        pre_inputs,
        compose_inputs,
    } = passes;
    classify(&mut render, "terrain class half", half, classes, &out.half);
    draw(
        &mut render,
        "field half pre-shading",
        pre,
        bindings,
        pre_inputs,
        &out.half,
        &[
            color(&out_lit.shadow.default_view),
            color(&out_lit.fog.default_view),
            color(&out_lit.diffuse.default_view),
            None,
            color(&out_lit.normal.default_view),
            color(&out_lit.depth.default_view),
        ],
    );
    classify(&mut render, "terrain class full", full, classes, &out.full);
    draw(
        &mut render,
        "field HDR composition",
        compose,
        bindings,
        compose_inputs,
        &out.full,
        &[Some(view_target.get_unsampled_color_attachment())],
    );
    if !*logged {
        info!("Terrain half pre-shading and class Equal HDR composition active");
        *logged = true;
    }
}

fn draw(
    render: &mut RenderContext,
    label: &str,
    pipeline: &RenderPipeline,
    view: &MeshViewBindGroup,
    inputs: &BindGroup,
    classification: &CachedTexture,
    colors: &[Option<RenderPassColorAttachment>],
) {
    let mut pass = render.begin_tracked_render_pass(RenderPassDescriptor {
        label: Some(label),
        color_attachments: colors,
        depth_stencil_attachment: Some(RenderPassDepthStencilAttachment {
            view: &classification.default_view,
            depth_ops: None,
            stencil_ops: None,
        }),
        ..default()
    });
    pass.set_render_pipeline(pipeline);
    pass.set_bind_group(0, &view.main, &view.main_offsets);
    pass.set_bind_group(1, &view.binding_array, &[]);
    pass.set_bind_group(2, inputs, &[]);
    let class = u32::from(FIELD_CLASS);
    pass.draw(0..3, class..class + 1);
}
