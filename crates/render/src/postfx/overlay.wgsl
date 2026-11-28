// Wii U frame 3830, E13778. See docs/research/wiiu-paired-tarrey-3830.md.
#import bevy_core_pipeline::fullscreen_vertex_shader::FullscreenVertexOutput

@group(0) @binding(0) var scene_hdr: texture_2d<f32>;
@group(0) @binding(1) var half_effect: texture_2d<f32>;
@group(0) @binding(2) var full_depth: texture_2d<f32>;
@group(0) @binding(3) var half_depth: texture_2d<f32>;
@group(0) @binding(4) var linear_clamp: sampler;
@group(0) @binding(5) var<uniform> edge_adjust: vec4<f32>;

fn depth_difference(coord: vec2<i32>, last: vec2<i32>, reference: f32) -> f32 {
    return abs(textureLoad(half_depth, clamp(coord, vec2<i32>(0), last), 0).r - reference);
}

@fragment
fn composite_overlay(in: FullscreenVertexOutput) -> @location(0) vec4<f32> {
    let pixel = vec2<i32>(in.position.xy);
    let uv = in.position.xy / vec2<f32>(textureDimensions(full_depth));
    let half_size = vec2<f32>(textureDimensions(half_depth));
    let half_position = uv * half_size - 0.5;
    let gather_origin = vec2<i32>(floor((uv + 0.0001) * half_size - 0.5));
    let last = vec2<i32>(half_size) - 1;
    let z = textureLoad(full_depth, pixel, 0).r;
    let tl = depth_difference(gather_origin, last, z);
    let tr = depth_difference(gather_origin + vec2<i32>(1, 0), last, z);
    let bl = depth_difference(gather_origin + vec2<i32>(0, 1), last, z);
    let br = depth_difference(gather_origin + vec2<i32>(1, 1), last, z);
    let shift = vec2<f32>((tl - br) + (bl - tr), (tl - br) - (bl - tr));
    let fraction = fract(half_position);
    let guided = saturate(fraction + shift * (edge_adjust.y / (z + edge_adjust.x)));
    let effect = textureSampleLevel(half_effect, linear_clamp, uv + (guided - fraction) / half_size, 0.0);
    return vec4<f32>(effect.rgb + textureLoad(scene_hdr, pixel, 0).rgb * effect.a, 1.0);
}
