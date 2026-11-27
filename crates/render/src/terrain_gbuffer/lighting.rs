//! Half-resolution field visibility and lighting, then full-resolution HDR composition.
use std::collections::HashMap;

use bevy::asset::{load_internal_asset, uuid_handle};
use bevy::camera::CameraUpdateSystems;
use bevy::core_pipeline::core_3d::{Opaque3d, main_opaque_pass_3d};
use bevy::core_pipeline::prepass::ViewPrepassTextures;
use bevy::core_pipeline::schedule::{Core3d, Core3dSystems};
use bevy::ecs::system::{SystemParamItem, lifetimeless::SRes};
use bevy::pbr::deferred::DeferredLightingLayout;
use bevy::pbr::{DrawMaterial, MeshPipelineKey, MeshViewBindGroup, ViewKeyCache};
use bevy::prelude::*;
use bevy::render::extract_resource::{ExtractResource, ExtractResourcePlugin};
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
use crate::deferred_light::{CubeMean, SolarIrradiation, SsaoNoise};
use crate::look::LookTexture;
use crate::object_material::ObjectMaterial;

use super::{
    CHARACTER_CLASS, CLASS_DEPTH, FIELD_CLASS, FIELD_OBJECT_CLASS, LEAF_CLASS, Stages, Targets,
    TerrainGBuffer, class_group, classify, color, target,
};
use super::ambient::{LightAnalysis, LightAnalyzerPipelines, LightAnalyzerPlugin, allocate_reduction};
use super::geometry::{TerrainMeshes, draw_geometry};

const SHADER: Handle<Shader> = uuid_handle!("e9b6a2f4-b244-46a5-bf90-36303dcd704a");
const EXPAND: TextureFormat = TextureFormat::Rgba8Unorm;
const SHADOW: TextureFormat = TextureFormat::Rgba8Unorm;
const INDIRECT: TextureFormat = TextureFormat::Rg11b10Ufloat;
const FIELD_GUIDES: TextureFormat = TextureFormat::Rg32Float;

pub(super) struct TerrainLightingPlugin;

