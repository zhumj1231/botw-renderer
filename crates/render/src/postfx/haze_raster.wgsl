// Frame 3830 uses a BC4 particle mask followed by a mesh that reads the mask.
// The replay vertices are already in Vulkan clip space.

struct MaskVertex {
    @location(0) clip: vec4<f32>,
    @location(1) color0: vec4<f32>,
    @location(2) color1: vec4<f32>,
    @location(3) fade: vec4<f32>,
    @location(4) tint: vec4<f32>,
    @location(5) uv: vec4<f32>,
}

struct MaskVaryings {
    @builtin(position) position: vec4<f32>,
    @location(0) color0: vec4<f32>,
    @location(1) color1: vec4<f32>,
    @location(2) fade: vec4<f32>,
    @location(3) tint: vec4<f32>,
    @location(4) uv: vec4<f32>,
}

struct HazeVertex {
    @location(0) clip: vec4<f32>,
    @location(1) color: vec4<f32>,
    @location(2) projected: vec4<f32>,
    @location(3) fade: vec4<f32>,
    @location(4) tint: vec4<f32>,
}

struct HazeVaryings {
    @builtin(position) position: vec4<f32>,
    @location(0) color: vec4<f32>,
    @location(1) projected: vec4<f32>,
    @location(2) fade: vec4<f32>,
    @location(3) tint: vec4<f32>,
}

struct Params {
    palette: vec4<f32>,
    alpha_ref: f32,
}

@group(0) @binding(0) var mask_input: texture_2d<f32>;
@group(0) @binding(1) var linear_sampler: sampler;
@group(0) @binding(2) var<uniform> params: Params;

@vertex
fn mask_vertex(v: MaskVertex) -> MaskVaryings {
    return MaskVaryings(v.clip, v.color0, v.color1, v.fade, v.tint, v.uv);
}

@fragment
fn haze_mask(v: MaskVaryings) -> @location(0) vec4<f32> {
    let mask = textureSample(mask_input, linear_sampler, v.uv.xy).r;
    let alpha = v.fade.x * saturate(v.color0.a * (v.tint.a * mask));
    if !(alpha > params.alpha_ref) {
        discard;
    }
    let rgb = (v.color1.rgb + (v.color0.rgb - v.color1.rgb) * (mask * mask)) * v.tint.rgb;
    return vec4<f32>(rgb, alpha);
}

@vertex
fn haze_vertex(v: HazeVertex) -> HazeVaryings {
    return HazeVaryings(v.clip, v.color, v.projected, v.fade, v.tint);
}

@fragment
fn haze_effect(v: HazeVaryings) -> @location(0) vec4<f32> {
    let mask = textureSample(mask_input, linear_sampler, v.projected.xy * (1.0 / v.projected.w)).r;
    let alpha = (v.fade.x * saturate(v.color.a * v.tint.a)) * saturate(mask * 0.5);
    if !(alpha > params.alpha_ref) {
        discard;
    }
    let rgb = (v.color.rgb * v.tint.a) * (params.palette.rgb * params.palette.a);
    return vec4<f32>(rgb, alpha);
}
