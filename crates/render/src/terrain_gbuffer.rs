//! First scene-backed check of the field G-buffer and class-depth passes.
use bevy::asset::{load_internal_asset, uuid_handle};
use bevy::core_pipeline::FullscreenShader;
use bevy::core_pipeline::core_3d::main_transparent_pass_3d;
use bevy::core_pipeline::prepass::ViewPrepassTextures;
use bevy::core_pipeline::schedule::{Core3d, Core3dSystems};
use bevy::prelude::*;
use bevy::render::render_resource::binding_types::{texture_2d, texture_depth_2d};
use bevy::render::render_resource::*;
use bevy::render::renderer::{RenderContext, RenderDevice, ViewQuery};
use bevy::render::texture::{CachedTexture, TextureCache};
use bevy::render::view::ViewTarget;
use bevy::render::{Render, RenderApp, RenderStartup, RenderSystems};

const SHADER: Handle<Shader> = uuid_handle!("11c705a1-1393-44f5-a5d1-b2992d663941");
pub const FIELD_CLASS: u8 = 4;
const CLASS_DEPTH: TextureFormat = TextureFormat::Depth16Unorm;

#[derive(Resource)]
pub struct TerrainGBuffer;

pub struct TerrainGBufferPlugin;

impl Plugin for TerrainGBufferPlugin {
    fn build(&self, app: &mut App) {
        load_internal_asset!(app, SHADER, "terrain_gbuffer.wgsl", Shader::from_wgsl);
        app.insert_resource(TerrainGBuffer);
        let Some(render_app) = app.get_sub_app_mut(RenderApp) else {
            return;
        };
        render_app
            .add_systems(RenderStartup, init_pipelines)
            .add_systems(
                Render,
                prepare_targets.in_set(RenderSystems::PrepareResources),
            )
            .add_systems(
                Core3d,
                draw_diagnostics
                    .after(main_transparent_pass_3d)
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
    unpack: Stage,
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
    let unpack = BindGroupLayoutDescriptor::new(
        "terrain_gbuffer_source",
        &BindGroupLayoutEntries::sequential(
            fragment,
            (
                texture_2d(TextureSampleType::Uint),
                texture_2d(TextureSampleType::Uint),
            ),
        ),
    );
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
        unpack: queue(
            "unpack_terrain",
            Some(unpack),
            vec![
                target(TextureFormat::Rg8Unorm),
                target(TextureFormat::Rgba8UnormSrgb),
                None,
                target(TextureFormat::Rgba8Unorm),
            ],
            None,
            false,
        ),
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
    equal: CachedTexture,
}

fn prepare_targets(
    mut commands: Commands,
    mut cache: ResMut<TextureCache>,
    device: Res<RenderDevice>,
    views: Query<(Entity, &ViewPrepassTextures)>,
) {
    for (entity, prepass) in &views {
        if prepass.deferred.is_none() {
            continue;
        }
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
            class: get("terrain_gbuffer_rt0", TextureFormat::Rg8Unorm, full),
            albedo: get("terrain_gbuffer_rt1", TextureFormat::Rgba8UnormSrgb, full),
            normal: get("terrain_gbuffer_rt3", TextureFormat::Rgba8Unorm, full),
            half: get("terrain_class_half", CLASS_DEPTH, half),
            full: get("terrain_class_full", CLASS_DEPTH, full),
            equal: get("terrain_class_equal", TextureFormat::Rg11b10Ufloat, full),
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

fn draw_diagnostics(
    view: ViewQuery<(&ViewTarget, &ViewPrepassTextures, &Targets)>,
    stages: Res<Stages>,
    cache: Res<PipelineCache>,
    device: Res<RenderDevice>,
    mut render: RenderContext,
    mut logged: Local<bool>,
) {
    let (target, prepass, out) = view.into_inner();
    let (Some(packed), Some(ids)) = (prepass.deferred_view(), &prepass.deferred_lighting_pass_id)
    else {
        return;
    };
    let [
        Some(unpack),
        Some(half),
        Some(full),
        Some(equal),
        Some(preview),
    ] = [
        &stages.unpack,
        &stages.half,
        &stages.full,
        &stages.equal,
        &stages.preview,
    ]
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
    let unpack_group = bind(
        &stages.unpack,
        &BindGroupEntries::sequential((packed, &ids.texture.default_view)),
    );
    let classify_group = bind(
        &stages.half,
        &BindGroupEntries::with_indices(((2, &out.class.default_view),)),
    );
    let preview_group = bind(
        &stages.preview,
        &BindGroupEntries::with_indices((
            (2, &out.class.default_view),
            (3, &out.albedo.default_view),
            (4, &out.normal.default_view),
            (5, &out.half.default_view),
            (6, &out.equal.default_view),
        )),
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
    draw(
        "terrain G-buffer unpack",
        unpack,
        &[
            color(&out.class.default_view),
            color(&out.albedo.default_view),
            None,
            color(&out.normal.default_view),
        ],
        None,
        Some(&unpack_group),
        0..1,
    );
    draw(
        "terrain class half",
        half,
        &[],
        depth(&out.half.default_view, LoadOp::Clear(0.0)),
        Some(&classify_group),
        0..1,
    );
    draw(
        "terrain class full",
        full,
        &[],
        depth(&out.full.default_view, LoadOp::Clear(0.0)),
        Some(&classify_group),
        0..1,
    );
    let class = u32::from(FIELD_CLASS);
    draw(
        "terrain class Equal",
        equal,
        &[color(&out.equal.default_view)],
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
