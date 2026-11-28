# Paired visual baseline at Tarrey Town

Original reference: Wii U JP v208, Cemu 1.26.2f Vulkan, RenderDoc 1.38,
zero graphics packs, frame 3830. The primary goal is a visually close
outdoor reproduction at one reproducible camera, followed by weather
and movement replays. The historical independent Hateno checks use
their own cameras. This original frame is from Tarrey Town. The
camera is 19 m horizontally from the `UMiiVillage` marker and the
scene color shows the village's building shapes.

## Camera and clock

At Field Expand E9949, `uniformBlockPS1[11..13]` gives the rows of
the inverse view. Its translation is
`(3945.8937988, 235.9955139, -1616.5034180)`. The negative third
column gives the world-space view forward. The equivalent renderer
yaw and pitch are `−158.5634605°` and `−6.6071439°`. The view has
negligible roll. The vertical projection scale `2.144506693` from
row 8 gives `2 atan(1 / scale) = 50°`; context row 14 gives the
original clip distances as 1 m to 25,000 m. Use a 1280×720 view.

The original displayed game clock reads 17:40. Start there with
`bluesky` and freeze the independent scene clock. The HUD only
establishes the displayed clock interval. The exact solar and cloud
state must be compared using constants and scene readbacks.

```sh
cargo bake --region umiivillage --radius 1000 --out assets-tarrey
cargo render --assets assets-tarrey --place umiivillage --camera 3945.8937988,235.9955139,-1616.5034180,-158.5634605,-6.6071439 --projection 50,1,25000 --time 17:40 --weather bluesky --resolution 1280x720 --terrain-deferred --freeze-animation --screenshot captures/tarrey-3830.png
```

Create the capture output directory first. On Windows, set
`WGPU_BACKEND=vulkan`. To collect scene HDR in the same offscreen
frame, launch the binary with `renderdoccmd capture -w` and include
`--renderdoc` in its arguments.

## Readback points

Export original presented color from the 1280×720 swapchain image.
Export original scene HDR at E14247 from `ResourceId::12282`,
R11G11B10F, 3,686,400 tightly packed bytes. The following draw is the
display compose. Sample (640, 45) decodes to linear RGB
`(0.515625, 0.6640625, 0.4375)`.

For the independent screenshot frame, read the full-size scene source
bound at the `hdr_compose` fragment entry point. Compare that before
comparing the presented PNGs. Check roof and terrain edges for
alignment first, then compare sky and distant terrain, direct-lit
and shaded materials, and contact AO. Keep the original HUD, Link,
NPCs, and moving effects out of the static pixel samples.

Keep the replay, assets and images local.

## First matched-camera comparison

Built the render crate at `ac7d5cd` and ran the frozen 1280×720
Vulkan configuration above with the regional bake and the
2000×1600 solar irradiation texture. Roof ridges and wooden path
segments line up visually, including the roof ridge near (500, 61).
The Bevy camera still uses its reversed, infinite depth projection.
The command uses the original far clip value for renderer culling.

The independent RenderDoc replay exposes a tightly packed
1280×720 RGBA16F input to `hdr_compose` at E28051
(`ResourceId::1761`, 7,372,800 bytes). The independent PNG was
saved in a separate, directly launched Vulkan run with the same
settings. Treat these as representative image samples. The
scene-HDR encoding and display pipeline differ across the two
renderers.

| Screen point | Wii U scene HDR RGB | Renderer scene HDR RGB | Wii U display RGB8 | Renderer display RGB8 |
| --- | --- | --- | --- | --- |
| Open sky (640, 45) | (0.516, 0.664, 0.438) | (0.468, 0.619, 0.434) | (166, 187, 151) | (152, 178, 158) |
| Distant ridge region (873, 75) | (0.340, 0.395, 0.199) | (0.435, 0.380, 0.215) | (146, 156, 107) | (159, 125, 100) |

Rank the remaining differences by their impact in this view:

1. Object state. The independent scene draws large rocks in front of
   several existing houses. Vegetation and small effects differ
   around the plaza. Omit scene-state mismatches from lighting metrics.
