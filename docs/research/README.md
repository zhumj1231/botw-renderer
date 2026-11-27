# BotW rendering research

The goal is to explain how an outdoor or indoor frame is produced, reproduce
the key rendering techniques in the independent renderer, and account for
their cost on constrained hardware. The active reference is Wii U JP v208.
Treat Switch as a separate version and carry claims across platforms only
after checking equivalent evidence.

Start with the [source catalog](sources.md), then check the
[original Wii U capture checklist](wiiu-deferred-shading.md#original-wii-u-capture-checklist)
and the module notes below. The repository [handoff](../../HANDOFF.md)
records the current checkpoint and how to run the renderer.

## Before researching a module

1. Read the module notes and search for work already done. Search by visible
   effect, native symbols, shader options and resource names. Follow citations
   to the authors' original work. Search across languages and publication dates,
   including Japanese conference archives, developer interviews, implementation
   and decompilation repositories, shader and frame analyses, format tools,
   and graphics papers. Record the search terms and checked date for findings
   that matter to the module.
2. Adopt a prior result within its evidence scope. Record the platform, game
   region and revision, and which scene, shader variant or resource it covers.
   Track emulator and graphics-pack behavior separately. Mirrors and reports
   derived from the same capture or translation share one evidence source.
3. Test the most consequential unknown. Cross-check a capture against the
   shader or CPU path and the resource bytes wherever possible. Match passes
   by attachments, bindings, state and shader identity across frames. Event IDs
   identify draws within a particular capture.
4. Update the module note and its checklist row at the milestone. Keep source
   links, evidence locations, commands, results, and unresolved cases in Git.
   Keep raw captures, extracted assets, shaders, and dumps local. Record enough
   metadata to identify or regenerate evidence on a second machine.

## How to record a claim

| Evidence state | What to record |
| --- | --- |
| Developer stated | The source and timestamp or section, what the author actually describes, and the design context. |
| Observed in a game capture | Game version, capture ID and EID, active graphics packs, host backend, relevant viewport, resource formats and state. Distinguish Cemu-translated behavior from a hardware capture. |
| Traced in code or data | Executable or archive revision, symbols or offsets, shader variant or program hash, bound resource, and the operations that support the result. State which branches or inputs were checked. |
| Verified in the independent renderer | Commit, reproduction command, scene and adapter, GPU output or readback, comparison method and measured discrepancy. |
| Inference or open question | The evidence so far, alternative explanations that still fit, and the smallest experiment that can distinguish them. |

For each implemented effect, trace the inputs from CPU state and assets
through the shader variant, render passes, attachment and channel formats,
blend and depth state, and downstream consumers. Record equations with
coordinate spaces and units. For a representative material, identify other
materials that exercise the same shader variant before generalizing.

Separate observed design intent from engineering inference. Explain a
tradeoff using a stated design goal, a measured consequence, or a clearly
labeled inference. For performance, report per-pass CPU/GPU time, work
dimensions or samples, logical storage, and bandwidth measurements where
available. A half-width, half-height pass covers one quarter of the pixel
positions. Whole-frame savings and hardware traffic require measurement.

## Module index

| Module | Evidence and implementation notes | Next discriminating check |
| --- | --- | --- |
| Frame pipeline and reconstruction | [Deferred frame chain](wiiu-deferred-shading.md) | Match the four-tap guide and downstream composition across depth breaks and material classes. |
| Dynamic and baked shadows | [Field shadows](wiiu-field-shading.md#ps-128-shadows-and-ssao--expand) and [foliage and character Expand visibility](wiiu-character-shading.md#expand-visibility-for-foliage-and-opaque-characters) | Read live cascade split, transform, sampler and fade values for the same view in the original. Compare receiver classes separately. |
| SSAO | [Field Expand](wiiu-field-shading.md#ps-128-shadows-and-ssao--expand) and [scene configuration](wiiu-deferred-shading.md#ssao-scene-configuration) | Compare Expand blue and reconstructed ambient against contact and open-ground pixels, including silhouettes. |
| Indirect light and exposure | [Outdoor lighting](wiiu-deferred-shading.md#global-outdoor-lighting) and [LightAnalyzer](wiiu-character-shading.md#the-light-of-the-environment-lightanalyzer--gsys_user4--sem2) | The [live camera trace](wiiu-deferred-shading.md#live-geometry-capture-camera) covers all six directions and compares mirror-parallax and capture centers. Trace long-displacement recentering and smaller mip target initialization next. |
| Atmosphere and day/night | [Sky resources](wiiu-sky-resources.md) | Align the original clear day/night inputs and read sky, fog and transmission before tone mapping. |
| Weather and precipitation | [Weather](weather.md), [Effect archives](eft-shaders.md) and [Effect runtime](eft-runtime.md) | Compare clear/rain at matched terrain samples. The [live snow draw](eft-shaders.md#live-snowfall-in-frame-123839) confirms additive HDR flakes with depth testing. Match the draw to its emitter settings. |
| Specific visual effects (item 7) | [Wind-blown ground haze and airborne debris](eft-shaders.md#wind-blown-ground-haze-and-airborne-debris) | Identify the emitters, shader inputs and render states in a before/during/after gust capture. |
| Materials and PBR | [Material variants](wiiu-material-variants.md), [shader containers](wiiu-shader-containers.md), [live terrain draw](wiiu-field-shading.md#live-terrain-material-sample-in-frame-3830) and [character shading](wiiu-character-shading.md) | Resolve the terrain blend weight and original material key. Trace a leaf, skin and equipment from texture channels through shader selection, G-buffer encoding and HDR output. |
| Translucency and post-processing | [Water variants](wiiu-water-variants.md), [model water and glass](model-water-glass.md) and [post-processing materials](wiiu-postfx-materials.md) | Confirm the second pre-shading layer and readers at a water or glass depth discontinuity. |

## Baseline for repeated comparison

The shared frame reference is Wii U JP v208, Cemu 1.26.2f Vulkan, RenderDoc
1.38, zero active graphics packs, frame 3830. The capture records 1280×720
geometry and 640×360 pre-shading. The observed three color attachments use
10 logical bytes per pixel, or 14 with camera D32. That describes attachment
storage. The reference has no controlled performance comparison. Use the
[event and format table](wiiu-deferred-shading.md#hateno-terrain-buffer-diagnostic)
and the [Cemu session notes](wiiu-cemu-sessions.md) to locate corresponding
passes in new captures.

For a paired sample, record scene and camera, game clock and weather, run
duration or animation state, viewport, graphics settings, and asset revision.
Use the same capture for before/after comparisons when dynamic inputs would
otherwise drift. Keep independent renderer validation distinct from
original-game validation and native hardware performance.
