// botw::deferred_light — the game's deferred shading, shared by the
// materials that do their own lighting (deferred_light.rs): what its main
// passes read of the frame (the environment cube map, `LightAnalyzer`'s
// ambient, the depth around a pixel, the main light) and the field's main
// passes (`field_hybrid`, `uking_sys_shading` program 32, with the terrain's
// pre-shading, program 112, and ambient occlusion, program 128; the
// foliage's `field_leaf`, programs 28 and 108;
// docs/research/wiiu-field-shading.md). The characters' own passes are in
// character_material.wgsl.
//
// Forward pass only: import it inside `#ifndef PREPASS_PIPELINE`.

#define_import_path botw::deferred_light

#import bevy_pbr::{
    mesh_view_bindings as view_bindings,
    mesh_view_bindings::{view, light_probes},
    mesh_view_types,
    mesh_types::MESH_FLAGS_SHADOW_RECEIVER_BIT,
    lighting,
    lighting::LAYER_BASE,
    clustered_forward as clustering,
    shadows,
    shadow_sampling::sample_shadow_map_hardware,
    pbr_types::PbrInput,
    view_transformations::{direction_world_to_view, position_ndc_to_view},
}
#import bevy_render::maths::PI

// The game's luminance (its shaders' weights).
fn luma(c: vec3<f32>) -> f32 {
    return dot(c, vec3<f32>(0.2989, 0.5866, 0.1144));
}

fn max3(c: vec3<f32>) -> f32 {
    return max(c.x, max(c.y, c.z));
}

fn min3(c: vec3<f32>) -> f32 {
    return min(c.x, min(c.y, c.z));
}

// --- The environment cube map (the game's `gsys_cube_map`) ---

// Mip of the environment map below its sharpest that stands for the game's
// LOD 3 (the size of the game's cube map is not known).
// SI-LGT-08: cube-map light mean, LOD and offsets are ours.
const CUBE_LOD_FROM_TOP: f32 = 4.0;

// The environment light's strength over the radiance it holds for surfaces
// lit the Bevy way (`daynight::SKY_LIGHT`, a fit for the land): the game
// reads its cube map at full strength.
// SI-LGT-08: cube-map light mean, LOD and offsets are ours.
const SKY_LIGHT: f32 = 0.5;

// The environment's radiance towards world direction `dir` from the view's
// environment map (the scene's cube map, `cubemap.rs`, or without the
// game's data the atmosphere's), pre-exposed, at the game's mip `lod` (3:
// the blurred light the game's ambient reads).
fn cube_at(dir: vec3<f32>, lod: f32) -> vec3<f32> {
#ifdef ENVIRONMENT_MAP
    if light_probes.view_cubemap_index < 0 {
        return vec3(0.0);
    }
    let q = light_probes.view_rotation;
    let t = 2.0 * cross(q.xyz, dir);
    var d = dir + q.w * t + cross(q.xyz, t);
    d.z = -d.z;
#ifdef MULTIPLE_LIGHT_PROBES_IN_ARRAY
    let levels = textureNumLevels(view_bindings::specular_environment_maps[light_probes.view_cubemap_index]);
    let level = max(f32(levels) - CUBE_LOD_FROM_TOP - 3.0 + lod, 0.0);
    let radiance = textureSampleLevel(view_bindings::specular_environment_maps[light_probes.view_cubemap_index], view_bindings::environment_map_sampler, d, level).rgb;
#else
    let level = max(f32(textureNumLevels(view_bindings::specular_environment_map)) - CUBE_LOD_FROM_TOP - 3.0 + lod, 0.0);
    let radiance = textureSampleLevel(view_bindings::specular_environment_map, view_bindings::environment_map_sampler, d, level).rgb;
#endif
    // The light's strength also undoes the exposure the scene's faces were
    // drawn with (`cubemap.rs`).
    return radiance * light_probes.intensity_for_view / SKY_LIGHT * view.exposure;
#else
    return vec3(0.0);
#endif
}

fn cube(dir: vec3<f32>) -> vec3<f32> {
    return cube_at(dir, 3.0);
}