2. Atmospheric and display color. Distant slopes in the independent
   image are more contrasty and orange. The color difference is
   visible before the display pass at the sampled ridge point.
   Compare fog transmission and additive fog first. Then feed the
   same HDR input into each display curve to isolate the display term.
3. Local occlusion and material detail. Check a matched object-ground
   contact and the wall in shadow after the rock placements match.
   Use class mask, Expand blue and visibility to distinguish AO from
   shadowing. Defer roughness and reflection fitting to surfaces that
   expose those terms.

The direct Vulkan screenshot run logged 34.87 ms mean over 300
post-load frames, with 386 terrain tiles resident and zero loading
at the measurement end. This is a whole-frame desktop observation
with this workload.

### Controlled object-state check

The baked `I-3` cell contains six
`TwnObj_Village_IchikaraHouse_DCRock_A_*` placements alongside
the village house placements. The current bake copies the actor
name, hash and transform into the placement file. Its placement
record has no original activation condition. In a temporary A/B input,
remove only those six rock placements and rerun the same screenshot
command. The large rocks in front of the center and right houses
disappear, and the houses are visible in the same positions as the
reference. Other vegetation, effects and rock differences remain.
The original baked cell was restored and verified byte-for-byte.

Trace the original activation condition for the rocks and houses from
the map or game-state data. Use that for the stable lighting baseline.
The controlled filter is a diagnostic. Leave it out of the production
bake and renderer. The next lighting comparison is distant-slope fog
transmission and in-scattering in this camera.

### Save flags and the paired scene bake

The original JP v208 static `I-3` map unit contains five `LinkTagAnd`
sources with `SaveFlagOnOffType = 0` and output links `Delete` and
`Create`. The `UMiiMini_MakeVillage_` flags select these rock removals:

| Flag suffix | Rock suffixes removed |
| --- | --- |
| `Activated` | `02` |
| `Invite01` | `06` |
| `Invite02` | `04` |
| `Invite03` | `01` |
| `Invite04` | `03`, `05` |

The `Create` edges also target village mesh instances. The reference
visuals motivate a diagnostic snapshot with all five flags active.
The bake's repeatable `--map-delete-flag` option reads the matching
static `Delete` edges and applies them during the objects step. It
does not interpret the save file or execute the full link runtime.
The option is opt-in and can be combined with `--only objects`
to reuse the same terrain, lighting textures and model assets.
In `I-3`, the snapshot removed 36 placements, including all six
rocks, and added zero. The Vulkan comparison shows that the occluding
rocks are gone while the corresponding houses remain.

### Identify the first large far-slope discrepancy

Read the original pre-shading outputs after E11011 and the old
independent outputs after pre-shading has stored the attachments.
The renderer guide stores view depth directly. Decode the original
normalized depth with `1 + 24999 × depth`. Transmission and sun
visibility are channels B and R, respectively, in the UNORM8
pre-shading target.

| Half pixel | Reference and renderer depth (m) | Transmission byte, original → renderer | Sun byte, original → renderer |
| --- | --- | --- | --- |
| (436, 37) | 297.52 / 296.94 | 105 → 108 | 35 → 255 |
| (425, 49) | 240.85 / 240.62 | 112 → 115 | 216 → 255 |
| (430, 50) | 240.38 / 240.32 | 112 → 115 | 201 → 255 |
| (437, 45) | 295.36 / 294.78 | 105 → 108 | 33 → 255 |

At (436, 37), additive fog RGB is (0.2422, 0.3086, 0.1680) in the
original and (0.2324, 0.2969, 0.1641) in the old renderer. The sun
visibility discrepancy is substantially larger. The original field
path blends in the solar irradiation texture beyond the near
cascades. The independent field Expand path previously returned
the near cascade result and faded to full sunlight. An initial
terrain-only patch left the selected ridge samples at 255. Reading
the independent classify attachment at those points gives D16 value
1285 (class 5, static field object). The native class-4 field
coverage at these locations includes that geometry. Both independent
field classes now use the existing baked-shadow blend shared with
the character path. The field classes still use the renderer's
near-cascade setup.

