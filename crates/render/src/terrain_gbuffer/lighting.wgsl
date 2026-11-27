#import bevy_pbr::{
    mesh_view_bindings::view,
    mesh_types::MESH_FLAGS_SHADOW_RECEIVER_BIT,
    pbr_functions::main_pass_post_lighting_processing,
    pbr_types::{PbrInput, pbr_input_new, STANDARD_MATERIAL_FLAGS_FOG_ENABLED_BIT},
    view_transformations::{position_ndc_to_view, position_ndc_to_world},
}
#import botw::clouds::{CloudParams, cloud_shadow}
#import botw::look::{read_look_at, sky_cover, field_wetness_at, haze_terms}
#import botw::deferred_light::{
    FieldSurface, Frame, MainLight, game_frame, scene_depth, ssao_on_grid,
    main_light_source, field_shadow, wet_field, sky_visibility, field_ambient_at, field_compose,
}

@group(2) @binding(0) var albedo: texture_2d<f32>;
@group(2) @binding(1) var normals: texture_2d<f32>;
@group(2) @binding(2) var look_texture: texture_2d<f32>;
@group(2) @binding(3) var ssao_noise: texture_2d<f32>;
@group(2) @binding(4) var cube_means: texture_2d<f32>;
@group(2) @binding(5) var cloud_shadow_map: texture_2d<f32>;
@group(2) @binding(6) var cloud_shadow_sampler: sampler;
@group(2) @binding(7) var<uniform> clouds: CloudParams;
@group(2) @binding(8) var half_shadow: texture_2d<f32>;
@group(2) @binding(9) var half_fog: texture_2d<f32>;
@group(2) @binding(10) var half_diffuse: texture_2d<f32>;
@group(2) @binding(11) var half_normal_guide: texture_2d<f32>;
@group(2) @binding(12) var half_depth_guide: texture_2d<f32>;

struct FieldPixel {
    pbr: PbrInput,
    frame: Frame,
    surface: FieldSurface,
    normal_guide: f32,
}

fn field_pixel(uv: vec2<f32>) -> FieldPixel {
    let size = vec2<f32>(textureDimensions(normals));
    let texel = clamp(vec2<i32>(uv * size), vec2(0), vec2<i32>(size) - vec2(1));
    let ndc = vec3(uv.x * 2.0 - 1.0, 1.0 - 2.0 * uv.y, max(scene_depth(uv), 1e-6));
    let normal = textureLoad(normals, texel, 0);
    let material = textureLoad(albedo, texel, 0);
    let bits = u32(round(normal.a * 255.0));
    var surface: FieldSurface;
    surface.albedo = material.rgb;
    surface.N = normalize(normal.rgb * 2.0 - 1.0);
    surface.gloss = f32(bits >> 2u) / 63.75;
    surface.metal = f32(u32(round(material.a * 255.0)) >> 2u) / 63.0;
    surface.flag = f32(bits & 1u);

    var pbr = pbr_input_new();
    pbr.frag_coord = vec4(uv * size, ndc.z, 1.0);
    pbr.world_position = vec4(position_ndc_to_world(ndc), 1.0);
    pbr.V = normalize(view.world_position - pbr.world_position.xyz);
    // SI-GBUF-03: the G-buffer carries the mapped normal, which also biases Bevy's shadow query here.
    pbr.world_normal = surface.N;
    pbr.N = surface.N;
    pbr.flags = MESH_FLAGS_SHADOW_RECEIVER_BIT;
    pbr.material.flags |= STANDARD_MATERIAL_FLAGS_FOG_ENABLED_BIT;
    let guide = dot(normal.rgb, vec3(1.0 / 3.0));
    return FieldPixel(pbr, game_frame(position_ndc_to_view(ndc), uv), surface, guide);
}

struct PreShading {
    @location(0) shadow: vec4<f32>,
    @location(1) fog: vec4<f32>,
    @location(2) diffuse: vec4<f32>,
    @location(4) normal_guide: f32,
    @location(5) depth_guide: f32,
}

struct FieldVertexOutput {
    @invariant @builtin(position) position: vec4<f32>,
    @location(0) uv: vec2<f32>,
}

@vertex
fn field_vertex(@builtin(vertex_index) vertex: u32, @builtin(instance_index) code: u32) -> FieldVertexOutput {
    let uv = vec2<f32>(f32(vertex >> 1u), f32(vertex & 1u)) * 2.0;
    let xy = uv * vec2(2.0, -2.0) + vec2(-1.0, 1.0);
    return FieldVertexOutput(vec4(xy, f32(code) / 255.0, 1.0), uv);
}

