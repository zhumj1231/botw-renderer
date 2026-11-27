#import bevy_pbr::{
    mesh_view_bindings::view,
    mesh_types::MESH_FLAGS_SHADOW_RECEIVER_BIT,
    pbr_functions::main_pass_post_lighting_processing,
    pbr_types::{PbrInput, pbr_input_new, STANDARD_MATERIAL_FLAGS_FOG_ENABLED_BIT},
    view_transformations::{direction_view_to_world, direction_world_to_view, position_ndc_to_view, position_ndc_to_world},
}
#import botw::clouds::{CloudParams, cloud_shadow}
#import botw::look::{read_look_at, sky_cover, field_wetness_at, haze_terms}
#import botw::deferred_light::{
    FieldSurface, Frame, MainLight, field_footprint, field_ssao, game_frame, scene_depth,
    main_light_source, field_surface_shadow, wet_field, sky_visibility, field_ambient_at, field_compose,
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
@group(2) @binding(11) var half_guides: texture_2d<f32>;
@group(2) @binding(13) var half_expand: texture_2d<f32>;
@group(2) @binding(14) var scene_normals: texture_2d<f32>;

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
    surface.N = normalize(direction_view_to_world(normal.rgb * 2.0 - 1.0));
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

fn object_pixel(uv: vec2<f32>) -> FieldPixel {
    let size = vec2<i32>(textureDimensions(scene_normals));
    let texel = clamp(vec2<i32>(uv * vec2<f32>(size)), vec2(0), size - vec2(1));
    let normal = normalize(textureLoad(scene_normals, texel, 0).rgb * 2.0 - 1.0);
    let ndc = vec3(uv.x * 2.0 - 1.0, 1.0 - 2.0 * uv.y, max(scene_depth(uv), 1e-6));
    var pbr = pbr_input_new();
    pbr.world_position = vec4(position_ndc_to_world(ndc), 1.0);
    pbr.V = normalize(view.world_position - pbr.world_position.xyz);
    pbr.world_normal = normal;
    pbr.flags = MESH_FLAGS_SHADOW_RECEIVER_BIT;
    var surface: FieldSurface;
    surface.N = normal;
    surface.flag = 1.0;
    let view_normal = direction_world_to_view(normal) * 0.5 + 0.5;
    return FieldPixel(pbr, game_frame(position_ndc_to_view(ndc), uv), surface, dot(view_normal, vec3(1.0 / 3.0)));
}

struct PreShading {
    @location(0) shadow: vec4<f32>,
    @location(1) fog: vec4<f32>,
    @location(2) diffuse: vec4<f32>,
    @location(4) guides: vec2<f32>,
}

struct FieldVertexOutput {
    @invariant @builtin(position) position: vec4<f32>,
    @location(0) uv: vec2<f32>,
    @location(1) @interpolate(flat) code: u32,
}

@vertex
fn field_vertex(@builtin(vertex_index) vertex: u32, @builtin(instance_index) code: u32) -> FieldVertexOutput {
    let uv = vec2<f32>(f32(vertex >> 1u), f32(vertex & 1u)) * 2.0;
    let xy = uv * vec2(2.0, -2.0) + vec2(-1.0, 1.0);
    return FieldVertexOutput(vec4(xy, f32(code) / 255.0, 1.0), uv, code);
}

fn field_visibility(pixel: FieldPixel, light: MainLight) -> vec4<f32> {
    let pbr = pixel.pbr;
    let ao = field_ssao(pixel.frame, ssao_noise, light.index);
    let sun = field_surface_shadow(light, pbr, pixel.frame.p.z);
    // Expand.x is sun visibility, .z is SSAO, and .w is neutral for terrain.
    return vec4(sun, 0.0, ao, 1.0);
}