// The mean brightness of the environment (`LightAnalyzer` folds its cube
// map into one texel from 14 directions: here the six axes and eight
// corners). Its 14 samples of `cube()` are taken once a frame
// (cube_mean.wgsl, `CubeMean` in deferred_light.rs) into `means`, before
// the light's strength and the exposure, which are applied here.
// SI-LGT-08: cube-map light mean, LOD and offsets are ours.
fn cube_mean(means: texture_2d<f32>) -> f32 {
#ifdef ENVIRONMENT_MAP
    if light_probes.view_cubemap_index < 0 {
        return 0.0;
    }
    let mean = textureLoad(means, vec2<i32>(0, 0), 0).r;
    return mean * light_probes.intensity_for_view / SKY_LIGHT * view.exposure;
#else
    return 0.0;
#endif
}

// --- LightAnalyzer (`gsys_user4`) ---

// The view's averaged visibility times the main light weight.
fn analyzed_light_share(means: texture_2d<f32>) -> f32 {
    return textureLoad(means, vec2<i32>(0), 0).g;
}

// The analyzer's field parameters (`ksysla` defaults, 0x038b37c0).
const FIELD_SAT: f32 = 0.5;
const FIELD_SAT_MIN: f32 = 0.8;
const FIELD_OFFSET_MIN: f32 = 0.1;
const FIELD_OFFSET_MAX: f32 = 0.5;
const FIELD_SCALE: f32 = 1.0;
const OFFSET_FAR: f32 = 1.0;

// The analyzer's texels for the field: its ambient's scale, saturation
// exponent and saturation scale (texel 1), and the sky's light straight up
// and down (texels 2 and 3).
struct FieldAmbient {
    scale: f32,
    sat: f32,
    sat_scale: f32,
    up: vec3<f32>,
    down: vec3<f32>,
}

fn field_ambient(means: texture_2d<f32>) -> FieldAmbient {
    let x = analyzed_light_share(means);
    let e = x * x * x;
    let mean = cube_mean(means);
    var a: FieldAmbient;
    // SI-LGT-08: cube-map light mean, LOD and offsets are ours.
    a.scale = FIELD_SCALE / (mean + mix(FIELD_OFFSET_MIN, FIELD_OFFSET_MAX, e));
    a.sat = FIELD_SAT;
    a.sat_scale = mix(FIELD_SAT_MIN, 1.0, e);
    a.up = cube(vec3<f32>(0.0, 1.0, 0.0)) / (mean + OFFSET_FAR);
    a.down = cube(vec3<f32>(0.0, -1.0, 0.0)) / (mean + OFFSET_FAR);
    return a;
}

// --- The depth around the pixel ---

// Screen offsets are in pixels of the game's 720-line frame.
const GAME_LINES: f32 = 720.0;

fn scene_depth(uv: vec2<f32>) -> f32 {
#ifdef DEPTH_PREPASS
    let size = vec2<f32>(textureDimensions(view_bindings::depth_prepass_texture));
    let texel = vec2<i32>(clamp(uv * size, vec2(0.0), size - 1.0));
    return textureLoad(view_bindings::depth_prepass_texture, texel, 0);
#else
    return 0.0;
#endif
}

// The view-space position of the scene at `uv`.
fn scene_position(uv: vec2<f32>) -> vec3<f32> {
    let ndc = vec3<f32>(uv.x * 2.0 - 1.0, 1.0 - 2.0 * uv.y, max(scene_depth(uv), 1e-6));
    return position_ndc_to_view(ndc);
}

// How far the scene at `uv` lies behind `p` (view space) along the view,
// against how far it lies aside: large at a silhouette over a far
// background, about zero on a continuous surface.
fn depth_gap(p: vec3<f32>, uv: vec2<f32>) -> f32 {
    let delta = scene_position(uv) - p;
    let v = normalize(p);
    let along = dot(v, delta);
    return along / max(length(delta - v * along), 1e-5);
}