fn shade_field(pixel: FieldPixel, light: MainLight, half_size: vec2<f32>) -> PreShading {
    let pbr = pixel.pbr;
    let point = pbr.world_position.xyz;
    let z = -pixel.frame.p.z;
    let look = read_look_at(look_texture, point);
    let cover = sky_cover(look_texture, look, point);
    let wetness = field_wetness_at(look_texture, look, point, cover, pixel.surface.N.y, pixel.surface.flag);
    let surface = wet_field(pixel.surface, -pbr.V, vec2(wetness, look.weather[0].y));
    let open = sky_visibility(cover, point.y, surface.N.y, surface.flag, z);
    var ao = 1.0;
    if z < 150.0 {
        ao = ssao_on_grid(pixel.frame, ssao_noise, pbr.frag_coord.xy, half_size);
    }
    let shade = 1.0 - cloud_shadow(cloud_shadow_map, cloud_shadow_sampler, clouds, point);
    let sun = field_shadow(light, pbr, pixel.frame.p.z, shade, surface.translucent);
    let ambient = field_ambient_at(surface, z, ao, open, cube_means);
    let haze = haze_terms(look, point, -pbr.V);
    return PreShading(vec4(sun, wetness, haze.transmission, open), vec4(haze.scattering, 0.0),
        vec4(ambient, 0.0), pixel.normal_guide, z);
}

@fragment
fn preshade(in: FieldVertexOutput) -> PreShading {
    let half_size = vec2<f32>(max(textureDimensions(normals) / 2u, vec2(1u)));
    let uv = in.position.xy / half_size;
    return shade_field(field_pixel(uv), main_light_source(), half_size);
}

fn reconstruction_weights(pixel: FieldPixel, center: vec2<f32>, taps: array<vec2<i32>, 4>) -> vec4<f32> {
    let z = -pixel.frame.p.z;
    var depth: vec4<f32>;
    var normal: vec4<f32>;
    for (var i = 0u; i < 4u; i += 1u) {
        depth[i] = textureLoad(half_depth_guide, taps[i], 0).r;
        normal[i] = textureLoad(half_normal_guide, taps[i], 0).r;
    }
    let valid = depth > vec4(0.0);
    let normal_cost = 4.0 * abs(normal - pixel.normal_guide);
    let depth_error = abs(depth - z);

    if all(valid) {
        // SI-GBUF-04: the Wii U shader compares opposite corners in the guide footprint.
        let depth_scale = 2.0 * abs(view.clip_from_view[1][1]) / max(z, 1e-6);
        let cost = fma(depth_error, vec4(depth_scale), normal_cost);
        let top_left_minus_bottom_right = cost.x - cost.w;
        let bottom_left_minus_top_right = cost.z - cost.y;
        let shift = vec2(top_left_minus_bottom_right + bottom_left_minus_top_right,
                         top_left_minus_bottom_right - bottom_left_minus_top_right);
        let f = clamp(fract(center) + shift, vec2(0.0), vec2(1.0));
        return vec4((1.0 - f.x) * (1.0 - f.y), f.x * (1.0 - f.y), (1.0 - f.x) * f.y, f.x * f.y);
    }

    // Host pre-shading is sparse outside terrain. Keep the nearest matching valid tap there.
    let cost = depth_error * (2.0 / max(z, 0.3)) + normal_cost;
    var weights = vec4(0.0);
    var score = 1e10;
    var distance_squared = 1e10;
    for (var i = 0u; i < 4u; i += 1u) {
        if !valid[i] {
            continue;
        }
        let offset = vec2<f32>(taps[i]) - center;
        let candidate = dot(offset, offset);
        if cost[i] < score || (cost[i] == score && candidate < distance_squared) {
            score = cost[i];
            distance_squared = candidate;
            weights = vec4(0.0);
            weights[i] = 1.0;
        }
    }
    return weights;
}

@fragment
fn compose(in: FieldVertexOutput) -> @location(0) vec4<f32> {
    let pixel = field_pixel(in.uv);
    let light = main_light_source();
    let size = vec2<i32>(textureDimensions(half_depth_guide));
    let center = pixel.frame.uv * vec2<f32>(size) - 0.5;
    let base = vec2<i32>(floor(center));
    let taps = array(
        clamp(base, vec2(0), size - 1),
        clamp(base + vec2(1, 0), vec2(0), size - 1),
        clamp(base + vec2(0, 1), vec2(0), size - 1),
        clamp(base + vec2(1, 1), vec2(0), size - 1),
    );
    let weights = reconstruction_weights(pixel, center, taps);
    var shadow = vec4(0.0);
    var fog = vec3(0.0);
    var diffuse = vec3(0.0);
    if any(weights > vec4(0.0)) {
        for (var i = 0u; i < 4u; i += 1u) {
            let at = taps[i];
            shadow += weights[i] * textureLoad(half_shadow, at, 0);
            fog += weights[i] * textureLoad(half_fog, at, 0).rgb;
            diffuse += weights[i] * textureLoad(half_diffuse, at, 0).rgb;
        }
    } else {
        let full = shade_field(pixel, light, vec2(0.0));
        shadow = full.shadow;
        fog = full.fog.rgb;
        diffuse = full.diffuse.rgb;
    }
    let look = read_look_at(look_texture, pixel.pbr.world_position.xyz);
    let surface = wet_field(pixel.surface, -pixel.pbr.V, vec2(shadow.y, look.weather[0].y));
    let lit = field_compose(pixel.pbr, surface, pixel.frame, light, shadow.x, diffuse, shadow.w);
    return main_pass_post_lighting_processing(pixel.pbr, vec4(lit * shadow.z + fog, 1.0));
}
