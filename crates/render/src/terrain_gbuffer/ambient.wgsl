#import bevy_core_pipeline::fullscreen_vertex_shader::FullscreenVertexOutput
#import botw::look::read_look

@group(0) @binding(0) var visibility: texture_2d<f32>;
@group(0) @binding(1) var linear_sampler: sampler;
@group(0) @binding(2) var look_texture: texture_2d<f32>;

#ifdef ANALYZE_WIDTH_PERCENT
@fragment
fn reduce_first(in: FullscreenVertexOutput) -> @location(0) vec4<f32> {
    let width_scale = f32(#{ANALYZE_WIDTH_PERCENT}) / 100.0;
    let uv = vec2(0.5 + (in.uv.x - 0.5) * width_scale, in.uv.y);
    return textureSampleLevel(visibility, linear_sampler, uv, 0.0);
}
#endif

@fragment
fn reduce(in: FullscreenVertexOutput) -> @location(0) vec4<f32> {
    return textureSampleLevel(visibility, linear_sampler, in.uv, 0.0);
}

@fragment
fn resolve() -> @location(0) vec4<f32> {
    let sample = textureSampleLevel(visibility, linear_sampler, vec2(0.5), 0.0);
    // Alpha weights the pixels supplied by the terrain and background passes.
    let share = select(1.0, sample.r / max(sample.a, 1e-6), sample.a > 0.0);
    let main_light = saturate(read_look(look_texture).light[0].y);
    return vec4(0.0, saturate(share) * main_light, 0.0, 0.0);
}