The subsequent Vulkan replay successfully ran both Expand and
composition. Pre-shading E24502 produced these visibility bytes
with the shared field path:

| Half pixel | Original | Old renderer | Shared baked shadow |
| --- | ---: | ---: | ---: |
| (436, 37) | 35 | 255 | 34 |
| (425, 49) | 216 | 255 | 222 |
| (430, 50) | 201 | 255 | 200 |
| (437, 45) | 33 | 255 | 34 |

In the renderer's Expand readback beyond 210 m, visibility below
255 appears on 14 of 150 class-4 cells and 8,843 of 12,554 class-5
cells. At the matched sample points, the fog bytes and RGB values
are unchanged by the fix. The remaining comparisons are the
near-cascade range and filter, plus the residual fog tint and
display mapping.

### Cascade split A/B

Replay the same object snapshot and camera with the renderer's default
`32,64,200` m bounds and with
`--shadow-cascades 7.790757,24.622145,120`. These trial bounds come
from the original frame's E9949 context buffer and are verified in
the trial's directional-light uniform. Both renderer runs use the
same Bevy shadow map dimensions and sampling code. The estimated
game-versus-renderer directional light angle differs by about
0.012 degrees in this view.

Compare Expand red before cloud attenuation. Match original class 4
against renderer class 4 or 5, require reference view depth within
max(0.1 m, 1%) of renderer depth, and require the two renderer depths
within 0.001 m. Record mean absolute errors in visibility bytes.
The shaded subset has original visibility below 243.

| View depth (m) | Matched pixels | All pixels, default → trial | Shaded subset, default → trial |
| --- | ---: | ---: | ---: |
| 2–7.79 | 62,325 | 6.256 → 6.983 | 68.137 → 75.719 |
| 7.79–24.62 | 62,829 | 10.029 → 9.212 | 26.015 → 23.219 |
| 24.62–80 | 27,950 | 9.338 → 7.917 | 17.312 → 19.925 |
| 80–120 | 0 | — | — |

Of the 1,034 nearest pixels darker than 128 in the original and
at least 224 in the trial, 739 lie in three 64×48 full-resolution
tiles around Link's shadow. Those pixels are also bright in the
default run, which has no corresponding shadow caster. The split
override is useful for replication. It does not remove the need to
match caster coverage or the original shadow projection and PCF.
Keep the default splits until more views establish a useful policy.

### Same-input display curve verification

At original display compose E14248, PS1 reads the 1280×720 scene
HDR. PS0 exposes a 320×180 Bloom texture with five mips. The scene
samples agree with the corresponding full-resolution texels. The
Bloom samples match bilinear sampling of mip 1, 160×90.

In an offline shader replacement, output each sampled RGB through
`c / (1 + c)` into the original 10-bit attachment. Invert that
encoding for the comparison. Sample 20 positions at x = 450, 640,
873 and 1050 and y = 45, 150, 290, 465 and 600. For Bloom, the
mean channel difference between GPU sample and bilinear CPU sample
is 0.00803 using mip 0 and 0.000261 using mip 1. This comparison
includes quantization from the diagnostic encoding.

Apply the renderer's exponential curve and saturation with the
original bindings' raw inputs and mip 1. This reproduces 59 of
60 original 10-bit channels exactly and the remaining one within
one code. Using mip 0 instead had a maximum error of 40 codes.
The independent renderer already composes its 1/8-resolution
Bloom level. Compare upstream HDR and Bloom production before
changing the curve for this case.

### Separate HDR, Bloom and field visibility

Hold the verified curve and substitute scene HDR and Bloom one at
a time between original E14248 and the independent compose draw.
At 2,551 far-slope screen samples, require the reference and renderer
depths to agree within max(0.1 m, 1%) and Expand red within eight codes.
The independent HDR substitution costs 48.67 mean absolute codes
per 10-bit RGB channel. The Bloom substitution costs 10.97. On sky
samples the corresponding errors are 28.38 and 1.06.

