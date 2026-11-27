//! Field G-buffer attachments, class-depth passes and diagnostic display.
mod ambient;
mod geometry;
mod lighting;
use bevy::asset::{load_internal_asset, uuid_handle};
use bevy::core_pipeline::FullscreenShader;
use bevy::core_pipeline::core_3d::{main_opaque_pass_3d, main_transparent_pass_3d};
use bevy::core_pipeline::prepass::ViewPrepassTextures;
use bevy::core_pipeline::schedule::{Core3d, Core3dSystems};
use bevy::prelude::*;
use bevy::render::extract_component::{ExtractComponent, ExtractComponentPlugin};
use bevy::render::render_resource::binding_types::{texture_2d, texture_depth_2d};
use bevy::render::render_resource::*;
use bevy::render::renderer::{RenderContext, RenderDevice, ViewQuery};
use bevy::render::texture::{CachedTexture, TextureCache};
use bevy::render::view::ViewTarget;
use bevy::render::{ExtractSchedule, Render, RenderApp, RenderStartup, RenderSystems};

const SHADER: Handle<Shader> = uuid_handle!("11c705a1-1393-44f5-a5d1-b2992d663941");
pub const FIELD_CLASS: u8 = 4;
const CLASS_DEPTH: TextureFormat = TextureFormat::Depth16Unorm;
const CLASS_FORMAT: TextureFormat = TextureFormat::Rg8Unorm;
const ALBEDO_FORMAT: TextureFormat = TextureFormat::Rgba8UnormSrgb;
const NORMAL_FORMAT: TextureFormat = TextureFormat::Rgba8Unorm;

#[derive(Component, Clone, Copy, ExtractComponent)]
pub enum TerrainGBuffer {
    Preview,
    Lighting,
}

pub struct TerrainGBufferPlugin(pub TerrainGBuffer);

impl Plugin for TerrainGBufferPlugin {
    fn build(&self, app: &mut App) {
        load_internal_asset!(app, SHADER, "terrain_gbuffer.wgsl", Shader::from_wgsl);
        app.add_plugins(ExtractComponentPlugin::<TerrainGBuffer>::default());
        if matches!(self.0, TerrainGBuffer::Lighting) {
            app.add_plugins(lighting::TerrainLightingPlugin);
        }
        let Some(render_app) = app.get_sub_app_mut(RenderApp) else {
            return;
        };
        render_app
            .init_resource::<geometry::TerrainMeshes>()
            .add_systems(ExtractSchedule, geometry::extract_meshes)
            .add_systems(RenderStartup, (init_pipelines, geometry::init_geometry))
            .add_systems(
                Render,
                prepare_targets.in_set(RenderSystems::PrepareResources),
            )
            .add_systems(
                Core3d,
                (
                    geometry::draw_geometry
                        .after(Core3dSystems::Prepass)
                        .before(main_opaque_pass_3d),
                    draw_diagnostics.after(main_transparent_pass_3d),
                )
                    .in_set(Core3dSystems::MainPass),
            );
    }
}

struct Stage {
    layout: Option<BindGroupLayoutDescriptor>,
    pipeline: CachedRenderPipelineId,
}

#[derive(Resource)]
struct Stages {
    half: Stage,
    full: Stage,
    equal: Stage,
    preview: Stage,
}

fn target(format: TextureFormat) -> Option<ColorTargetState> {
    Some(ColorTargetState {
        format,
        blend: None,
        write_mask: ColorWrites::ALL,
    })
}