// What the game's main passes need of the pixel and the frame.
struct Frame {
    // View-space position, the screen UV, one game pixel in UV.
    p: vec3<f32>,
    uv: vec2<f32>,
    pixel: vec2<f32>,
    // 1 / (z·tan(fovy/2)) and fovy.
    d: f32,
    fovy: f32,
}

fn game_frame(p: vec3<f32>, uv: vec2<f32>) -> Frame {
    var frame: Frame;
    frame.p = p;
    frame.uv = uv;
    let aspect = view.viewport.z / view.viewport.w;
    frame.pixel = vec2<f32>(1.0 / (GAME_LINES * aspect), 1.0 / GAME_LINES);
    let m11 = view.clip_from_view[1][1];
    frame.d = m11 / max(-p.z, 1e-3);
    frame.fovy = 2.0 * atan(1.0 / m11);
    return frame;
}

// The depth gap `offset` game pixels away on screen (y up).
fn edge(frame: Frame, offset: vec2<f32>) -> f32 {
    return depth_gap(frame.p, frame.uv + offset * vec2<f32>(1.0, -1.0) * frame.pixel);
}

// --- The main light ---

// The main light (sun or moon): towards it, its colour on a white surface
// in the frame's exposed units, its shadow at the point.
struct MainLight {
    to_light: vec3<f32>,
    color: vec3<f32>,
    index: u32,
    shadow: f32,
}

// SI-LGT-09: brightest Bevy light as the main light (game: one dir_main).
fn main_light_source() -> MainLight {
    var light: MainLight;
    light.to_light = vec3<f32>(0.0, 1.0, 0.0);
    light.color = vec3<f32>(0.0);
    light.shadow = 1.0;
    var brightest = -1.0;
    for (var i: u32 = 0u; i < view_bindings::lights.n_directional_lights; i = i + 1u) {
        let brightness = luma(view_bindings::lights.directional_lights[i].color.rgb);
        if brightness > brightest {
            brightest = brightness;
            light.index = i;
        }
    }
    if view_bindings::lights.n_directional_lights > 0u {
        let sun = &view_bindings::lights.directional_lights[light.index];
        light.to_light = (*sun).direction_to_light;
        light.color = (*sun).color.rgb / PI * view.exposure;
    }
    return light;
}

fn receives_main_shadow(light: MainLight, pbr: PbrInput) -> bool {
    let receives = (pbr.flags & MESH_FLAGS_SHADOW_RECEIVER_BIT) != 0u;
    if receives && view_bindings::lights.n_directional_lights > 0u {
        let flags = view_bindings::lights.directional_lights[light.index].flags;
        return (flags & mesh_view_types::DIRECTIONAL_LIGHT_FLAGS_SHADOWS_ENABLED_BIT) != 0u;
    }
    return false;
}

fn field_shadow(light: MainLight, pbr: PbrInput, view_z: f32, clouds: f32, translucent: f32) -> f32 {
    if (translucent <= 0.5 || LEAF_SHADOW_FROM_MAP) && receives_main_shadow(light, pbr) {
        return clouds * shadows::fetch_directional_shadow(light.index, pbr.world_position, pbr.world_normal, view_z, pbr.frag_coord.xy);
    }
    return clouds;
}

fn field_shadow_coord(light_index: u32, cascade_index: u32, pbr: PbrInput) -> vec4<f32> {
    let light = &view_bindings::lights.directional_lights[light_index];
    let texel_size = (*light).cascades[cascade_index].texel_size;
    let normal_offset = (*light).shadow_normal_bias * texel_size * pbr.world_normal;
    let point = pbr.world_position + vec4(normal_offset, 0.0);
    return shadows::world_to_directional_light_local(light_index, cascade_index, point);
}

fn field_shadow_depth(depth: f32, cascade: u32) -> f32 {
    // PS 128 subtracts this bias in forward shadow depth. Bevy uses reverse Z.
    return saturate(depth + 0.002 + 0.0005 * f32(cascade));
}

fn field_shadow_range(light_index: u32) -> f32 {
    if light_index < view_bindings::lights.n_directional_lights {
        let light = &view_bindings::lights.directional_lights[light_index];
        let count = (*light).num_cascades;
        if count > 0u {
            return (*light).cascades[count - 1u].far_bound;
        }
    }
    return 150.0;
}