Pixel History at (873, 75) identifies original opaque field
composition at E12855 and a later overlay at E13778. In the
independent frame, the placed-object material draw at E24645
contributes brighter radiance over the nearby field result.
Recomposing four same-depth distant points with the original fog
has little effect on their combined HDR error. The reconstruction
uses half-resolution fog samples, so it estimates the effect without
reproducing the full-resolution gather or later overlays.

The object material reconstructed the pre-shaded environment
light and recomputed sun visibility using only its dynamic
shadow route. The revised object material also reconstructs
the shared pre-shading shadow's red channel using the existing
depth/normal footprint. That channel includes the baked far shadow
and cloud attenuation. Other materials and missing guides use the
existing forward lighting calculation.

The repaired Vulkan capture writes the shared shadow target at
E21662. RenderDoc lists 675 fragment read events, beginning with
terrain composition at E21688 and followed by material draws.
With the same 2,551 far-slope points, mean RGB absolute error
falls from 52.95 to 45.75 in 10-bit output codes. Sky error remains
29.10 codes. At (873, 75), renderer scene HDR moves from
(0.435, 0.380, 0.215) to (0.279, 0.321, 0.180). The original
opaque composite is (0.266, 0.320, 0.172) and the original HDR
after its later overlay is (0.340, 0.395, 0.199). Keep those event
boundaries distinct when making the next color comparison.

The original overlay reads a 640×360 RGBA16F effect buffer copied
at E13769. It gathers half-resolution R32F depth and samples
full-resolution R16F depth to guide the upscale. E13778 writes
to full-resolution HDR with RGB blend `src + dst × src.a`.
At half pixel (436, 37), E13765 supplies the only color after the
buffer clear, approximately (0.0843, 0.0894, 0.0349, 0.9629).
The visually significant pass is thus outside the opaque field
formula. Its originating effect or emitter is still open.

### E13765 effect geometry and mask

Replay of the original Wii U frame confirms the following resource chain
in Cemu Vulkan. The image extents describe replay resources, which include
padding in the source attachment.

| Events | Draw or copy | Observed result |
| --- | --- | --- |
| E10797–E10819 | Mask draws sample a 512×512 BC4_UNORM texture. The first and last draws in this range each submit 144 indices and use source-alpha RGB blending. | A 160×90 viewport writes into R16F attachment 12571, whose allocation is 192×96. |
| E13751 | Copy from 12571. | Sampled R16F resource 12607 is 160×90. |
| E13765 | 480 indices, triangle list, one instance, with depth testing disabled. The fragment shader samples only resource 12607. | RGBA16F attachment 20013 changes at 188,826 texels, with an inclusive affected rectangle of (0, 0) to (639, 323). The attachment is 640×368. |
| E13769 and E13778 | Copy to a 640×360 sampled RGBA16F image, then use a full-screen triangle to composite into 1280×720 HDR. | The fragment shader samples full-resolution R16F and half-resolution R32F depth to guide reconstruction. |

E13765 uses projected mask UV `Sem3.xy / Sem3.w`. Its fragment RGB is
`Sem0.rgb × Sem7.w × cb0.rgb × cb0.w`. Its source alpha is
`Sem4.x × saturate(Sem0.w × Sem7.w) × saturate(mask.r × 0.5)`,
subject to the shader's alpha test. The blend writes
`RGB = src.rgb × src.a + dst.rgb × (1 − src.a)` and
`A = dst.a × (1 − src.a)`. E13778 then uses `src.rgb + dst.rgb × src.a`.
The effect buffer alpha therefore carries the surviving fraction of scene
HDR through those blends. A constant fog color would discard the observed
variation from the earlier mask and mesh draws.

### Full-frame E13778 reconstruction and E13765 A/B

The [RenderDoc exporter](../../tools/research/renderdoc_overlay_ab_export.py)
reads the frame 3830 buffers and fragment constants. Set
`BOTW_FOG_ORIGINAL` to the capture path and `BOTW_FOG_OUT` to an existing
output directory. Run the exporter with `qrenderdoc --python=<script>`,
then run `python tools/research/compare_overlay_ab.py <output directory>`.