fn shade_field(pixel: FieldPixel, visibility: vec4<f32>) -> PreShading {
    let pbr = pixel.pbr;
    let point = pbr.world_position.xyz;
    let z = -pixel.frame.p.z;
    let look = read_look_at(look_texture, point);
    let cover = sky_cover(look_texture, look, point);
    let surface = pixel.surface;
    // The rainy pre-shading variant writes wetness to Shadow.y. Only
    // composition changes the normal and gloss.
    let wetness = field_wetness_at(look_texture, look, point, cover, surface.N.y, surface.flag);
    let open = sky_visibility(cover, point.y, surface.N.y, surface.flag, z);
    let shade = 1.0 - cloud_shadow(cloud_shadow_map, cloud_shadow_sampler, clouds, point);
    let sun = shade * visibility.x;
    let ambient = field_ambient_at(surface, z, visibility.z, open, cube_means);
    let haze = haze_terms(look, point, -pbr.V);
    return PreShading(vec4(sun, wetness, haze.transmission, open), vec4(haze.scattering, 0.0),
        vec4(ambient, 0.0), vec2(z, round(clamp(pixel.normal_guide, 0.0, 1.0) * 255.0) / 255.0));
}

fn field_half_size() -> vec2<f32> {
    return vec2<f32>(max(textureDimensions(normals) / 2u, vec2(1u)));
}

@fragment
fn expand(in: FieldVertexOutput) -> @location(0) vec4<f32> {
    let half_size = field_half_size();
    let uv = in.position.xy / half_size;
    if in.code == 0u {
        // Opaque materials without a supported class leave no visibility sample.
        return vec4(select(0.0, 1.0, scene_depth(uv) == 0.0));
    }
    let light = main_light_source();
    if in.code == 4u {
        return field_visibility(field_pixel(uv), light);
    }
    if in.code == 5u {
        return field_visibility(object_pixel(uv), light);
    }

    let size = vec2<i32>(textureDimensions(scene_normals));
    let texel = clamp(vec2<i32>(uv * vec2<f32>(size)), vec2(0), size - vec2(1));
    let normal = normalize(textureLoad(scene_normals, texel, 0).rgb * 2.0 - 1.0);
    let center = (vec2<f32>(texel) + 0.5) / vec2<f32>(size);
    let ndc = vec3(center.x * 2.0 - 1.0, 1.0 - 2.0 * center.y, scene_depth(uv));
    var pbr = pbr_input_new();
    pbr.world_position = vec4(position_ndc_to_world(ndc), 1.0);
    // The character program applies its normal offset only on light-facing surfaces.
    let use_normal_bias = in.code == 6u || dot(normal, light.to_light) >= 0.35;
    pbr.world_normal = select(vec3(0.0), normal, use_normal_bias);
    pbr.flags = MESH_FLAGS_SHADOW_RECEIVER_BIT;
    let sun = field_surface_shadow(light, pbr, position_ndc_to_view(ndc).z);
    return vec4(sun, 0.0, 1.0, 1.0);
}

@fragment
fn preshade(in: FieldVertexOutput) -> PreShading {
    let xy = in.position.xy;
    let uv = xy / field_half_size();
    let visibility = textureLoad(half_expand, vec2<i32>(xy), 0);
    if in.code == 5u {
        return shade_field(object_pixel(uv), visibility);
    }
    return shade_field(field_pixel(uv), visibility);
}

@fragment
fn compose(in: FieldVertexOutput) -> @location(0) vec4<f32> {
    let pixel = field_pixel(in.uv);
    let light = main_light_source();
    let footprint = field_footprint(pixel.frame, pixel.normal_guide, half_guides);
    let weights = footprint.weights;
    var shadow = vec4(0.0);
    var fog = vec3(0.0);
    var diffuse = vec3(0.0);
    if any(weights > vec4(0.0)) {
        for (var i = 0u; i < 4u; i += 1u) {
            let at = footprint.taps[i];
            shadow += weights[i] * textureLoad(half_shadow, at, 0);
            fog += weights[i] * textureLoad(half_fog, at, 0).rgb;
            diffuse += weights[i] * textureLoad(half_diffuse, at, 0).rgb;
        }
    } else {
        let full = shade_field(pixel, field_visibility(pixel, light));
        shadow = full.shadow;
        fog = full.fog.rgb;
        diffuse = full.diffuse.rgb;
    }
    let look = read_look_at(look_texture, pixel.pbr.world_position.xyz);
    let surface = wet_field(pixel.surface, -pixel.pbr.V, vec2(shadow.y, look.weather[0].y));
    let lit = field_compose(pixel.pbr, surface, pixel.frame, light, shadow.x, diffuse, shadow.w, look.weather[0].w);
    return main_pass_post_lighting_processing(pixel.pbr, vec4(lit * shadow.z + fog, 1.0));
}