fn four_shadow_comparisons(uv: vec2<f32>, depth: f32, layer: i32) -> f32 {
    let half_texel = 0.5 / vec2<f32>(textureDimensions(view_bindings::directional_shadow_textures));
    return 0.25 * (
        sample_shadow_map_hardware(uv + half_texel, depth, layer) +
        sample_shadow_map_hardware(uv - half_texel, depth, layer) +
        sample_shadow_map_hardware(uv + vec2(half_texel.x, -half_texel.y), depth, layer) +
        sample_shadow_map_hardware(uv + vec2(-half_texel.x, half_texel.y), depth, layer));
}

fn field_surface_shadow(light: MainLight, pbr: PbrInput, view_z: f32) -> f32 {
    if !receives_main_shadow(light, pbr) {
        return 1.0;
    }
    let state = &view_bindings::lights.directional_lights[light.index];
    let cascade = shadows::get_cascade_index(light.index, view_z);
    let count = (*state).num_cascades;
    if cascade >= count {
        return 1.0;
    }
    let coord = field_shadow_coord(light.index, cascade, pbr);
    if coord.w == 0.0 {
        return 1.0;
    }
    let depth = field_shadow_depth(coord.z, cascade);
    let index = i32((*state).depth_texture_base_index + cascade);
    var visibility = four_shadow_comparisons(coord.xy, depth, index);
    if cascade > 0u && visibility < 1.0 {
        let previous = field_shadow_coord(light.index, cascade - 1u, pbr);
        if previous.w > 0.0 {
            visibility *= sample_shadow_map_hardware(previous.xy, field_shadow_depth(previous.z, cascade - 1u), index - 1);
        }
    }
    let range = field_shadow_range(light.index);
    let fade = saturate(0.05 * (-view_z - (range - 20.0)));
    return mix(visibility, 1.0, fade);
}

fn main_light(pbr: PbrInput, view_z: f32) -> MainLight {
    var light = main_light_source();
    light.shadow = field_shadow(light, pbr, view_z, 1.0, 0.0);
    return light;
}

// Bevy's point and spot lights on a matte `albedo` (campfires, lamps), in
// the frame's exposed units: the game adds its local lights through its
// light pre-pass, whose path is not traced.
// SI-LGT-11: Bevy local lights instead of gsys_light_prepass.
fn local_lights(pbr: PbrInput, N: vec3<f32>, albedo: vec3<f32>, view_z: f32) -> vec3<f32> {
    let cluster_index = clustering::view_fragment_cluster_index(pbr.frag_coord.xy, view_z, pbr.is_orthographic);
    var ranges = clustering::unpack_clusterable_object_index_ranges(cluster_index);
    var input: lighting::LightingInput;
    input.layers[LAYER_BASE].NdotV = max(dot(N, pbr.V), 1e-4);
    input.layers[LAYER_BASE].N = N;
    input.layers[LAYER_BASE].R = reflect(-pbr.V, N);
    input.layers[LAYER_BASE].perceptual_roughness = 1.0;
    input.layers[LAYER_BASE].roughness = 1.0;
    input.P = pbr.world_position.xyz;
    input.V = pbr.V;
    input.diffuse_color = albedo;
    input.metallic = 0.0;
    input.F0_dielectric = vec3<f32>(0.0);
    input.F0_metallic = vec3<f32>(0.0);
    input.F_ab = vec2<f32>(1.0, 0.0);
    var local = vec3<f32>(0.0);
    for (var i: u32 = ranges.first_point_light_index_offset; i < ranges.first_spot_light_index_offset; i = i + 1u) {
        local += lighting::point_light(clustering::get_clusterable_object_id(i), &input, true, true);
    }
    for (var i: u32 = ranges.first_spot_light_index_offset; i < ranges.first_reflection_probe_index_offset; i = i + 1u) {
        local += lighting::spot_light(clustering::get_clusterable_object_id(i), &input, true);
    }
    return local * view.exposure;
}

