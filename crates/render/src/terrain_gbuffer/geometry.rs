//! Draws visible terrain directly into the field G-buffer. The diagnostic
//! uses Bevy's mesh buffers, material bind groups and opaque scene depth.
use std::collections::HashMap;

use bevy::core_pipeline::core_3d::CORE_3D_DEPTH_FORMAT;
use bevy::ecs::system::SystemParam;
use bevy::mesh::{MeshVertexBufferLayoutRef, VertexBufferLayout};
use bevy::pbr::{MaterialBindGroupAllocators, PreparedMaterial, RenderMaterialInstances};
use bevy::prelude::*;
use bevy::render::Extract;
use bevy::render::erased_render_asset::ErasedRenderAssets;
use bevy::render::mesh::{
    RenderMesh, RenderMeshBufferInfo,
    allocator::{MeshAllocator, MeshBufferSlice},
};
use bevy::render::render_asset::RenderAssets;
use bevy::render::render_resource::{binding_types::uniform_buffer, *};
use bevy::render::renderer::{RenderContext, RenderDevice, RenderQueue, ViewQuery};
use bevy::render::sync_world::MainEntity;
use bevy::render::view::{
    ViewDepthTexture, ViewUniform, ViewUniformOffset, ViewUniforms,
    visibility::RenderVisibleEntities,
};
use bevy::shader::ShaderDefVal;

use crate::terrain::TerrainTile;
use crate::terrain_material;

use super::{
    ALBEDO_FORMAT, CLASS_FORMAT, FIELD_CLASS, NORMAL_FORMAT, Targets, TerrainGBuffer, color, target,
};

#[derive(Resource, Default)]
pub(super) struct TerrainMeshes(HashMap<MainEntity, (AssetId<Mesh>, Vec3)>);

impl TerrainMeshes {
    pub(super) fn contains(&self, entity: MainEntity) -> bool {
        self.0.contains_key(&entity)
    }
}

pub(super) fn extract_meshes(
    tiles: Extract<Query<(Entity, &Mesh3d, &GlobalTransform), With<TerrainTile>>>,
    mut meshes: ResMut<TerrainMeshes>,
) {
    meshes.0.clear();
    meshes
        .0
        .extend(tiles.iter().map(|(entity, mesh, transform)| {
            // Terrain quadrants only translate their baked positions.
            (entity.into(), (mesh.id(), transform.translation()))
        }));
}

#[derive(Resource)]
pub(super) struct FieldGeometry {
    view: BindGroupLayoutDescriptor,
}

pub(super) fn init_geometry(mut commands: Commands) {
    commands.insert_resource(FieldGeometry {
        view: BindGroupLayoutDescriptor::new(
            "field_gbuffer_view",
            &BindGroupLayoutEntries::single(
                ShaderStages::VERTEX_FRAGMENT,
                uniform_buffer::<ViewUniform>(true),
            ),
        ),
    });
}

impl SpecializedMeshPipeline for FieldGeometry {
    type Key = BindGroupLayoutDescriptor;

    fn specialize(
        &self,
        material: Self::Key,
        layout: &MeshVertexBufferLayoutRef,
    ) -> Result<RenderPipelineDescriptor, SpecializedMeshPipelineError> {
        let mesh = layout.0.get_layout(&[
            Mesh::ATTRIBUTE_POSITION.at_shader_location(0),
            Mesh::ATTRIBUTE_NORMAL.at_shader_location(1),
            Mesh::ATTRIBUTE_COLOR.at_shader_location(2),
        ])?;
        let instance = VertexBufferLayout {
            array_stride: size_of::<[f32; 3]>() as u64,
            step_mode: VertexStepMode::Instance,
            attributes: vec![VertexAttribute {
                format: VertexFormat::Float32x3,
                offset: 0,
                shader_location: 3,
            }],
        };
        let defs = vec![
            "BOTW_TERRAIN_GBUFFER".into(),
            "VERTEX_COLORS".into(),
            ShaderDefVal::UInt("MATERIAL_BIND_GROUP".into(), 1),
            ShaderDefVal::UInt("BOTW_FIELD_CLASS".into(), u32::from(FIELD_CLASS)),
        ];
        Ok(RenderPipelineDescriptor {
            label: Some("field_gbuffer_geometry".into()),
            layout: vec![self.view.clone(), material],
            vertex: VertexState {
                shader: terrain_material::SHADER,
                entry_point: Some("gbuffer_vertex".into()),
                shader_defs: defs.clone(),
                buffers: vec![mesh, instance],
                ..default()
            },
            fragment: Some(FragmentState {
                shader: terrain_material::SHADER,
                entry_point: Some("gbuffer_fragment".into()),
                shader_defs: defs,
                targets: vec![
                    target(CLASS_FORMAT),
                    target(ALBEDO_FORMAT),
                    None,
                    target(NORMAL_FORMAT),
                ],
                ..default()
            }),
            primitive: PrimitiveState {
                cull_mode: Some(Face::Back),
                ..default()
            },
            depth_stencil: Some(DepthStencilState {
                format: CORE_3D_DEPTH_FORMAT,
                depth_write_enabled: Some(false),
                depth_compare: Some(CompareFunction::GreaterEqual),
                stencil: default(),
                bias: default(),
            }),
            ..default()
        })
    }
}

#[derive(SystemParam)]
pub(super) struct SceneGeometry<'w> {
    terrain: Res<'w, TerrainMeshes>,
    meshes: Res<'w, RenderAssets<RenderMesh>>,
    instances: Res<'w, RenderMaterialInstances>,
    materials: Res<'w, ErasedRenderAssets<PreparedMaterial>>,
    material_groups: Res<'w, MaterialBindGroupAllocators>,
    allocator: Res<'w, MeshAllocator>,
}

