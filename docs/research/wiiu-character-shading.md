# Wii U: Character Lighting (`chara_*` `DeferredMain`)

Date: 2026-09-28 Subsystem: render, characters. Task:  surface lighting  (upstream reference: `../tasks/deferred-lighting.md`). Context: [ delayed lighting and water ](wiiu-deferred-shading.md), [ materials `uking_mat`](wiiu-material-variants.md).

## Question and sources

- What programs `uking_sys_shading` highlight opaque characters and
  What formulas; where do their inputs come from (environmental light, shadows, G-buffer flags).
- Dump: Wii U update v208 (EU title `00050000-101c9500`, as in neighboring ones)
  `Pack/Bootup_Graphics.pack` → `Model/SystemModel.sbfres` (model `DeferredMain`) and `Shader/uking_sys.product.sbfsha`; `uking_pass_shader.sharcb` (program `light_analyzer`); `Link.sbfres` (`Mt_Face`). All recovered in `game-data/reference/visual-formulas/`, outside Git.
- Cemu: `20260928T012415Z-cache-replay` image, exact byte mapping
  (`matches.json`); uniforms — `tools/research/cemu_uniform_map.py`.
- Ghidra: `U-King.rpx` (EU v208), read only; addresses below.
- Switch decompilation was not used.

## The result

### Materials `DeferredMain` → programs

The `SystemModel.sbfres` (`ALL_OPTIONS=1 model_info`) material options are decoded using the keys of 148 manifest `uking_sys` (model 1) programs. All matches are accurate (all `gsys_deferred_shading_pass=1`) materials; the group programs differ only from `gsys_assign_variation` 0-3.

| Materials | compare id | behave | Programme programmes | Cemu PS (variation 0) |
|---|---:|---:|---|---|
| `chara_nonmetal`, `chara_nonmetal_direct` | 8, 9 | 0 | 0–3 | `7c6e02daf538f23e` |
| `chara_metal` | 10 | 1 | 4–7 | `1bd1c50969e7e4ac` |
| `chara_grossy` | 11 | 10 | 8–11 | `ac099496a4733213` |
| `chara_hair` | 12 | 100 | 12–15 | `c580a12764680547` |
| `chara_skin` | 13 | 101 | 16–19 | `1f8a651427ddb2a8` |
| `chara_eye` | 14 | 104 | 20–23 | `b2e61dfb5275f27d` |
| `field_water` | 6 | 103 | 24–27 | `2e2543216c04766d` |
| `field_leaf` | 7 | 105 | 28–31 | `3179b85d41bfb80d` |
| `field_hybrid` | 4 | 2 | 32–35 | `8d24f32f18e6de47` |
| `preshading_chara` (type 7) | 8 | 0 | 104–107 | `1c7db40ff5d693ab` |
| `preshading_field_leaf` (type 7) | 6 | 105 | 108–111 | `fb2e18ae56397ca7` |
| `preshading_field` (Type 7, SSAO) | 4 | 2 | 112–115 | `bec68ec6f40a864f` |
| `preshading_clear` (type 8) | 0 | 0 | 116–119 | `3f3264b305b0a43a` |
| `preshading_shadow_field_leaf` (type 9) | 6 | 105 | 120–123 | `9c0b7031078fba88` |
| `preshading_shadow_chara` (type 9) | 8 | 0 | 124–127 | `45e72a252caba763` |
| `preshading_shadow_field` (type 9) | 4 | 0 | 128–131 | `88133ee405eaae28` |
| `preshading_shadow_clear` (Type 10) | 0 | 0 | 132–135 | `2364006f2b86ab25` |
| `preshading_chara_xlu`, `_field_water`, `_field_xlu` (Type 11) | 24, 22, 20 | 0, 103, 2 | 136–139, 140–143, 144–147 | see [ to ](wiiu-deferred-shading.md) |

Programs 36-103 (types 1-6) - `mat*` of other models `SystemModel` and `debug_simple`; not included in the lighting of the scene. the previous entry "PS 108 - `preshading_field`" incorrect: 108 - foliage, relief - 112.

### Frame chain for an opaque character

