# BOTW renderer research handoff

Work in the **botw-renderer** repository on `fix_vulkan_object_shader`. The
goal is to reproduce the Wii U rendering pipeline and image as closely as the
evidence allows, measure the cost of the resulting passes, and keep the
research reproducible. Use Hateno as the first regression scene. The 3D map
project and the Unity port are downstream work. Read [CLAUDE.md](CLAUDE.md)
for repository rules and [README.md](README.md) for general setup.

## Start on a second machine

Clone the branch and build it with the locked dependencies.

```sh
git clone --branch fix_vulkan_object_shader https://github.com/zhumj1231/botw-renderer.git
cd botw-renderer
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

The renderer only needs the baked assets. Cemu is needed only for new
original-game captures. Keep the dump, asset bundles, local configuration,
shader dumps and RenderDoc captures outside tracked files.

## Reproduce the verified renderer paths

The previous machine ran all three paths with Vulkan on Windows and an
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
- Independent screenshots are for inspection. Cloud shadows can move
  between runs even with a fixed time of day and weather. For a numerical
  A/B while both draws are present, read the same HDR attachment immediately
  before and after `compose` and filter by full-resolution D16 class 4.

The original reference is Wii U JPN v208 in Cemu 1.26.2f with Vulkan and
zero graphics packs, frame 3830. Keep its observations separate from the
renderer A/B. The checked-in [deferred pipeline notes](docs/research/wiiu-deferred-shading.md)
contain the original event map, attachment formats, measurements and
approximations. The [field shader notes](docs/research/wiiu-field-shading.md)
trace the lighting terms. The raw Wii U replay and the previous renderer
replay are local evidence on the previous machine. An existing capture can
be privately transferred if needed, or a fresh zero-pack frame can be used
with the event map as a reference for matching pipeline state.

## Continue here

1. **Remove the duplicate forward draw from the experimental main view.**
   The current path draws forward terrain first and overwrites its class-4
   pixels during HDR composition. Gate the removal on readiness of the
   custom geometry and lighting resources. Preserve depth-prepass, shadow
   and cubemap behavior. Check startup, camera jumps and tile streaming
   for missing terrain before timing the new path. The default forward
   mode remains the reference and fallback during loading.
2. **Measure the terrain path after the duplicate draw is gone.** Fix
   renderer backend, adapter, resolution, Hateno camera and scene settings.
   Use `--profile` and GPU pass timings, with a warm scene and a repeatable
   camera run. Record the depth prepass, geometry, both class passes,
   pre-shading and HDR composition separately. Compare frame cost on the
   same machine. The existing captures do not establish a performance win.
3. **Match the original guided reconstruction.** Start at
   `guide_texel` in
   [lighting.wgsl](crates/render/src/terrain_gbuffer/lighting.wgsl) and
   compare it to the original EID 12855. Follow the four depth and R8
   encoded-normal samples through to the final sample coordinate and check
   steep slopes, silhouettes and class boundaries. The current four-neighbor
   ranking, Bevy depth reconstruction and mapped-normal shadow bias are
   documented approximations (SI-GBUF-03/04). Leave lighting coefficients
   in place while resolving these structural differences.
4. **Extend coverage after terrain is verified.** Add the second shared
   pre-shading layer used by water and extend D16 class coverage to the
   other materials. Treat the exact cube update cadence, probe placement
   and native shadow scheduling as research questions until captures
   establish them.

Start code review in [terrain_gbuffer.rs](crates/render/src/terrain_gbuffer.rs)
for scheduling, target allocation and D16 classification,
[geometry.rs](crates/render/src/terrain_gbuffer/geometry.rs) for terrain MRTs,
and [lighting.rs](crates/render/src/terrain_gbuffer/lighting.rs) for the
half/full-resolution pass bindings. Shared lighting math lives in
[deferred_light.wgsl](crates/render/src/deferred_light.wgsl).