struct TileDraw<'a> {
    pipeline: &'a RenderPipeline,
    material: &'a BindGroup,
    vertices: MeshBufferSlice<'a>,
    indices: Option<(MeshBufferSlice<'a>, IndexFormat, u32)>,
    vertex_count: u32,
    instance: u32,
}

pub(super) struct Batch {
    offsets: RawBufferVec<[f32; 3]>,
    pipelines: SpecializedMeshPipelines<FieldGeometry>,
    reported_geometry: bool,
}

impl Default for Batch {
    fn default() -> Self {
        Self {
            offsets: RawBufferVec::new(BufferUsages::VERTEX),
            pipelines: default(),
            reported_geometry: false,
        }
    }
}

pub(super) fn draw_geometry(
    view: ViewQuery<
        (
            &mut Targets,
            &ViewDepthTexture,
            &ViewUniformOffset,
            &RenderVisibleEntities,
        ),
        With<TerrainGBuffer>,
    >,
    views: Res<ViewUniforms>,
    field: Res<FieldGeometry>,
    geometry: SceneGeometry,
    cache: Res<PipelineCache>,
    device: Res<RenderDevice>,
    queue: Res<RenderQueue>,
    mut render: RenderContext,
    mut batch: Local<Batch>,
) {
    let (mut out, depth, offset, visible) = view.into_inner();
    let (Some(view_binding), Some(visible)) = (views.uniforms.binding(), visible.get::<Mesh3d>())
    else {
        return;
    };
    batch.offsets.clear();
    let mut draws = Vec::new();
    // A GPU-culled tile is outside the list this pass can draw.
    let mut visible_tiles = visible
        .entities_gpu_culling
        .keys()
        .filter(|&&entity| geometry.terrain.contains(entity))
        .count();
    for &(_, entity) in &visible.entities_cpu_culling {
        let Some(&(mesh_id, translation)) = geometry.terrain.0.get(&entity) else {
            continue;
        };
        visible_tiles += 1;
        let (Some(mesh), Some(vertices), Some(instance)) = (
            geometry.meshes.get(mesh_id),
            geometry.allocator.mesh_vertex_slice(&mesh_id),
            geometry.instances.instances.get(&entity),
        ) else {
            continue;
        };
        let Some(material) = geometry.materials.get(instance.asset_id) else {
            continue;
        };
        let (Some(layout), Some(material_group)) = (
            &material.properties.material_layout,
            geometry
                .material_groups
                .get(&instance.asset_id.type_id())
                .and_then(|allocator| allocator.get(material.binding.group))
                .and_then(|allocation| allocation.bind_group()),
        ) else {
            continue;
        };
        let Ok(pipeline) = batch
            .pipelines
            .specialize(&cache, &field, layout.clone(), &mesh.layout)
        else {
            continue;
        };
        let Some(pipeline) = cache.get_render_pipeline(pipeline) else {
            continue;
        };
        let indices = match mesh.buffer_info {
            RenderMeshBufferInfo::Indexed {
                index_format,
                count,
            } => {
                let Some(indices) = geometry.allocator.mesh_index_slice(&mesh_id) else {
                    continue;
                };
                Some((indices, index_format, count))
            }
            RenderMeshBufferInfo::NonIndexed => None,
        };
        draws.push(TileDraw {
            pipeline,
            material: material_group,
            vertices,
            indices,
            vertex_count: mesh.vertex_count,
            instance: batch.offsets.push(translation.to_array()) as u32,
        });
    }

    let bindings = device.create_bind_group(
        "field_gbuffer_view",
        &cache.get_bind_group_layout(&field.view),
        &BindGroupEntries::single(view_binding),
    );
    batch.offsets.write_buffer(&device, &queue);
    if !draws.is_empty() && !batch.reported_geometry {
        info!(
            "Field G-buffer geometry draws directly to RT0/1/3 ({} visible terrain quadrants)",
            draws.len()
        );
        batch.reported_geometry = true;
    }
    let mut pass = render.begin_tracked_render_pass(RenderPassDescriptor {
        label: Some("field gbuffer geometry"),
        color_attachments: &[
            color(&out.class.default_view),
            color(&out.albedo.default_view),
            None,
            color(&out.normal.default_view),
        ],
        depth_stencil_attachment: Some(RenderPassDepthStencilAttachment {
            // SI-GBUF-02: use the scene prepass depth for this field-only diagnostic.
            view: depth.view(),
            depth_ops: None,
            stencil_ops: None,
        }),
        ..default()
    });
    let Some(offsets) = batch.offsets.buffer() else {
        return;
    };
    let geometry_ready = !draws.is_empty() && draws.len() == visible_tiles;
    pass.set_vertex_buffer(1, offsets.slice(..));
    for draw in draws {
        pass.set_render_pipeline(draw.pipeline);
        pass.set_bind_group(0, &bindings, &[offset.offset]);
        pass.set_bind_group(1, draw.material, &[]);
        pass.set_vertex_buffer(0, draw.vertices.buffer.slice(..));
        let instance = draw.instance..draw.instance + 1;
        if let Some((indices, index_format, count)) = draw.indices {
            pass.set_index_buffer(indices.buffer.slice(..), index_format);
            pass.draw_indexed(
                indices.range.start..indices.range.start + count,
                draw.vertices.range.start as i32,
                instance,
            );
        } else {
            pass.draw(
                draw.vertices.range.start..draw.vertices.range.start + draw.vertex_count,
                instance,
            );
        }
    }
    drop(pass);
    out.geometry_ready = geometry_ready;
}