// --- The field: ambient occlusion (program 128) ---

// The game's repeating rotations (`ssao` of SystemModel.Tex2, 4×4).
fn ssao_rotation(noise: texture_2d<f32>, noise_coord: vec2<f32>) -> vec2<f32> {
    let size = vec2<i32>(textureDimensions(noise));
    let texel = vec2<i32>(floor(noise_coord)) % size;
    return textureLoad(noise, texel, 0).rg * 2.0 - 1.0;
}

fn ssao_sample_z(uv: vec2<f32>, half_size: vec2<f32>) -> f32 {
    // SI-GBUF-05: reconstruct the point-sampled half-depth grid from Bevy's depth prepass.
    let texel = clamp(floor(uv * half_size), vec2(0.0), half_size - 1.0);
    return scene_position((texel + 0.5) / half_size).z;
}

// One direction's pair of samples, at `uv ± offset`: how much each is in
// front of the point, `k` its scale per metre.
fn ssao_pair(uv: vec2<f32>, biased_z: f32, offset: vec2<f32>, k: f32, half_size: vec2<f32>) -> f32 {
    let dp = biased_z + ssao_sample_z(uv + offset, half_size);
    let dm = biased_z + ssao_sample_z(uv - offset, half_size);
    let tp = saturate(k * dp + 0.5);
    let tm = saturate(k * dm + 0.5);
    let fp = saturate(1.0 - 0.8333333 * dp);
    let fm = saturate(1.0 - 0.8333333 * dm);
    let up = 0.5 + fp * (0.5 - tp);
    let um = 0.5 + fm * (0.5 - tm);
    return mix(um, tp, fp) + mix(up, tm, fm);
}

// The game's screen-space ambient occlusion at the pixel (1: open): four
// depth samples in two directions turned by the noise, a fixed share of
// the screen away.
fn field_ssao(frame: Frame, noise: texture_2d<f32>, light_index: u32) -> f32 {
#ifdef DEPTH_PREPASS
    // PS 128 keeps AO active through the last cascade's shadow fade.
    if -frame.p.z >= field_shadow_range(light_index) {
        return 1.0;
    }
    let depth_size = textureDimensions(view_bindings::depth_prepass_texture);
    let half_size = vec2<f32>(max(depth_size / 2u, vec2(1u)));
    let noise_coord = clamp(frame.uv * half_size, vec2(0.0), half_size - 1.0);
    let r = ssao_rotation(noise, noise_coord);
    let turn = mat2x2<f32>(r.x, r.y, -r.y, r.x);
    let aspect = vec2<f32>(view.viewport.w / view.viewport.z, 1.0);
    let a = turn * vec2<f32>(-0.0036606, 0.0364835) * aspect;
    let b = turn * vec2<f32>(0.0199001, 0.0019967) * aspect;
    // The original applies 0.995 to normalized depth, measured from the near plane.
    let near_correction = 0.005 * view.clip_from_view[3][2];
    let biased_z = 0.995 * -frame.p.z + near_correction;
    let open = 0.17764 * ssao_pair(frame.uv, biased_z, a, 14.70871, half_size)
        + 0.254544 * ssao_pair(frame.uv, biased_z, b, 10.9109, half_size);
    return 1.0 - saturate((open - 0.4321855) * 6.0);
#else
    return 1.0;
#endif
}

// --- The field: pre-shading (programs 112, 108) and main pass (32, 28) ---

// A field surface as the game's G-buffer holds it.
struct FieldSurface {
    albedo: vec3<f32>,
    // World normal.
    N: vec3<f32>,
    // Gloss (normal.w), metal (albedo.a bits 2–7), flag (normal.w bit 0:
    // ambient occlusion and rim light).
    gloss: f32,
    metal: f32,
    flag: f32,
    // 1 for foliage (material 7, `field_leaf`: programs 28 and 108 instead
    // of 32 and 112), and its albedo.a bit 1 (light passes through).
    leaf: f32,
    translucent: f32,
}

