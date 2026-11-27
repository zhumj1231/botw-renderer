#import bevy_core_pipeline::fullscreen_vertex_shader::FullscreenVertexOutput
#import botw::look::read_look

@group(0) @binding(0) var visibility: texture_2d<f32>;
@group(0) @binding(1) var linear_sampler: sampler;
@group(0) @binding(2) var look_texture: texture_2d<f32>;

@fragment
fn reduce(in: FullscreenVertexOutput) -> @location(0) vec4<f32> {
    return textureSampleLevel(visibility, linear_sampler, in.uv, 0.0);
}

@fragment
fn resolve() -> @location(0) vec4<f32> {
    let sample = textureSampleLevel(visibility, linear_sampler, vec2(0.5), 0.0);
    // Alpha weights the pixels supplied by the terrain and background passes.
    let share = select(1.0, sample.r / max(sample.a, 1e-6), sample.a > 0.0);
    let main_light = saturate(1.0 - read_look(look_texture).weather[0].w);
    return vec4(0.0, saturate(share) * main_light, 0.0, 0.0);
}
