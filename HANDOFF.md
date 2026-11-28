# BOTW renderer research handoff

Work in the **botw-renderer** repository on `rendering_deep_dive`. The
goal is to reproduce the Wii U rendering pipeline and image as closely as the
evidence allows, measure the cost of the resulting passes, and keep the
research reproducible. Use Hateno as the first regression scene. The 3D map
project and the Unity port are downstream work. Read [CLAUDE.md](CLAUDE.md)
for repository rules and [README.md](README.md) for general setup. Begin
each module with the [research index](docs/research/README.md) and its
[source catalog](docs/research/sources.md).

## Start on a second machine

Clone the branch and build it with the locked dependencies.

```sh
git clone --branch rendering_deep_dive https://github.com/zhumj1231/botw-renderer.git
cd botw-renderer
git rev-parse HEAD
cargo fetch --locked
cargo build --release --locked -p render
cargo test --release --locked -p render
```

The branch uses Bevy 0.19.1 from `Cargo.lock`. See the README for Rust and
platform toolchain requirements. Windows needs the MSVC C++ toolchain and
Windows SDK. Use a graphical session with compatible GPU drivers for visual
validation.

If the user has the existing Hateno baked assets on the second machine, use
the extracted asset directory directly via `--assets`. Point the flag at
the directory containing `places.ron`. Otherwise, use the user's own
extracted Wii U dump and the README's `renderer.toml` configuration for base
and v208 update layers, plus DLC if available. A regional bake is enough to
start.

```sh
cargo bake --region hateno --radius 1000 --out assets-hateno
```

On an existing bake, check for `light/solar_irradiation.ktx2`. The
class-8 far-shadow comparison needs that resource. If it is missing, use
the user's original base and v208 update roots and re-bake only lighting.

```powershell
cargo bake --game $base --game $update --only light --out $assets
```

The renderer only needs the baked assets. Cemu is needed only for new
original-game captures. Keep the dump, asset bundles, local configuration,
shader dumps and RenderDoc captures outside tracked files.

## Reproduce the verified renderer paths

The earlier terrain checkpoint ran all three paths with Vulkan on Windows and an
RTX 4080 Laptop GPU. On Windows PowerShell, select the same backend with
`$env:WGPU_BACKEND = 'vulkan'`. Create the local `captures` directory before
running the screenshot commands below.

```sh
cargo render --assets assets-hateno --place hateno --time 10:00 --weather bluesky --screenshot captures/hateno-forward.png
cargo render --assets assets-hateno --place hateno --time 10:00 --weather bluesky --terrain-gbuffer --screenshot captures/hateno-gbuffer.png
cargo render --assets assets-hateno --place hateno --time 10:00 --weather bluesky --terrain-deferred --screenshot captures/hateno-deferred.png
```

- `b8de2d1` introduced direct terrain writes to RT0, RT1 and RT3. The
  diagnostic shows albedo, encoded normal, half-resolution D16 classification
  and full-resolution `Equal` coverage.
- `dc95369` added the opt-in terrain lighting path. It reuses the current
  field and haze equations to pre-shade at 800×450 for a 1600×900 view.
  Full-resolution class 4 composition writes to the view's HDR target
  before transparency. The test run recorded 177 passed, 4 ignored.
- In the previous renderer capture, the shader entry points `preshade` and
  `compose` ran at EIDs 45245 and 45270. The preceding draw at EID 45256
  still had the forward terrain in HDR. Across 778,282 class-4 pixels, the
  mean absolute difference was 0.0118 per linear RGB channel before tone
  mapping. This measures the renderer's own migration. Treat new capture
  EIDs and exact pixel counts as frame-specific.
- At `4333fb9`, the main view began skipping redundant forward terrain
  once deferred geometry and lighting are ready. It still has a forward
  loading fallback. The older same-frame comparison above was recorded
  while both draws were present.