// Where the light passes through the leaves (albedo.a bit 1), the game's
// pre-shading takes their shadow from `Expand.w`, which the shadow passes
// read (programs 120, 128) write as 1: such leaves take no shadow map, only
// the clouds' (docs/CHOICES.md, LEAF-SHADOW-001: the reading of the code).
const LEAF_SHADOW_FROM_MAP: bool = false;

// The game's `skyocclusion_off` (scene_material[3].y, from
// `uking_dynamic_skyocclusion_off`): not read, taken as 0.
// SI-LGT-10: sky occlusion, ambient tint and specular terms simplified.
const SKYOCCLUSION_OFF: f32 = 0.0;

// How open the sky is above a field surface (program 112's `vis`, the
// pre-shading's `Shadow.w`): hidden where what covers it (`cover`, the
// height of the terrain's and the shading models' top above the point,
// `botw::look::sky_cover`) rises more than about a metre above the point
// (less on ground facing up with the flag), fully 4 m higher; open again
// from 70 to 90 m away.
fn sky_visibility(cover: f32, world_y: f32, n_y: f32, flag: f32, z: f32) -> f32 {
    let u = saturate(0.25 * (cover - world_y + 1.3 - flag * (2.0 * n_y - 1.0)));
    let open = 1.0 - u;
    return saturate(mix(saturate(0.05 * z - 3.5) + SKYOCCLUSION_OFF, 1.0, open * open * open));
}

// A field surface lit by the game's deferred shading: colour before the
// haze (the game's fog and `Shadow.z`, `apply_haze`), in the frame's
// exposed units. `shadow` is the main light's shadow times the clouds'
// (`Shadow.x`); `ao` the ambient occlusion (1: open); `cover` the height
// of what covers the sky above the point (`botw::look::sky_cover`);
// `means` the environment's mean brightness (`cube_mean`).
//
// `rain`: the wetness (the rain pre-shading's `Shadow.y`,
// `botw::look::field_wetness_at`) and `uking_dynamic_rainfall`; the
// rain's main pass (programs 33, 29) raises the gloss towards 0.75 and,
// once the surface is more than half wet, tilts the normal towards the
// camera (docs/research/weather.md §11.5). Nothing else changes.
fn wet_field(s_in: FieldSurface, V: vec3<f32>, rain: vec2<f32>) -> FieldSurface {
    var s = s_in;
    let wet_g = saturate(2.0 * rain.x);
    let wet_n = saturate(2.0 * rain.x - 1.0);
    s.gloss = s.gloss + (0.75 - s.gloss) * wet_g;
    s.N = normalize(s.N + V * rain.y * wet_n * (0.1 + 0.4 * s.flag));
    return s;
}

fn field_ambient_at(s: FieldSurface, z: f32, ao: f32, vis: f32, means: texture_2d<f32>) -> vec3<f32> {
    let N = s.N;
    // Pre-shading: the cube map's light along the normal (the foliage's:
    // along the normal raised to the sky), more saturated where it is
    // occluded, far away the analyzer's sky up and down.
    let amb_in = field_ambient(means);
    var ambient_dir = N;
    if s.leaf > 0.5 {
        let raised = N + vec3<f32>(0.0, 1.0, 0.0);
        ambient_dir = select(vec3<f32>(0.0, 1.0, 0.0), normalize(raised), dot(raised, raised) > 1e-6);
    }
    let cn = cube(ambient_dir);
    let M = max3(cn);
    let sat = 1.0 - min3(cn) / max(M, 1e-10);
    let occluded = 0.8 * s.flag * (1.0 - ao);
    let x = saturate(saturate(sat + 0.1 * (1.0 - vis)) + occluded);
    let c_sat = M + (cn - M) * amb_in.sat_scale * pow(x, amb_in.sat) / max(sat, 1e-6);
    let near = c_sat * amb_in.scale * (1.0 + vis) * 0.5 * (1.0 - occluded);
    let hemi = mix(amb_in.up, amb_in.down, (1.0 - N.y) * 0.5);
    // SI-LGT-10: sky occlusion, ambient tint and specular terms simplified.
    return mix(near, hemi, saturate(0.01 * z - 0.5));
}