For normalized pixel-center UV `uv`, start with the fractional position of
`uv × halfExtent − 0.5`. Gather the four half-depth taps in that footprint
and compute absolute differences from the full-resolution depth `z`.
Let `l`, `r`, `t`, and `b` be the sums on the left, right, top, and bottom.
Add `(l − r, t − b) × 4.2890134 / (z + 0.0000400016)` to the fractional
position and clamp it to the four-tap footprint. Bilinearly sample RGBA
there and apply `effect.rgb + scene.rgb × effect.a`. The fragment shader
adds 0.0001 to the depth-gather UV. At 1280×720 that offset preserves the
four-tap footprint of each full-resolution pixel center.

Against the captured R11G11B10F HDR, the CPU reconstruction yields
0.000167 mean absolute error per linear HDR channel for all 921,600
pixels, with exact RGB for 89.26% of pixels using truncation to the
destination format. Truncation was chosen empirically for this Vulkan
replay. The maximum error is 0.5 at a bright pixel whose captured blue
channel is 22.0, one destination code above the reconstruction.
On the 3,476 sky/geometry edge pixels defined below, plain bilinear
upsampling gives 0.01555 mean error per channel. The depth-guided
position reduces it to 0.000154 on the same pixels.

Holding the guides and pre-composite HDR fixed, replacing the effect input
with the active 640×360 area immediately before E13765 gives these
mean signed linear HDR differences (with the draw minus without it):

| Sample region | Pixels | ΔR | ΔG | ΔB |
| --- | ---: | ---: | ---: | ---: |
| Far-slope box, x 848–894 and y 64–83 | 940 | 0.0737 | 0.0773 | 0.0283 |
| Sky, full guide equals 1 | 37,370 | 0.0576 | 0.0583 | 0.0184 |
| Sky/geometry edges, half gather spans below 0.999 to at least 0.99999 | 3,476 | 0.0531 | 0.0577 | 0.0201 |
| All pixels | 921,600 | 0.0237 | 0.0270 | 0.0098 |

Use the full-frame comparison as the checkpoint for integrating the HDR
composite. The mask asset, mesh producer and frame-to-frame inputs still
need identification.

### Renderer Vulkan replay of E13778

At 1280×720, set `BOTW_OVERLAY_REPLAY` to the directory produced by the
original-frame exporter. The renderer uploads the original pre-composite
HDR, half-resolution effect and both depth guides. The WGSL
`composite_overlay` pass runs before the renderer's bloom mask and
supplies the same HDR image to the tone pipeline. This is a fixture mode
for validating the math, including the sampler and color format. A normal
run uses the renderer's own HDR inputs.

Capture the renderer with `renderdoccmd capture -w -c <path>` and the
paired command above, adding `--renderdoc`. Set
`BOTW_OVERLAY_RENDERER_CAPTURE` to the renderer capture and reuse the
RenderDoc exporter. Pass `--gpu` to the comparison script to include the
renderer readback.

The Vulkan replay drew `composite_overlay` at E28104 and wrote RGBA16F.
Against the unquantized CPU calculation, its full-frame RGB mean
absolute error is 0.0000879 per channel. After truncation to the capture's
R11G11B10F format, mean error versus the game is 0.0001277 with exact RGB
on 91.09% of pixels. In the 3,476 sky/geometry edge samples, mean
error is 0.0001446, with exact RGB on 93.21%. The largest absolute
full-frame error is 0.5. A default-mode 640×360
Vulkan screenshot verified that the original viewer path still runs.

A live scene needs the two E13778 depth guides and the half-resolution
effect buffer produced by the renderer.

### Resident haze meshes and the two mask snapshots