fn init_pipelines(
    mut commands: Commands,
    fullscreen: Res<FullscreenShader>,
    cache: Res<PipelineCache>,
) {
    let float = || texture_2d(TextureSampleType::Float { filterable: false });
    let fragment = ShaderStages::FRAGMENT;
    let classify = BindGroupLayoutDescriptor::new(
        "terrain_gbuffer_class",
        &BindGroupLayoutEntries::with_indices(fragment, ((2, float()),)),
    );
    let preview = BindGroupLayoutDescriptor::new(
        "terrain_gbuffer_preview",
        &BindGroupLayoutEntries::with_indices(
            fragment,
            (
                (2, float()),
                (3, float()),
                (4, float()),
                (5, texture_depth_2d()),
                (6, float()),
            ),
        ),
    );
    let queue = |name: &'static str,
                 layout: Option<BindGroupLayoutDescriptor>,
                 targets,
                 depth_stencil,
                 geometry| {
        let vertex = if geometry {
            VertexState {
                shader: SHADER,
                entry_point: Some("equal_vertex".into()),
                ..default()
            }
        } else {
            fullscreen.to_vertex_state()
        };
        let pipeline = cache.queue_render_pipeline(RenderPipelineDescriptor {
            label: Some(format!("terrain_gbuffer_{name}").into()),
            layout: layout.iter().cloned().collect(),
            vertex,
            fragment: Some(FragmentState {
                shader: SHADER,
                entry_point: Some(name.into()),
                targets,
                ..default()
            }),
            depth_stencil,
            ..default()
        });
        Stage { layout, pipeline }
    };
    let depth = |write, compare| {
        Some(DepthStencilState {
            format: CLASS_DEPTH,
            depth_write_enabled: Some(write),
            depth_compare: Some(compare),
            stencil: default(),
            bias: default(),
        })
    };
    commands.insert_resource(Stages {
        half: queue(
            "classify_half",
            Some(classify.clone()),
            vec![],
            depth(true, CompareFunction::Always),
            false,
        ),
        full: queue(
            "classify_full",
            Some(classify),
            vec![],
            depth(true, CompareFunction::Always),
            false,
        ),
        equal: queue(
            "equal_terrain",
            None,
            vec![target(TextureFormat::Rg11b10Ufloat)],
            depth(false, CompareFunction::Equal),
            true,
        ),
        preview: queue(
            "preview",
            Some(preview),
            vec![target(TextureFormat::Rgba16Float)],
            None,
            false,
        ),
    });
}

#[derive(Component)]
struct Targets {
    class: CachedTexture,
    albedo: CachedTexture,
    normal: CachedTexture,
    half: CachedTexture,
    full: CachedTexture,
    equal: Option<CachedTexture>,
    lighting: Option<lighting::LightingTargets>,
    geometry_ready: bool,
    lighting_passes: Option<lighting::LightingPasses>,
}

fn prepare_targets(
    mut commands: Commands,
    mut cache: ResMut<TextureCache>,
    device: Res<RenderDevice>,
    views: Query<(Entity, &ViewPrepassTextures, &TerrainGBuffer)>,
) {
    for (entity, prepass, mode) in &views {
        let full = prepass.size;
        let half = Extent3d {
            width: (full.width / 2).max(1),
            height: (full.height / 2).max(1),
            ..full
        };
        let mut get = |label, format, size| {
            cache.get(
                &device,
                TextureDescriptor {
                    label: Some(label),
                    size,
                    mip_level_count: 1,
                    sample_count: 1,
                    dimension: TextureDimension::D2,
                    format,
                    usage: TextureUsages::RENDER_ATTACHMENT | TextureUsages::TEXTURE_BINDING,
                    view_formats: &[],
                },
            )
        };
        commands.entity(entity).insert(Targets {
            class: get("terrain_gbuffer_rt0", CLASS_FORMAT, full),
            albedo: get("terrain_gbuffer_rt1", ALBEDO_FORMAT, full),
            normal: get("terrain_gbuffer_rt3", NORMAL_FORMAT, full),
            half: get("terrain_class_half", CLASS_DEPTH, half),
            full: get("terrain_class_full", CLASS_DEPTH, full),
            equal: matches!(mode, TerrainGBuffer::Preview)
                .then(|| get("terrain_class_equal", TextureFormat::Rg11b10Ufloat, full)),
            lighting: matches!(mode, TerrainGBuffer::Lighting)
                .then(|| lighting::allocate_targets(half, get)),
            geometry_ready: false,
            lighting_passes: None,
        });
    }
}

fn color(view: &TextureView) -> Option<RenderPassColorAttachment<'_>> {
    Some(RenderPassColorAttachment {
        view,
        depth_slice: None,
        resolve_target: None,
        ops: Operations {
            load: LoadOp::Clear(Default::default()),
            store: StoreOp::Store,
        },
    })
}