struct FieldFootprint {
    taps: array<vec2<i32>, 4>,
    weights: vec4<f32>,
}

fn field_footprint(frame: Frame, normal_guide: f32, guides: texture_2d<f32>) -> FieldFootprint {
    let size = vec2<i32>(textureDimensions(guides));
    let center = frame.uv * vec2<f32>(size) - 0.5;
    let base = vec2<i32>(floor(center));
    let taps = array(
        clamp(base, vec2(0), size - 1),
        clamp(base + vec2(1, 0), vec2(0), size - 1),
        clamp(base + vec2(0, 1), vec2(0), size - 1),
        clamp(base + vec2(1, 1), vec2(0), size - 1),
    );
    let z = -frame.p.z;
    var depth: vec4<f32>;
    var normal: vec4<f32>;
    for (var i = 0u; i < 4u; i += 1u) {
        let guide = textureLoad(guides, taps[i], 0).rg;
        depth[i] = guide.x;
        normal[i] = guide.y;
    }
    let valid = depth > vec4(0.0);
    let normal_cost = 4.0 * abs(normal - normal_guide);
    let depth_error = abs(depth - z);
    if all(valid) {
        // PS 32 compares opposite corners before the bilinear reads.
        let depth_scale = 2.0 * abs(view.clip_from_view[1][1]) / max(z, 1e-6);
        let cost = fma(depth_error, vec4(depth_scale), normal_cost);
        let tl_br = cost.x - cost.w;
        let bl_tr = cost.z - cost.y;
        let f = clamp(fract(center) + vec2(tl_br + bl_tr, tl_br - bl_tr), vec2(0.0), vec2(1.0));
        return FieldFootprint(taps, vec4((1.0 - f.x) * (1.0 - f.y), f.x * (1.0 - f.y), (1.0 - f.x) * f.y, f.x * f.y));
    }

    // Keep the closest match when other material classes left holes.
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
    return FieldFootprint(taps, weights);
}

// RGB is diffuse ambient. Alpha is sun visibility including cloud shadow,
// or a negative value when pre-shading cannot be reconstructed.
const NO_FIELD_PRESHADING: vec4<f32> = vec4(0.0, 0.0, 0.0, -1.0);

fn field_lighting_from_prepass(
    frame: Frame,
    normal: vec3<f32>,
    diffuse: texture_2d<f32>,
    guides: texture_2d<f32>,
    shadow: texture_2d<f32>,
) -> vec4<f32> {
#ifdef DEPTH_PREPASS
    let size = max(textureDimensions(view_bindings::depth_prepass_texture) / 2u, vec2(1u));
    if any(textureDimensions(guides) != size) || any(textureDimensions(diffuse) != size)
            || any(textureDimensions(shadow) != size) {
        return NO_FIELD_PRESHADING;
    }
    let view_normal = direction_world_to_view(normal) * 0.5 + 0.5;
    let footprint = field_footprint(frame, dot(view_normal, vec3(1.0 / 3.0)), guides);
    if !any(footprint.weights > vec4(0.0)) {
        return NO_FIELD_PRESHADING;
    }
    var preshaded = vec4(0.0);
    for (var i = 0u; i < 4u; i += 1u) {
        let tap = footprint.taps[i];
        preshaded += footprint.weights[i] * vec4(textureLoad(diffuse, tap, 0).rgb,
            textureLoad(shadow, tap, 0).x);
    }
    return preshaded;
#else
    return NO_FIELD_PRESHADING;
#endif
}