The original Wii U v208 capture exposes two snapshots of the same R16F
attachment. E10713 copies attachment 12571 to sampled texture 12607,
which E10724 reads at PS1 while generating resource 12606. Then
E10745–E10819 submit 38 draws to attachment 12571. All use the same
vertex and pixel programs, one instance of a 144-index mesh, and the
same 512×512 BC4_UNORM input. Seventeen draws change the R16F target.
Comparing the active 160×90 region immediately before the sequence
and after E10819, 14,399 out of 14,400 texels change (11,426 decrease,
2,973 increase). The mask mean falls from 1.36484 to 0.86510. E13751
copies the updated attachment to 12607, which E13765 reads at PS9.

Cross-check against the matching local Wii U bake of
`Pack/Bootup.pack → Effect/GameResident.sesetlist`:

- The mip-0 BC4 payload (131,072 bytes) of resource 27375 is identical
  to the mip-0 KTX2 payload of `GameResident/69b453c0.ktx2`.
- At E10819, all 144 indices and all 33 XYZ positions match resident
  primitive 3500784928. Primitive 1775848997 shares that mesh data.
  Intersecting the mesh and texture IDs in the emitter definitions
  leaves `Haze_Far` in `VolumeMaskFog02` and
  `VolumeMaskAdd01test`. The captured squared texture, disabled
  depth test and source-alpha RGB blend match `VolumeMaskAdd01test`
  (path 24, blend type 0). `VolumeMaskFog02` selects the unsquared,
  depth-tested additive variant. `VolumeMaskAdd01` shares the shader
  and texture but uses primitive 3705724266.
- At E13765, all 480 indices and all 102 XYZ positions match resident
  primitive 1076106540, used by `Haze_Far_simple_new` in
  `VolumeMaskFog01` and `VolumeMaskFog01_Demo`. Those definitions
  share the shader pair. Their draw paths are 8 and 7 respectively.
  The field frame favors `VolumeMaskFog01`. The running set instance
  still needs confirmation.

For reproduction, use the RenderDoc texture readback at mip 0,
the draw's post-VS index stream, and the raw position vertex stream.
E10819 binds position at Sem9 and E13765 at Sem3, both with a 16-byte
stride. Compare the first three components after GX2 big-endian
conversion to the baked primitive array. Use the active mask rows
with the 192-texel allocation stride when counting changes.

E13765's first fragment constant is `(0.942, 1.0, 0.39, 3.5)`.
The RGB matches `VolumeMaskColor` entries in the baked palette.
This supports a palette-derived color. The source of the fourth
component and the CPU upload path are open. The next live-renderer
checkpoint is the R16F mask update and the RGBA16F effect draw,
each compared against its own capture boundary.

### Mask shader and causal A/B

At E10819 the BC4 sampler view maps every component of sampled RGBA
to red. The fragment samples scalar `m` at `Sem8.xy` and computes
`src.r = Sem7.r × lerp(Sem1.r, Sem0.r, m²)` and
`src.a = Sem4.x × saturate(Sem7.a × m × (Sem0.a − Sem1.a))`.
It discards at the alpha threshold. The R16F color blend applies
`mask = src.r × src.a + mask × (1 − src.a)`. Instrumented RenderDoc
replay confirmed the red-to-alpha mapping at all 35 sampled passing
fragments across 10 coordinates. Native blend writebacks differ from
the CPU calculation by 0.000370 mean and 0.000913 maximum absolute
R16F value for those fragments.

The [mask A/B replay script](../../tools/research/renderdoc_haze_mask_ab.py)
uses the same environment as the overlay exporter and needs
`spirv-cross` on PATH. It makes the mask shader discard in an offline
replay. The original and bypass runs have identical pre-mask,
pre-E13765 effect, and pre-E13778 HDR buffers. After the mask sequence,
the bypass result equals the pre-mask buffer. The haze effect differs
at 188,503 half-resolution texels, and the E13778 HDR result differs
at 615,466 full-resolution pixels.

With the mask draws active, mean linear HDR RGB relative to the bypass
changes by `(−0.0707, −0.0740, −0.0271)` on the far-slope box and by
`(−0.0941, −0.0965, −0.0304)` on sky. The localized haze draw adds
color while the preceding mask draws restrain its strength.

A separate forward-mode Vulkan screenshot at 640×360 verified that
materials still render when shared pre-shading is unavailable.