- At `aebec37`, separate foliage and opaque character filters feed
  visibility into Expand and LightAnalyzer. Foliage was checked with a
  Vulkan replay. The Link Vulkan/RenderDoc run on the GTX 1660 SUPER did
  not produce a replay. Character verification used DX12 staging readback.
  See the [case settings and limits](docs/research/wiiu-character-shading.md#expand-visibility-for-foliage-and-opaque-characters).
- Independent screenshots are for inspection. Cloud shadows can move
  between runs even with a fixed time of day and weather. Use the
  [research workflow](docs/research/README.md#baseline-for-repeated-comparison)
  when making a numerical comparison.

The original reference is Wii U JPN v208 in Cemu 1.26.2f with Vulkan and
zero graphics packs, frame 3830 at 17:40 in Tarrey Town. The
[paired scene recipe](docs/research/wiiu-paired-tarrey-3830.md) records
the camera and HDR extraction. Keep its observations separate from the
renderer A/B. The checked-in [deferred pipeline notes](docs/research/wiiu-deferred-shading.md)
contain the original event map, attachment formats, measurements and
approximations. The [field shader notes](docs/research/wiiu-field-shading.md)
trace the lighting terms. The raw Wii U replay and the previous renderer
replay are local evidence on the previous machine. An existing capture can
be privately transferred if needed, or a fresh zero-pack frame can be used
with the event map as a reference for matching pipeline state.

## Continue here

The first matched visual pair is in
[the Tarrey Town checkpoint](docs/research/wiiu-paired-tarrey-3830.md).
Use the five documented map flags for the post-construction object
snapshot. The far-slope shadow fix and the cascade split A/B are
verified in the paired camera. The object material now consumes the
pre-shaded sun term. Frame 3830 establishes the E13765 geometry draw,
its 160×90 mask input, and the depth-guided E13778 HDR composite. The
full-frame CPU comparison and E13765 A/B are verified. The renderer-side
HDR composite is also verified on Vulkan in `BOTW_OVERLAY_REPLAY` mode
with the four original inputs. The 160×90 mask draws match
`VolumeMaskAdd01test/Haze_Far` and their shader swizzle, blend and
downstream contribution are verified. E13765's mesh matches
`Haze_Far_simple_new`. Replacement fragment shaders reproduce
both captured targets exactly. Use `BOTW_HAZE_EXPORT` with the
mask A/B script to get all 39 indexed post-VS streams and compare
independent Vulkan raster passes against the R16F and RGBA16F
snapshots, then drive the passes with live emitter state.
The same-input display curve is verified at the sampled points.
The weather replays are additional validation scenes.

1. **Close the outdoor clear-day baseline.** Use frame 3830 for the
   [terrain shadow and SSAO readbacks](docs/research/wiiu-deferred-shading.md#clear-day-terrain-expand-at-eid-9949).
   Use the same capture to check the AO silhouette and the reconstructed
   ambient output. Compare `guide_texel` to original EID 12855 across the
   same material boundaries.
2. **Measure outdoor ambient and atmosphere.** The clear-day
   [LightAnalyzer readback](docs/research/wiiu-character-shading.md#clear-day-lightanalyzer-readback-in-frame-3830)
   and the [clear-day atmosphere samples](docs/research/wiiu-deferred-shading.md#clear-day-atmosphere-in-frame-3830)
   are verified. The [Tarrey Town day/night pair](docs/research/wiiu-deferred-shading.md#fixed-camera-clear-day-and-night-in-tarrey-town)
   provides matched camera, terrain fog, LightAnalyzer and display samples.
   Capture matched rainy daylight and Hebra snowfall for the weather checks.
3. **Trace remaining material paths.** Frame 3830 establishes the
   [terrain overlap weights](docs/research/wiiu-field-shading.md#live-terrain-material-sample-in-frame-3830)
   and the [foliage route](docs/research/wiiu-character-shading.md#live-class-7-foliage-in-frame-3830).
   Link skin, cloth and equipment are paused at the user's request.
   Match the native terrain permutation and verify prior-buffer access
   before scheduling the independent blend. Verify the second shared
   pre-shading layer used by water.

Start code review in [terrain_gbuffer.rs](crates/render/src/terrain_gbuffer.rs)
for scheduling, target allocation and D16 classification,
[geometry.rs](crates/render/src/terrain_gbuffer/geometry.rs) for terrain MRTs,
and [lighting.rs](crates/render/src/terrain_gbuffer/lighting.rs) for the
half/full-resolution pass bindings. Shared lighting math lives in
[deferred_light.wgsl](crates/render/src/deferred_light.wgsl).