1. **G-buffer** (`uking_mat`, for `Mt_Face` program 10607, Cemu
   `d4670aaae44bf58c`; `Mt_Upper_Skin` — 5111. Outputs: material id (skin 13/255), albedo, normal, emission. `albedo.a·255 = 65 + ((ao.w·255 & 0xf0) >> 3)`: bit 0 = 1 (flare/alpha), bits 1-4 — the senior half-byte channel `w` `_ao0` card (transmission, `uking_transmission_channel` 40), bits 5-7 = 2 = `uking_chara_size`. Normal is written with a length of `(0.75 + 0.25·ao.x)` — **AO is encoded in the length of normal Z**; `normal.w` — quantized gly.
2. **Expand shadows** (PS 124, `preshading_shadow_chara`). Red blends
   the current cascade and baked world visibility using two distance fades.
   Green comes from `uking_tex1` sampled in the reflected view direction.
   Blue is 0.5 inside the dynamic shadow range and 1 outside. Alpha is 1.
   [Expand visibility](#expand-visibility-for-foliage-and-opaque-characters)
3. **Pre-shading** (PS 104, `preshading_chara`, 5 outputs).
   opaque character (id  ⁇  24): Shadow = (shadow from Expand, cloud shadow `sat(gsys_projection0 + proj_shadow_off)·depth_shadow_scale`, mist transmission, **0**); Fog = fog color; **Diffuse (layer 0 `gsys_light_prepass`) = 0**; Specular (layer 1) = simplified illuminated color (for refraction under water); Upscale = average coded normal. The fog is the same as water (PS 140).
4. **Main Passage** (PS 16 leather / PS 0 `nonmetal`), blend
   `src + dst·src.a` on target 0 (see water).

### Main Pass Formula (PS 16 and PS 0, read in Cemu GLSL)

Uniforms (native map): `u0..u4` — context`(tan·aspect, tan(fovy/2), fovy)`4], `uking_dynamic_toon_light_adjust_for_demo`7], [15], [16], [18] (near/far, `(tan·aspect, tan(fovy/2), fovy)`, `1/(far−near)`, `far−near`, `(W, H, 1/W, 1/H)` - layout [water-water](wiiu-water-variants.md#layout-and-coefficients)); `u5`, `u7` — environment[4], [5] (direction and color of the main light); `u6` — scene_material[2] (`.w` = `base_light_change_ratio`); `u8` — scene_material[0] = `uking_dynamic_toon_light_adjust_for_demo`PS 16: albedo 1, normal 2, linear depth 4, half-deep 6 `gsys_light_prepass` 8, `gsys_static_depth_shadow` 9 (← Shadow), `gsys_normal_modify` 11 (← Fog), depth 12.

Designations: `N` - normal, `|n|` - its length to normalization, `V` - unit vector from camera to point, `L = env4·ratio` - direction, **kuda** light goes (to light - `−L`), `A` - albedo G-buffer, `S` - Shadow, `F` - Fog, `Amb = Sem2.rgb`, `w = Sem2.w` (VS, from `gsys_user4`, see below), `C = env5` (color of light), `z` - range, `d = 1/(z·tan(fovy/2))`, `k = d·2^(chara_size−2)`, ZXQQ16.

```text
r = sat(1 + N·V) rim (0 face to camera, 1 on silhouette)
b = sat(−V·L) camera looking at light
ao = sat((sat(4|n|−3)−0.5)·4) = sat((ao−0.5)·4) by G-buffer
s = S.y·sat((S.x − 0.4)·10) shadow (cascades/baked) × clouds
t = sat(((sat(N·(−env4)) − 0.4)·75·(1 + 0.1d))) step of light
lit  = lerp(1, s, sat(w))
cw   = sat(w + s)
q    = sat(4k)·sat((e1·sat(N·normalize(V − 0.8L) + 1)·b − 0.4)·75)
       [ skin: + b·sat((f/15·sat(N·H) − 0.5)·40)·0.15] contour against light
D    = ao·lerp(sat(q), lit, t)          [nonmetal: + ((a8 & 0x1c)/28)·((a8 & 2)/2)]
Dc   = lerp(0.8·Amb, C, cw)·toon_adjust
light = Dc·ratio·D + Amb·(1 + (sat(ao + 0.8) − 1)·sat(5b))
       [ skin: + w·(1 − D)·Dc·(0.08, 0.04, 0)]
out = S.z·(A′·light) + F.rgb, out.a = bit0(a8)·S.z2
```

`H = normalize(−V − L)` (semivector), `a8 = A.a·255`, `f` - bits 1-4 `a8`. `e1, e2, e3` - screen edge detector: from the depth at the point `uv + (nₓ, −n_y)·(W⁻¹, H⁻¹)·o` , the position `P′`, `Δ = P′ − P`, `e = (V·Δ)/|Δ − V(V·Δ)|` is restored:

- `e1` = `sat(e·0.01)`, offset along the screen projection of normal by
  `6·min(k, 2)` pixels x and `1.2·min(k, 2)` y;
- `e2` = `sat(0.12·fovy·e)`, the displacement of `2·sat(k)` pixels along the normal.
- `e3` = `sat(0.12·fovy·e)`, displacement of `3·sat(k)` pixels to light
  (screen projection of `L`).

Albedo with "backlit" (`uking_deferred_enable_albedo_highlight` is not needed here - the branch is built in):

```text
g    = r·pow(sat(N·normalize(−L − 0.3V)), 20)        (nonmetal: r^0.4·pow(…, 12))
g2 = g·sat(1.2k)·normal.w2
h0   = sat((g2 − 0.01)·50)·0.05 + sat((g2 − 0.05)·20)·0.12
v    = r·e2·sat(5 − 5·V·L)·sat((sat(N·(−L)) − 0.4)·75) − 0.4
h1   = sat(lit + 0.08)·sat(h0) + sat(6k)·sat(75v)·0.4
h    = sat(sat(r·b·sat((e3 − 0.5)·40)·8)·sat(2k)·0.25 + sat(h1))
M = max(A), sat = 1 − min(A)/M
A_s = M + (A − M)·sat((1 − h)·sat)/sat discoloration
A' = A_s·sat(M + h·M^0.6·(4 − 3·sat)/M lightening
```

(plus decoding G-buffer `×1.07321 − 0.000528` and reverse `×0.931784 + 0.0004925`, together is an identity.) For an opaque character, `S.w = 0` and `light_prepass = 0`; branches with them (`(1−r)^0.2·S.w`, `prepass·v384/luma`) fall out. With `S.w = 0` , the `0.3·S.w` stepshift is zero: **is the boundary of light always at `N·L = 0.4`**, the width of `1/(75(1 + 0.1d))` through `N·L` — hence the sharp shadow on the face.

### Other classes (Cemu GLSL, same entrances)

Designations above; `sat_ = 1 − min(A)/max(A)`, `M = max(A)`, `gl = normal.w`, `fl` - bits 2-4 `a8`/28, `b1` - bit 1 `a8`, `rimL = sat(r·b·sat((e3 − 0.5)·40)·8)·sat(2k)·0.25`, `rimE = sat(6k)·sat((r·e2·sat(5 − 5V·L)· sat((sat(N·(−L)) − 0.4)·75) − 0.4)·75)·0.4`, `Hs = normalize(−L − 0.3V)`.

- **Hair, PS 12** (Cemu `c580a12764680547`): `D = fl·b1 + ao·lerp(q, lit, t)` (q without skin additive); `g = sat(6k)·gl²·pow(sat(N·Hs), 6)`;
  `h = sat(rimL + sat(h1·sat(lit + 0.5) + rimE))`, `h1 = sat(sat((g − 0.14)·70)·0.7)·sat(sat_ + 1.9M + 0.1)·0.1 + sat((sat(6k)·fl·(1 − b1)·g − 0.14)·70)·(1 − sat(3M(1 − sat_)))·0.15`; `light = Dc·sat(D) + (w·(1 − sat(D))·Dc·0.08 + Amb)·amb_mod` (warm supplement on all channels); albedo - like the skin.
- **Metal, PS 4** (`1bd1c50969e7e4ac`): shadow only half, `lit_m = 0.5·lit + 0.5`; `D = fl·b1 + ao·lit_m·t` (without q); glare in direction
  species-bound `Hm = normalize(−1.5·L_view.x, 0.6, 1)`, `g = pow(sat(N_view·Hm), e^(gl² + 3))`, `hm = sat(sat((g − 0.005)·100)·0.2 + sat((g − 0.14)·70)·0.3)`; `E = sat(lit_m·sat(N·(−L) + 0.9)·hm·8gl² + rimL)·3 + sat(D)`; `light = Dc·E + Amb·amb_mod`; albedo unchanged.
- **Eyes, PS 20** (`b2e61dfb5275f27d`): `k = d/4` (without `chara_size`)
  `fade = 0.6·sat(0.15d) + 0.4`; `D = ao·lerp(q, lit, t)·fade`; `light = Dc·D + Amb·1.8·fade·amb_mod`; over (after multiplied by albedo) the `((a8 & 0xfc)/252)·(0.6·sat(0.075d) + 0.4)·(0.8·Dc + 0.2)` glare.
- **Gloss, PS 8** (`ac099496a4733213`): Like `nonmetal`, but the glare is
  `pow(Expand.y, 1.5gl² + 1)`, which is a spherical map of `uking_tex1` ( `preshading_shadow_chara`) from the reflected view from PS 124; shadow, like metal, is only half. Not fully dismantled.

### The light of the environment: `LightAnalyzer` → `gsys_user4` → `Sem2`

The main aisle VS (Cemu `1a14de8e58d5b30a`, full-screen triangle) reads `gsys_user4` (12×1): `Sem2.rgb = lerp(T[4], T[5], v)` (v - 0 at the top of the screen, 1 at the bottom), `Sem2.w = T[0].y`.

KSys supplies `gsys_user4` from `gfx::LightAnalyzer` at `0x033ff8cc`.
The analyzer's sampler at `+0x340` points to its 12×1 result at `+0x638`
(format `0x1a`). In `uking_pass_shader`, `light_analyzer` step 5 folds 14
cube directions into `cCubeMap2D`. Steps 0–2 reduce `Expand.x`, and step 4
combines both inputs into the 12 texels.

The JP v208 RPX binds the Expand target at G-buffer `+0x2e4c` in the
reduction routine at `0x038b4d70`. The loop at `0x038b5160–0x038b53e0`
updates each dimension to `max(2, ceil(n/2))` and stops at 2×2. In the
JP shader archive, PS 479 and 481 sample red once and replicate it.
PS 483 does the same and multiplies by `cForceShadowRatio`. The last
reduction target becomes `cExposure`, which PS 487 samples at `(0.5, 0.5)`
before multiplying by `cMainLightColor.w` and cubing the result. The
intermediate `TextureSampler` is initialized with clamp and linear
filtering by `GX2InitSampler` at `0x03a82fcc`.

JP v208 `0x038b4fe8–0x038b5154` reads the Expand width and height from
the sampler at G-buffer `+0x2e4c`. For the 2D Expand texture the extracted
dimensions are the width at sampler `+0xc4` (at least 1) and the height
at `+0xc8` (at least 1). The height path also checks the value returned
by `0x03a7e470`, which returns 1 for the 2D texture. The starting
dimensions are `floor(W * width_scale * scale)` and
`floor(H * scale)`, calculated in float32. Dimensions below 2 are
replaced with 3 before the halving loop. The constructor at
`0x038b38dc–0x038b3904` initializes the two scales to `0.8` and `0.25`
from RPX constants `0x103314d0` and `0x103314d4`. With those defaults,
640×360 Expand starts the loop at 128×90, and the first draw targets
64×45 while sampling the original Expand. KSys constructs the analyzer
at `0x034071e8` and initializes its per-view resources at `0x0340720c`.
The two floats at analyzer `+0x3a8/+0x3ac` are initialized outside the
named `ksysla` parameter registrations. The traced initialization and
update path keeps the constructor values.

`0x038b4974–0x038b4994` also uploads the width scale as `cAnalyzeScale`.
VS 478 applies `u = 0.5 + (u - 0.5) * cAnalyzeScale`, leaving `v`
unchanged. Thus the first draw samples the central 80% of Expand's
width. VS 480 and 482 pass the whole preceding level through. PS 483
applies `cForceShadowRatio` at the final reduction.

The `preshading_shadow_clear` material selects class 0. Its JP v208
PS 132–135 binaries are identical and export `(1, 1, 1, 1)` to Expand.
The reduction shader has one red-channel sample per output pixel with no
material test. Background pixels written by that pass therefore count as
full visibility. The original half-resolution class buffer packs
`(material_id & 0x0e) / 255` into D16. The four Expand materials in JP
v208 `Bootup_Graphics.pack / Model/SystemModel.sbfres` use the following
comparison values and depth tests. The depth-function bits of their
material state words are 2 for Equal and 3 for LessEqual.

| Expand material | Test against half-resolution class | Red output |
| --- | --- | --- |
| `preshading_shadow_clear` | 0, Equal | 1 |
| `preshading_shadow_field` | 4, Equal | Filtered cascade and baked-world visibility |
| `preshading_shadow_field_leaf` | 6, Equal | Filtered cascade visibility, including foliage with raw ID 7 after masking |
| `preshading_shadow_chara` | 8, LessEqual | Character visibility, including masked classes 8, 10, 12 and 14 |

The character triangle writes depth `8/255`, so it passes wherever the
stored class is at least 8. This accounts for the character variants
classified separately at full resolution. The table gives the shader
writes selected by those material states. The original frame still
provides the scene-specific coverage and resource values.

The native `uking_tree` pixel variants 7 and 11 both write raw material
ID 7 after their alpha test. The class mask maps either variant to 6,
which selects the foliage Expand draw. The analyzer then samples those
red values alongside terrain and characters. [Far-tree material IDs](wiiu-field-shading.md#grass-and-distant-trees-g-buffer-uking_grass-uking_tree)

The independent `--terrain-deferred` path marks near leaves, far-tree
billboards, and character body and face materials in the camera normal
prepass. Billboards share their atlas layer, alpha cutout and model handoff
mask with the forward draw. They use the atlas normal for the class-6
visibility query. The half-resolution classifier samples the tags at the
same coordinates as terrain and scene depth. Character variants share
class 8. Both classes write visibility from the independent cascade sampler
with weight 1. Character samples skip the normal bias below the native
0.35 light-facing threshold. The independent renderer still shares the
terrain cascade filter for foliage and characters. The native receiver
offsets, character compare bias and baked world blend are traced below.

A 640×360 Hateno Vulkan replay at 10:00 in clear weather forced terrain
trees to use billboards with `BOTW_FAR_TREES_ONLY=1`. The camera prepass atlas draws
(EIDs 30804–30843) wrote 2,654 tagged pixels that survived to the final
normal and depth inputs. Of these, 674 were sampled at half resolution.
All 674 were class 6 with blue and alpha at 255 in Expand (EID 31645).
147 had red below 255. LightAnalyzer resolved to 0.705078 on the GPU
(EID 31717). CPU reconstruction from the recorded final reduction level
gave 0.705017. The normal forward mode also rendered the same camera
successfully with the billboard depth prepass enabled.

In the JP v208 KSys update, `0x034019bc–0x034019dc` writes
`1 - (KSys+0xb58)` to the alpha component of the color passed to
`0x038b48a4`. That function uploads the color as `cMainLightColor`.
`0x03408770` stores the light-switch or flash fade at `KSys+0xb58`,
and the outdoor update at `0x03657c94–0x03657cac` sends it
`min(1, switch_fade + lightning_flash)`. PS 487 multiplies the reduced
exposure by the resulting main-light ratio. The independent renderer
feeds both its analyzer and its forward fallback from `Sky::light_change`.

The final reduction's separate `cForceShadowRatio` lives at view
`+0x6ec`. It is initialized to 1 and written by `0x038b5620`.
`0x03408948` calls that setter with `1 - f2`, where the outdoor
caller at `0x03658600` supplies
`f2 = (SkyMgr+0x2174) * (1 - lightning_flash)`.
`0x03658320–0x036585b4` selects a coefficient using the wanted weather
from `0x03672890`, multiplies it by world cloudiness (`SkyMgr+0x2120`),
and chases that target into `+0x2174`. The stock reset at
`0x0364f5c0–0x0364f5ec` gives these coefficients.

| Wanted weather | Target coefficient before cloudiness |
| --- | ---: |
| Clear, cloudy, sun shower | 0.4 |
| Rain, snow, thunder rain | 0.8 |
| Heavy rain, heavy snow | 1 |
| Thunderstorm | 0 |

For positive frame factor `dt`, the chase uses
`1 - pow(0.99, dt)` with a minimum step of `0.0001 * dt`, a maximum of
`0.001 * dt`, and a snap when the distance is within the minimum step.
The update also assigns the target immediately when the world flag at
`+0x53c` is set. A settled stock cloudy state with cloudiness 1 and no
flash gives a final reduction multiplier of 0.6. The independent reducer
uploads this multiplier from the weather update and applies it during the
final 2×2 reduction. It preserves its coverage weight in alpha while scaling
visibility. The independent renderer initializes settled and forced weather
to their target values for reproducible captures. It uses the native chase
for ordinary transitions.

```text
e     = (cExposure·main.w)³,  avg = luma(cCubeMap2D)
T[0]  = (x, sat(1.2x), Master),  x = cExposure·main.w
T[4,5]= c = cube(normalize(look.x, 1 − 2g, −look.z), LOD 3),
        g = chara ambient gra y offset (+ chara ambient gra y for T[5])
        M = max(c), s = 1 − min(c)/M,
        c′ = M + (c − M)·lerp(chara_sat_min, 1, e)·(s + 0.0001)^(chara_sat − 1)
        T = c′·Master·chara_scale/(avg + lerp(chara_min, chara_max, e))·cAmbientScaleChara
```

(T[1] field, T[2,3] cube up/down, T[6,7] gray, T[8,9] effects, T[10] medium cube, T[11] thresholds light prepass.) Parameters - object `ksysla` ( `0x038b37c0` constructor, entry in uniforms `0x038b48a4` in order of announcements).

| Parameter. | Shut up. | | Parameter. | Shut up. |
|---|---:|---|---|---:|
| `chara_ambient_gra_y` | 1.0 | | `ambient_offset_chara_min` | 0.1 |
| `chara_ambient_gra_y_offset` | 0.0 | | `ambient_offset_chara_max` | 0.5 |
| `chara_ambient_sat` | 0.5 | | `ambient_offset_chara_scale` | 1.75 |
| `chara_ambient_sat_min` | 0.75 | | `ambient_offset_field_min/max/scale` | 0.1 / 0.5 / 1.0 |
| `field_ambient_sat` / `_min` | 0.5 / 0.8 | | `ambient_offset_cubemap` / `_far` | 0.01 / 1.0 |
| `effect_ambient_sat` | 2.0 | | `light_prepass` / `_threshold` / `_offset` | 0.1 / 3 / 3 |

`cAmbientMasterIntensity` (+0x6e8) defaults to 1.0.
`cAmbientScaleChara` (+0x6d8) starts from global vec4 `0x1054a51c`
and is updated by `0x038b55c4`. `cLook` is the horizontal camera
look vector. `cMainLightColor.rgb` is `dir_main` color times intensity
(`0x030c04ec`).

**Interpretation.** The light of the character's surroundings **It doesn't depend on the normal.**: these are two colors of the sky (cube maps of the environment) - above and below the view, with raised saturation and normalized for the average brightness of the environment, with a smooth transition from top to bottom on the screen. `w = sat(1.2·sunshot)`: in the shadow (forest, cave) shadows on the character weaken, direct light is colored in `0.8·Amb`.

### Expand visibility for foliage and opaque characters

The JP v208 `uking_sys` pixel code is identical within variants
120–123 (`preshading_shadow_field_leaf`) and within variants 124–127
(`preshading_shadow_chara`). The former covers the masked class 6 used
by near foliage and the two traced distant-tree variants. The latter
covers opaque classes 8, 10, 12 and 14, including skin, hair and
equipment materials that use this Expand draw. Translucent passes
consume the resulting pre-shading targets later in the frame.
Grass variants with raw class 4 join the terrain writer.

Both writers sample half normalized linear depth, G-buffer normal and
albedo, and the cascade array. The sampler slots for those four inputs
are `[5, 2, 1, 7]` in PS 120 and `[6, 3, 2, 7]` in PS 124.
They select the current cascade from `gsys_context[58].xy` using strict
view-depth comparisons. Each uses four hardware comparisons at all
sign combinations of a half-texel U and V offset. The texel sizes come
from `gsys_environment[53].xy`. The shader averages the four comparison
results equally.

Here `z` is positive view depth, `P` is the reconstructed view-space
point and `N` is the normalized G-buffer normal. `G` is the normalized
cross product of the horizontal depth-neighbor difference and then the
vertical one. Both differences are relative to `P`. The shaders form
`P.xy` from the half-depth sample and `P.z` from a gathered depth tap.
`D` is `environment[4].xyz * scene_material[2].w` and points along light
travel. Let `M = normalize(1.5 * N - G)`, `d = dot(M,D)` and
`Q = M - d*D`. `fadeNear` fades dynamic visibility to 1 over the last
20 m, starting at `environment[53].z`.

**Foliage.** The normal byte's low bit `h` gates the depth-neighborhood
offset. The albedo byte's bit 1 gives `t`, a 0 or 1 metre offset along
the surface normal. `Z` is the projected depth of the offset point.
`i` is the selected cascade index.

```text
point        = P + t*N + 0.0791*h*(1 - d*d)*sat(-dot(N,D))*Q
compareDepth = sat(Z - 0.002 - 0.0005*i)
C            = average(four comparisons in cascade i)
if i > 0 and C < 1:
    C *= one center comparison in cascade i - 1, using its projection and bias
Expand.r     = sat(lerp(C, 1, fadeNear) * depth_shadow_scale + depth_shadow_off)
```

ALU groups 20–46 select the cascade, offset the receiver and form the
sample coordinates. Groups 51–75 apply the average, optional previous
cascade and fade. Group 77 applies the material scale and offset. The
dynamic samples are skipped when `fadeNear` reaches 1. Far visibility
for this writer is 1 before the material scale and offset.

**Opaque characters.** The albedo alpha byte `a` supplies the compare
bias and the surface-facing test gates the receiver offset. The camera
term in that offset is `gsys_context[17].y = tan(fovy/2)`.
`B` is the fourth power of the clock-weighted baked RGB sample.

```text
T            = ((a & 0x1c) / 28) * (1 - ((a & 2) >> 1))
gate         = 1 if -dot(N,D) >= 0.35 else 0
point        = P + 0.003*gate*(1 + z*tan(fovy/2))*(1 - d*d)*Q
compareDepth = sat(max(0.00001, Z - 0.0015 - 0.05*T))
C            = average(four comparisons in cascade i)
dynamic      = lerp(C, 1, fadeNear)
fadeFar      = sat(0.05 * (z - environment[53].w))
dynamicMix   = sat(1 + world_shadow_off - fadeFar)
bakedUV      = (worldX*0.0001 + 0.5, worldZ*0.000125 + 0.5)
B            = dot(sampleLevelZero(solar_irradiation, bakedUV).rgb, environment[42].rgb)^4
Expand.r     = sat(lerp(B, dynamic, dynamicMix) + depth_shadow_off)
```

PS 124 uses only the current cascade for `C`. Groups 12–21 unpack the
albedo flags, groups 26–54 form the receiver and comparison coordinates,
and groups 59–74 average and blend the two sources. World X and Z come
from applying rows 11 and 13 of `gsys_context` to the view-space point.
The dynamic sample is skipped once `fadeNear` reaches 1. With
`world_shadow_off` at zero,
the baked blend starts at range minus 40 m and reaches its full weight
at range minus 20 m. PS 124 reads `solar_irradiation` through
`gsys_dynamic_reflection` at slot 14. Its asset and clock-derived
weights are shared with the field shader. [Baked texture and clock weights](wiiu-field-shading.md#far-shadow-weights-by-time-of-day)

An offline interpreter checked selected ALU groups with 2,048 inputs
per check for fade and cascade selection, baked UVs and blend, alpha-byte
bias, depth-neighbor reconstruction, receiver offsets and comparison
coordinates. The checks used synthetic comparison values. Recomputing the
analyzer from the archived independent Vulkan readbacks reproduced the GPU
result to within 0.0001 in the clear, cloudy and clear-night Link views.
The separate billboard replay above verified class-6 coverage.
A native replay at the requested views can read the draw-time sampler
state and the resulting Expand and analyzer values.

#### Live class-7 foliage in frame 3830

Wii U JP v208, Cemu 1.26.2f Vulkan, zero graphics packs. Use the same
[frame 3830 capture](wiiu-field-shading.md#live-terrain-material-sample-in-frame-3830)
and read RT0 as raw RG8. Pixel History and before/after buffers identify
two distinct writers:

| Full-resolution pixel | Last RT0 writer | Pass and inputs | RT0 before → after |
| --- | ---: | --- | --- |
| (579, 287) | 4000 | 2,340 indices. PS3 samples BC3 sRGB 256×256 and discards when its alpha is below the bound alpha-test reference. D32 write on, LessEqual. | `04 00` → `07 00` |
| (205, 233) | 4176 | 948 indices. PS3/PS4/PS5 sample BC1 sRGB 200×200, BC1 UNORM 100×100 and BC4 UNORM 200×200. D32 write off, Equal. | `04 00` → `07 40` |

EID 5774 reads full-resolution RT0.R, applies a bit mask, and writes
the result as half-resolution D16 depth. It maps these samples to
`(289,143)` and `(102,116)`. Read the active 640×360 viewport of
resource 12332, whose backing image in this replay is 640×368.
For 13,309 half-resolution cells with class 7 in all four corresponding
full-resolution neighbors at EID 5773, every half-resolution result is
D16 `1542 = 6 * 257`. The mask `7 & 0x0e` removes the low bit for
this pass.

The two sample cells pass D16 Equal at Expand EID 9953. Its shadow
sampler is `sampler2DArrayShadow` on PS7, with albedo, normal and
half-depth on PS1, PS2 and PS5. This independently matches the static
foliage writer's slot layout. The raw RGBA8 Expand output changes from
`ff 1d fb ff` and `ff 1d ff fc`, respectively, to `ff 00 ff ff`
at both cells in resource 12397. Pixel History shows pre-shading EID
11015 passing depth at both cells. These particular samples have red
visibility 1. They establish routing and the bound depth array, while
numeric PCF checks need partially shadowed samples.

Full-resolution classification at EID 12831 stores D16
`1799 = 7 * 257` at both original pixels. EID 12860 uses Equal and
changes their raw R11G11B10F HDR words from zero to `0x619a9b47` and
`0x5dd9ab5f`. Compare direct readbacks at EID 12859 and 12860, since
Pixel History misses this write in the replay. Thus the low-bit mask
groups the samples for half-resolution lighting while full-resolution
composition still selects raw class 7. The exact object names and
native permutation keys of these two writers remain open.



#### Independent renderer

The same half-resolution Expand pass routes near leaf meshes and far
billboards to the class-6 writer. Their alpha cutout controls coverage.
For covered pixels, a separate material bit selects the 1 m receiver
shift. The traced crown program sets that bit, while the billboard
renderer uses the `NrmTrs` blue channel as in program 7. The class-8
writer covers opaque characters in the independent prepass, including
skin with a transmission map and equipment. The PS 124 transmission
bias comes from the same material map and UV set as character shading.
Both writers keep full weight wherever they cover an Expand pixel.
LightAnalyzer uses their red values through its existing reduction.

`bake --only light` exports level 0 from the base game's
`Model/SystemModel.Tex1.sbfres` to `light/solar_irradiation.ktx2`. The
update's Tex2 entry holds the smaller mip. The renderer samples level 0
with repeat and bilinear filtering, as specified by the stored material's
[Latte sampler fields](https://github.com/cemu-project/Cemu/blob/main/src/Cafe/HW/Latte/ISA/LatteReg.h).
The character blend uses the clock's three RGB weights. With an older
asset directory that omits the map, baked visibility defaults to 1.

Offscreen validation used 640×360 at Hateno, clear 10:00, and checked
the 320×180 Expand image and all five LightAnalyzer reduction levels.
The last column removes only the named class from the CPU reference.

| Independent scene | Class samples | Red values below 255 | GPU resolved share | CPU share with that class excluded |
| --- | ---: | ---: | ---: | ---: |
| Vulkan, distant-tree billboards | 4,096 | 2,979 | 0.700195 | 0.742416 |
| DX12, close Link | 487 | 467 | 0.824707 | 0.830786 |
| DX12, five characters at surveyed depths | 28 | 28 | 1.000000 | 1.000000 |

Both of the first two scenes wrote red values from 0 through 255.
The final CPU reduction with all samples matched the GPU within
0.0004 across the three scenes. In the billboard capture (Expand draw
25156), replayed depth, normal and cascade textures reproduced 4,092
of 4,096 red values within two 8-bit steps. The four outliers remain
unresolved. In the controlled character scene, 16 pixels at 161 and
172 m fell within the expected dynamic/baked blend bounds. The 12
pixels at 182, 200 and 210 m read 224–229. That fully baked range was
within the CPU BC1 sampling bounds around the corresponding placements.
At this camera, the sparse character pixels did not affect the final
reduction sample. The close Link readback verifies their contribution
under a camera that covers more of the frame.

The port uses Bevy's reverse-Z cascades and the selected full-resolution
depth taps for the receiver and its two neighbors. It assumes scene
material depth scale 1 and both depth and world shadow offsets 0. Native
draw-time uniforms, depth gathering at silhouettes and the camera's
cascade map content can affect the Wii U result. Other material programs,
such as grass and translucent blending, have their own routing.

## Reproduction and limitations

```sh
REF=/absolute/game-data/reference/visual-formulas
ALL_OPTIONS=1 target/release/examples/model_info $REF/shading/SystemModel.sbfres
python3 tools/research/cemu_uniform_map.py \
  $REF/shader-archives/uking_sys/model001-program00016-ps.code \
  $REF/cemu-sessions/20260928T012415Z-cache-replay/shaders/1f8a651427ddb2a8_000000000f249a49_ps.txt
python3 tools/research/cemu_uniform_map.py \
  $REF/shader-archives/uking_pass_shader/00487-stage1.code \
  $REF/cemu-sessions/20260928T012415Z-cache-replay/shaders/728b9bc3556c3de1_0000000000001ec9_ps.txt
```

Verified: Options matching → programs (exact), uniforms cards PS 16, PS 0, PS 104, VS 16, LightAnalyzer step 4 (tool), CPU addresses. The earlier character-composition formulas were read from GLSL. The Expand red arithmetic was checked separately against GX2 instruction groups. `cMainLightColor.w` outside the default branch, `cAmbientScaleChara`writer `toon_adjust` (scene_material[0]How G-buffer reads hair `uking_grossy_intensity` (2, FORMATS (archived reference)), loading `ksysla` (file not found), which cube map is tied as `cCubeMap`/`gsys_cube_map`, full formula `chara_grossy` texture `uking_tex1`difference `nonmetal_direct` (id 9 with the same programs 0-3), flags `albedo.a` materials other than the face. task (upstream reference: `../tasks/deferred-lighting.md`).
