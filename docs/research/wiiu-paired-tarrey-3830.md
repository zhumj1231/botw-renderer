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
