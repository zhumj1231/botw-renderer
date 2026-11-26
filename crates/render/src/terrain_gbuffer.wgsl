// The terrain prepass uses Bevy's packed MRT as transport (SI-GBUF-01).
// The texture slots below agree with the unpacked field targets in frame 3830.
#import bevy_core_pipeline::fullscreen_vertex_shader::FullscreenVertexOutput
#import bevy_pbr::pbr_deferred_types::{unpack_unorm4x8_, unpack_24bit_normal}
#import bevy_pbr::utils::octahedral_decode

@group(0) @binding(0) var packed: texture_2d<u32>;
@group(0) @binding(1) var lighting_id: texture_2d<u32>;
@group(0) @binding(2) var classifications: texture_2d<f32>;
@group(0) @binding(3) var albedo: texture_2d<f32>;
@group(0) @binding(4) var normals: texture_2d<f32>;
@group(0) @binding(5) var half_classes: texture_depth_2d;
@group(0) @binding(6) var equal_result: texture_2d<f32>;

const TERRAIN_CLASS: u32 = 4u;

struct GBuffer {
    @location(0) classification: vec2<f32>,
    @location(1) albedo: vec4<f32>,
    @location(3) normal: vec4<f32>,
};

@fragment
fn unpack_terrain(in: FullscreenVertexOutput) -> GBuffer {
    let xy = vec2<i32>(in.position.xy);
    let id = textureLoad(lighting_id, xy, 0).r;
    if id != TERRAIN_CLASS {
        discard;
    }
    let material = textureLoad(packed, xy, 0);
    let base_roughness = unpack_unorm4x8_(material.r);
    let normal = octahedral_decode(unpack_24bit_normal(material.a));
    let gloss = floor((1.0 - base_roughness.a) * 63.75);
    // SI-GBUF-01: Bevy transports a quantized normal and gamma-encoded albedo.
    return GBuffer(
        vec2(f32(id) / 255.0, 0.0),
        vec4(pow(base_roughness.rgb, vec3(2.2)), 0.0),
        vec4(normal * 0.5 + vec3(0.5), (gloss * 4.0 + 1.0) / 255.0),
    );
}

fn coord(uv: vec2<f32>, size: vec2<u32>) -> vec2<i32> {
    return clamp(vec2<i32>(uv * vec2<f32>(size)), vec2(0), vec2<i32>(size) - vec2(1));
}

fn classification_depth(uv: vec2<f32>, mask: u32) -> f32 {
    let code = u32(round(textureLoad(classifications, coord(uv, textureDimensions(classifications)), 0).r * 255.0));
    return f32(code & mask) / 255.0;
}

@fragment
fn classify_half(in: FullscreenVertexOutput) -> @builtin(frag_depth) f32 {
    return classification_depth(in.uv, 0x0eu);
}

@fragment
fn classify_full(in: FullscreenVertexOutput) -> @builtin(frag_depth) f32 {
    return classification_depth(in.uv, 0x0fu);
}

@vertex
fn equal_vertex(@builtin(vertex_index) vertex: u32, @builtin(instance_index) code: u32) -> @builtin(position) vec4<f32> {
    let p = vec2(f32(vertex >> 1u), f32(vertex & 1u)) * 4.0 - vec2(1.0);
    return vec4(p, f32(code) / 255.0, 1.0);
}

@fragment
fn equal_terrain() -> @location(0) vec4<f32> {
    return vec4(0.09, 0.62, 0.32, 1.0);
}

@fragment
fn preview(in: FullscreenVertexOutput) -> @location(0) vec4<f32> {
    let uv = fract(in.uv * 2.0);
    let xy = coord(uv, textureDimensions(classifications));
    let code = u32(round(textureLoad(classifications, xy, 0).r * 255.0));
    let empty = vec4(0.014, 0.021, 0.032, 1.0);
    if in.uv.y < 0.5 {
        if code == 0u {
            return empty;
        }
        if in.uv.x < 0.5 {
            return vec4(textureLoad(albedo, xy, 0).rgb, 1.0);
        }
        return vec4(textureLoad(normals, xy, 0).rgb, 1.0);
    }
    if in.uv.x < 0.5 {
        let depth = textureLoad(half_classes, coord(uv, textureDimensions(half_classes)), 0);
        return select(empty, vec4(0.54, 0.22, 0.08, 1.0), depth > 0.0);
    }
    return max(empty, textureLoad(equal_result, xy, 0));
}