fn field_compose(pbr: PbrInput, s: FieldSurface, frame: Frame, light: MainLight, S: f32, ambient: vec3<f32>, vis: f32, overcast: f32) -> vec3<f32> {
    let V = -pbr.V;
    let N = s.N;
    let to_light = light.to_light;
    // Bevy's `colour·lux/π·exposure`, mapped to the game's `env5`.
    let C = light.color;
    let c = saturate(dot(N, pbr.V));
    let n_l = saturate(dot(N, to_light));
    let m = s.metal;
    let gl = s.gloss;
    // The main pass: Lambert, GGX with Schlick's Fresnel, the sky's
    // reflection.
    let alpha = saturate((1.0 - gl) * (1.0 - gl) + 0.01);
    let a2 = alpha * alpha;
    let kg = (2.0 - gl) * (2.0 - gl) / 8.0;
    let H = normalize(to_light + pbr.V);
    let n_h = saturate(dot(N, H));
    let l_h = saturate(dot(to_light, H));
    let dd = n_h * n_h * (a2 - 1.0) + 1.0;
    let D = a2 / (dd * dd);
    let G = 1.0 / ((kg + c * (1.0 - kg)) * (kg + n_l * (1.0 - kg)));
    let F0 = mix(vec3<f32>(0.04), s.albedo, m);
    let F_L = F0 + (1.0 - F0) * pow(1.0 - l_h, 5.0);
    // The foliage's sheen and reflection are weaker where light passes
    // through, its reflection by a quarter anyway.
    let through = 1.0 - 0.5 * s.translucent;
    // PS 32/28 use uking_dynamic_cloud_ratio for direct specular.
    let clear_sky_share = saturate(1.0 - overcast);
    let spec = C * F_L * (D * G * 0.25 * S * n_l) * through * clear_sky_share;
    let F_E = F0 + (1.0 - F0) * pow(1.0 - c, 5.0) / (4.0 - 3.0 * gl);
    let env = cube_at(reflect(V, N), 3.0 - 3.0 * gl) * F_E * mix(0.25 * (1.0 + m), 1.0, vis)
        * mix(1.0, 0.75, s.leaf) * through;

    // Brighter at the edge of a silhouette, towards the light or away:
    // the depth 1.5 game pixels along the normal on screen.
    let Nv = direction_world_to_view(N);
    let e = edge(frame, 1.5 * Nv.xy);
    let n_l_signed = dot(N, to_light);
    let rim = 1.0 + saturate(0.029 * frame.fovy * e) * s.flag * 2.0 * n_l_signed * n_l_signed;

    // The direct light; the foliage's keeps 7.5 % in shadow and from behind
    // where light passes through, and fades on the far side when the sky
    // above is hidden.
    var direct = S * n_l;
    if s.leaf > 0.5 {
        direct = (direct + 0.075 * s.translucent * (1.0 - direct)) * saturate(n_l + 0.5 + 0.5 * vis);
    }
    let diffuse = (C * direct + ambient) * (1.0 - m) * rim;
    return s.albedo * diffuse + spec + env + local_lights(pbr, N, s.albedo * (1.0 - m), frame.p.z);
}

fn field_color(pbr: PbrInput, s_in: FieldSurface, frame: Frame, shadow: f32, ao: f32, cover: f32, means: texture_2d<f32>, rain: vec2<f32>, overcast: f32) -> vec3<f32> {
    return field_color_with_lighting(pbr, s_in, frame, shadow, ao, cover, means, rain, overcast, NO_FIELD_PRESHADING);
}

fn field_color_with_lighting(pbr: PbrInput, s_in: FieldSurface, frame: Frame, shadow: f32, ao: f32, cover: f32, means: texture_2d<f32>, rain: vec2<f32>, overcast: f32, shared_lighting: vec4<f32>) -> vec3<f32> {
    let light = main_light_source();
    let vis = sky_visibility(cover, pbr.world_position.y, s_in.N.y, s_in.flag, -frame.p.z);
    var S = shared_lighting.w;
    var ambient = shared_lighting.rgb;
    if S < 0.0 {
        if s_in.leaf > 0.5 {
            S = field_shadow(light, pbr, frame.p.z, shadow, s_in.translucent);
        } else {
            S = shadow * field_surface_shadow(light, pbr, frame.p.z);
        }
        ambient = field_ambient_at(s_in, -frame.p.z, ao, vis, means);
    }
    let s = wet_field(s_in, -pbr.V, rain);
    return field_compose(pbr, s, frame, light, S, ambient, vis, overcast);
}
