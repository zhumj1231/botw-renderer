#import bevy_core_pipeline::fullscreen_vertex_shader::FullscreenVertexOutput
@group(0) @binding(2) var classifications: texture_2d<f32>;
@group(0) @binding(3) var albedo: texture_2d<f32>;
@group(0) @binding(4) var normals: texture_2d<f32>;
@group(0) @binding(5) var half_classes: texture_depth_2d;
@group(0) @binding(6) var equal_result: texture_2d<f32>;

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
fn equal_vertex(@builtin(vertex_index) vertex: u32, @builtin(instance_index) code: u32) -> @invariant @builtin(position) vec4<f32> {
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