fn depth(view: &TextureView, load: LoadOp<f32>) -> Option<RenderPassDepthStencilAttachment<'_>> {
    Some(RenderPassDepthStencilAttachment {
        view,
        depth_ops: Some(Operations {
            load,
            store: StoreOp::Store,
        }),
        stencil_ops: None,
    })
}

fn class_group(
    cache: &PipelineCache,
    device: &RenderDevice,
    stages: &Stages,
    out: &Targets,
) -> BindGroup {
    device.create_bind_group(
        "field_class",
        &cache.get_bind_group_layout(stages.half.layout.as_ref().unwrap()),
        &BindGroupEntries::with_indices(((2, &out.class.default_view),)),
    )
}

fn classify(
    render: &mut RenderContext,
    label: &str,
    pipeline: &RenderPipeline,
    group: &BindGroup,
    target: &CachedTexture,
) {
    let mut pass = render.begin_tracked_render_pass(RenderPassDescriptor {
        label: Some(label),
        depth_stencil_attachment: depth(&target.default_view, LoadOp::Clear(0.0)),
        ..default()
    });
    pass.set_render_pipeline(pipeline);
    pass.set_bind_group(0, group, &[]);
    pass.draw(0..3, 0..1);
}

fn draw_diagnostics(
    view: ViewQuery<(&ViewTarget, &ViewPrepassTextures, &Targets), With<TerrainGBuffer>>,
    stages: Res<Stages>,
    cache: Res<PipelineCache>,
    device: Res<RenderDevice>,
    mut render: RenderContext,
    mut logged: Local<bool>,
) {
    let (target, prepass, out) = view.into_inner();
    let Some(equal_target) = &out.equal else {
        return;
    };
    let [Some(half), Some(full), Some(equal), Some(preview)] =
        [&stages.half, &stages.full, &stages.equal, &stages.preview]
            .map(|stage| cache.get_render_pipeline(stage.pipeline))
    else {
        return;
    };

    let bind = |stage: &Stage, entries: &[BindGroupEntry<'_>]| {
        device.create_bind_group(
            None,
            &cache.get_bind_group_layout(stage.layout.as_ref().unwrap()),
            entries,
        )
    };
    let classify_group = class_group(&cache, &device, &stages, out);
    let preview_group = bind(
        &stages.preview,
        &BindGroupEntries::with_indices((
            (2, &out.class.default_view),
            (3, &out.albedo.default_view),
            (4, &out.normal.default_view),
            (5, &out.half.default_view),
            (6, &equal_target.default_view),
        )),
    );

    classify(
        &mut render,
        "terrain class half",
        half,
        &classify_group,
        &out.half,
    );
    classify(
        &mut render,
        "terrain class full",
        full,
        &classify_group,
        &out.full,
    );

    let mut draw = |label,
                    pipeline: &RenderPipeline,
                    colors: &[Option<RenderPassColorAttachment>],
                    depth_stencil_attachment,
                    group: Option<&BindGroup>,
                    instances| {
        let mut pass = render
            .command_encoder()
            .begin_render_pass(&RenderPassDescriptor {
                label: Some(label),
                color_attachments: colors,
                depth_stencil_attachment,
                ..default()
            });
        pass.set_pipeline(pipeline);
        if let Some(group) = group {
            pass.set_bind_group(0, group, &[]);
        }
        pass.draw(0..3, instances);
    };
    let class = u32::from(FIELD_CLASS);
    draw(
        "terrain class Equal",
        equal,
        &[color(&equal_target.default_view)],
        depth(&out.full.default_view, LoadOp::Load),
        None,
        class..class + 1,
    );
    draw(
        "terrain G-buffer diagnostics",
        preview,
        &[Some(target.get_unsampled_color_attachment())],
        None,
        Some(&preview_group),
        0..1,
    );
    if !*logged {
        info!(
            "terrain G-buffer diagnostics: {}x{}, D16 class masks 0x0e/0x0f, Equal for class {FIELD_CLASS}",
            prepass.size.width, prepass.size.height
        );
        *logged = true;
    }
}