impl Plugin for TerrainLightingPlugin {
    fn build(&self, app: &mut App) {
        load_internal_asset!(app, SHADER, "lighting.wgsl", Shader::from_wgsl);
        app.add_plugins(LightAnalyzerPlugin);
        app.add_plugins((
            ExtractResourcePlugin::<CloudShadows>::default(),
            ExtractResourcePlugin::<SsaoNoise>::default(),
            ExtractResourcePlugin::<FieldPreshading>::default(),
        ));
        app.init_resource::<FieldPreshading>().add_systems(
            PostUpdate,
            resize_preshading.after(CameraUpdateSystems),
        );
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

/// The main view's indirect light and its depth/normal guides, also
/// sampled by the forward field materials.
#[derive(Resource, Default, Clone, ExtractResource)]
pub(crate) struct FieldPreshading {
    pub diffuse: Option<Handle<Image>>,
    pub guides: Option<Handle<Image>>,
    size: UVec2,
}

fn resize_preshading(
    views: Query<(&Camera, &TerrainGBuffer)>,
    mut shared: ResMut<FieldPreshading>,
    mut images: ResMut<Assets<Image>>,
    mut materials: ResMut<Assets<ObjectMaterial>>,
) {
    let Ok((camera, TerrainGBuffer::Lighting)) = views.single() else {
        return;
    };
    let Some(frame) = camera.physical_target_size() else {
        return;
    };
    let size = (frame / 2).max(UVec2::ONE);
    if shared.size == size {
        return;
    }
    shared.size = size;
    shared.diffuse = Some(images.add(Image::new_target_texture(size.x, size.y, INDIRECT, None)));
    shared.guides = Some(images.add(Image::new_target_texture(size.x, size.y, FIELD_GUIDES, None)));
    for (_, material) in materials.iter_mut() {
        material.extension.field_diffuse = shared.diffuse.clone();
        material.extension.field_guides = shared.guides.clone();
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
    expand: CachedTexture,
    reduction: Vec<CachedTexture>,
    shadow: CachedTexture,
    fog: CachedTexture,
}

pub(super) fn allocate_targets(
    size: Extent3d,
    mut get: impl FnMut(&'static str, TextureFormat, Extent3d) -> CachedTexture,
) -> LightingTargets {
    LightingTargets {
        expand: get("field_half_expand", EXPAND, size),
        reduction: allocate_reduction(size, &mut get),
        shadow: get("field_half_shadow", SHADOW, size),
        fog: get("field_half_fog", INDIRECT, size),
    }
}

#[derive(Resource)]
struct LightingLayouts {
    expand: BindGroupLayoutDescriptor,
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
            ),
        )
        .iter()
        .cloned(),
    );
    let mut pre = base.to_vec();
    pre.extend(
        BindGroupLayoutEntries::with_indices(ShaderStages::FRAGMENT, ((13, float()), (14, float())))
            .iter()
            .cloned(),
    );
    let mut expand = base.to_vec();
    expand.extend(
        BindGroupLayoutEntries::with_indices(
            ShaderStages::FRAGMENT,
            (
                (14, float()),
                (15, texture_2d(TextureSampleType::Float { filterable: true })),
                (16, sampler(SamplerBindingType::Filtering)),
            ),
        )
        .iter()
        .cloned(),
    );
    commands.insert_resource(LightingLayouts {
        expand: BindGroupLayoutDescriptor::new("field_expand", &expand),
        pre: BindGroupLayoutDescriptor::new("field_preshading", &pre),
        compose: BindGroupLayoutDescriptor::new("field_compose", &compose),
    });
}

fn queue_pipelines(
    cache: &PipelineCache,
    pbr: &DeferredLightingLayout,
    layouts: &LightingLayouts,
    key: MeshPipelineKey,
) -> [CachedRenderPipelineId; 3] {
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
        queue("expand", &layouts.expand, vec![target(EXPAND)]),
        queue(
            "preshade",
            &layouts.pre,
            vec![
                target(SHADOW),
                target(INDIRECT),
                target(INDIRECT),
                None,
                target(FIELD_GUIDES),
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
    pipelines: HashMap<MeshPipelineKey, [CachedRenderPipelineId; 3]>,
    clouds: UniformBuffer<CloudParams>,
}

pub(super) struct LightingPasses {
    half: RenderPipeline,
    full: RenderPipeline,
    expand: RenderPipeline,
    pre: RenderPipeline,
    compose: RenderPipeline,
    classes: BindGroup,
    scene_classes: BindGroup,
    expand_inputs: BindGroup,
    analysis: LightAnalysis,
    pre_inputs: BindGroup,
    compose_inputs: BindGroup,
    diffuse: TextureView,
    guides: TextureView,
}

fn prepare_lighting(
    view: ViewQuery<(&ExtractedView, &ViewPrepassTextures, &MeshViewBindGroup, &mut Targets)>,
    stages: Res<Stages>,
    pbr: Res<DeferredLightingLayout>,
    keys: Res<ViewKeyCache>,
    layouts: Res<LightingLayouts>,
    analyzer: Res<LightAnalyzerPipelines>,
    cache: Res<PipelineCache>,
    device: Res<RenderDevice>,
    queue: Res<RenderQueue>,
    images: Res<RenderAssets<GpuImage>>,
    shared: Res<FieldPreshading>,
    scene: (
        Option<Res<LookTexture>>,
        Option<Res<SsaoNoise>>,
        Option<Res<CubeMean>>,
        Option<Res<CloudShadows>>,
        Option<Res<SolarIrradiation>>,
    ),
    mut state: Local<LightingState>,
) {
    let (view, prepass, _, mut out) = view.into_inner();
    if !out.geometry_ready {
        return;
    }
    let (Some(out_lit), Some(&key)) = (&out.lighting, keys.get(&view.retained_view_entity)) else {
        return;
    };
    let Some(scene_normals) = prepass.normal_view() else {
        return;
    };
    let (Some(diffuse), Some(guides)) = (
        shared.diffuse.as_ref().and_then(|handle| images.get(handle)),
        shared.guides.as_ref().and_then(|handle| images.get(handle)),
    ) else {
        return;
    };
    if diffuse.texture.size() != out.half.texture.size() || guides.texture.size() != diffuse.texture.size() {
        return;
    }
    let (Some(look), Some(noise), Some(cube), Some(clouds), Some(solar)) = scene else {
        return;
    };
    let (Some(look), Some(noise), Some(cube), Some(cloud_map), Some(solar)) = (
        images.get(&look.0),
        images.get(&noise.0),
        images.get(&cube.0),
        images.get(&clouds.shadow_map),
        images.get(&solar.0),
    ) else {
        return;
    };
    let Some(analysis) = analyzer.prepare(
        &cache,
        &device,
        &out_lit.expand.default_view,
        &out_lit.reduction,
        &look.texture_view,
        &cube.texture_view,
    ) else {
        return;
    };
    let [expand, pre, compose] = *state
        .pipelines
        .entry(key)
        .or_insert_with(|| queue_pipelines(&cache, &pbr, &layouts, key));
    let [Some(half), Some(expand), Some(pre), Some(full), Some(compose)] =
        [stages.scene_half.pipeline, expand, pre, stages.full.pipeline, compose]
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
    let mut expand_inputs = base.to_vec();
    expand_inputs.extend(
        BindGroupEntries::with_indices((
            (14, scene_normals),
            (15, &solar.texture_view),
            (16, &solar.sampler),
        ))
        .iter()
        .cloned(),
    );
    let expand_inputs = device.create_bind_group(
        "field_expand_inputs",
        &cache.get_bind_group_layout(&layouts.expand),
        &expand_inputs,
    );
    let mut pre_inputs = base.to_vec();
    pre_inputs.extend(
        BindGroupEntries::with_indices(((13, &out_lit.expand.default_view), (14, scene_normals)))
            .iter()
            .cloned(),
    );
    let pre_inputs = device.create_bind_group(
        "field_preshade_inputs",
        &cache.get_bind_group_layout(&layouts.pre),
        &pre_inputs,
    );
    let mut compose_inputs = base.to_vec();
    compose_inputs.extend(
        BindGroupEntries::with_indices((
            (8, &out_lit.shadow.default_view),
            (9, &out_lit.fog.default_view),
            (10, &diffuse.texture_view),
            (11, &guides.texture_view),
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
    let scene_classes = device.create_bind_group(
        "field_scene_class",
        &cache.get_bind_group_layout(stages.scene_half.layout.as_ref().unwrap()),
        &BindGroupEntries::with_indices(((2, &out.class.default_view), (7, scene_normals))),
    );
    out.lighting_passes = Some(LightingPasses {
        half: half.clone(),
        full: full.clone(),
        expand: expand.clone(),
        pre: pre.clone(),
        compose: compose.clone(),
        classes,
        scene_classes,
        expand_inputs,
        analysis,
        pre_inputs,
        compose_inputs,
        diffuse: diffuse.texture_view.clone(),
        guides: guides.texture_view.clone(),
    });
}

fn draw_lighting(
    view: ViewQuery<(&ViewTarget, &MeshViewBindGroup, &Targets)>,
    mut render: RenderContext,
    mut logged: Local<bool>,
    shared: Res<FieldPreshading>,
    images: Res<RenderAssets<GpuImage>>,
) {
    let (view_target, bindings, out) = view.into_inner();
    let (Some(out_lit), Some(passes)) = (&out.lighting, &out.lighting_passes) else {
        // Forward materials must never reconstruct from a previous frame.
        if out.lighting.is_some()
            && let Some(guides) = shared.guides.as_ref().and_then(|handle| images.get(handle))
        {
            render.begin_tracked_render_pass(RenderPassDescriptor {
                label: Some("clear field pre-shading guides"),
                color_attachments: &[color(&guides.texture_view)],
                ..default()
            });
        }
        return;
    };
    let LightingPasses {
        half,
        full,
        expand,
        pre,
        compose,
        classes,
        scene_classes,
        expand_inputs,
        analysis,
        pre_inputs,
        compose_inputs,
        diffuse,
        guides,
    } = passes;
    classify(&mut render, "field scene class half", half, scene_classes, &out.half);
    draw(
        &mut render,
        "field half shadow and SSAO (Expand)",
        expand,
        bindings,
        expand_inputs,
        &out.half,
        &[0, FIELD_CLASS, FIELD_OBJECT_CLASS, LEAF_CLASS, CHARACTER_CLASS],
        &[color(&out_lit.expand.default_view)],
    );
    analysis.draw(&mut render, &out_lit.reduction);
    draw(
        &mut render,
        "field half pre-shading",
        pre,
        bindings,
        pre_inputs,
        &out.half,
        &[FIELD_CLASS, FIELD_OBJECT_CLASS],
        &[
            color(&out_lit.shadow.default_view),
            color(&out_lit.fog.default_view),
            color(diffuse),
            None,
            color(guides),
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
        &[FIELD_CLASS],
        &[Some(view_target.get_unsampled_color_attachment())],
    );
    if !*logged {
        info!("Terrain deferred chain active: Expand, LightAnalyzer, pre-shading, HDR composition");
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
    classes: &[u8],
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
    for &class in classes {
        let class = u32::from(class);
        pass.draw(0..3, class..class + 1);
    }
}
