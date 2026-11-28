# BotW rendering source catalog

Initial survey checked on 2026-11-27. For use and evidence rules, see the
[research index](README.md). A source can be useful for one question and
insufficient for another. Record the inspected revision or section when
citing a result in a module note.

## Nintendo and conference material

- **CEDEC 2017, 3D graphics art and implementation** (Japanese, BotW,
  public abstract). Takuhiro Dohta and Satoru Takizawa describe image-making
  in near, middle and far layers and in time and space. Use the abstract
  for stated art and engineering intent. The official listing says
  「資料公開：不可」. A search for the official complete slides need not be
  repeated without new evidence of publication. [Official session](https://cedec.cesa.or.jp/2017/session/VA/s58eed7182c81b.html)
- **CEDEC 2017, visual effects and data-driven implementation** (Japanese,
  BotW, public abstract). The presenters explain the challenge of combining
  physics and chemistry interactions with dynamic weather and a large
  quantity of authored effects. The listing also says 「資料公開：不可」.
  Use this for the production problem and goals. Trace effect resources and
  runtime passes separately. [Official session](https://cedec.cesa.or.jp/2017/session/VA/s58eed8209ffce.html)
- **CEDEC 2017, game engine and production** (Japanese, BotW, public
  abstract). The artist and engineer perspectives span prototyping and
  production. This is a source for the engine workflow and the reason for
  investing in tools. [Official session](https://cedec.cesa.or.jp/2017/session/PRD/s58eecd0d9e467.html)
- **GDC 2017, Change and Constant** (public talk). Use the Nintendo
  presenters' discussion for gameplay, art and development context. Cite
  the speaker and time when drawing a specific conclusion from the video.
  [GDC Vault](https://www.gdcvault.com/play/1024562/Change-and-Constant-Breaking-Conventions-with-The-Legend-of-Zelda-Breath-of-the-Wild)
- **The Making of BotW** (2017, developer interviews). The open-air episode
  covers goals around exploration and scale. The Japanese and English
  releases of the episode are two access points to the same source.
  [Nintendo interview series](https://www.nintendo.com/en-gb/News/2017/March/Go-behind-the-scenes-with-the-making-of-The-Legend-of-Zelda-Breath-of-the-Wild-1206592.html) · [Japanese open-air episode](https://www.youtube.com/watch?v=ZwqFNZfSvCA)

The public abstracts establish goals. Shader equations, pass order and
attachment formats need a capture or a code and resource trace.

## Code, captures and format research

- **Cemu** (Wii U, emulator source). Follow GX2/Latte state and shader
  translation into Vulkan before attributing a translated shader detail to
  the game. The frame-3830 notes used the early public 2.0 source as a
  translation cross-check for a 1.26.2f capture. Recheck the relevant code
  when Cemu version or backend changes. [Cemu repository](https://github.com/cemu-project/Cemu) · [Local session notes](wiiu-cemu-sessions.md)
- **RenderDoc** (capture and replay tool). Use the actual replay version and
  distinguish viewport sizes from host allocations and valid layer ranges
  from full view ranges. Check the output format and color space before
  comparing pixels. [RenderDoc repository](https://github.com/baldurk/renderdoc)
- **zeldaret/botw** (Switch 1.5.0, in-progress matching decompilation). Useful
  for recovering names, tracing CPU state and finding matching concepts to
  test. Re-establish control flow and constants in Wii U v208 before using
  them as Wii U evidence. [Project repository](https://github.com/zeldaret/botw)
- **oead** (format implementation). Use alongside the local parsers to
  cross-check SARC, Yaz0, BYML and supported AAMP versions. Record archive
  layering and decoded field offsets for a claim about a rendering
  parameter. [Maintainer repository](https://github.com/zeldamods/oead)
- **Switch Toolbox** (Wii U and Switch, format tooling). The maintainer's
  porting guide identifies material and shader differences even when BFRES
  geometry or textures can be transferred. Compare platform-specific material
  bindings before equating same-name resources. [Pinned maintainer guide](https://github.com/KillzXGaming/Switch-Toolbox/wiki/Porting-Wii-U-Switch-mods/244f7be2e8a28857a9db0e45ee288add4bdc1f38)
- **Frame 3830 and Wii U resources** (local original-game evidence and static
  studies). The documented zero-pack replay provides attachment topology,
  the class-depth passes and half-resolution shading. The archive study
  records container and shader matching, and the material study traces the
  key and variant selection. [Frame and formats](wiiu-deferred-shading.md#hateno-terrain-buffer-diagnostic) · [Shader containers](wiiu-shader-containers.md) · [Material variants](wiiu-material-variants.md)

## Papers and comparison

- **Walter, Marschner, Li and Torrance, 2007**. Use the microfacet
  distribution, masking and transmission derivation as a reference when
  naming terms and testing water or equipment shaders. Match the recovered
  program's remapping, terms and implementation before identifying a full
  BRDF or BTDF. [Authors' paper](https://www.cs.cornell.edu/~srm/publications/EGSR07-btdf.pdf)

## Chinese references and cross-checks

The supplied local MHTML files were read for the public article bodies and
source links. The catalog records the linked evidence. The saved pages stay
local.

| Source and scope | Cross-check or test |
| --- | --- |
| GuardHei's Switch **Tears of the Kingdom** capture study, using Ryujinx, Vulkan and RenderDoc. The article and Q&A copy have identical substantive text after normalizing whitespace and removing the extra heading. Count them as one study. [Article](https://zhuanlan.zhihu.com/p/674148942) · [Q&A copy](https://www.zhihu.com/question/626510155/answer/3344018036) | The author's observations about reduced-resolution lighting and depth-based class dispatch suggest comparison points. Wii U BotW frame 3830 independently shows half-width/half-height pre-shading, RGB-encoded normals in the sampled path, and three 720×720 shadow viewports. The TotK report describes RG normals with AO in blue and a four-cascade configuration. Use direct BotW evidence before carrying over compute material binning, attachment semantics, or pixel ratios. |
| 元素界王-徐文轩's **BotW rendering practice notes**, illustrated by the author's own reproduction. The article describes observations about character response in shadow, metal, rim light and equipment effects. It gives no build or draw identity for those observations. [Author's article](https://zhuanlan.zhihu.com/p/580141974) | Use these as tests for material variants. Compare skin and metal with the same view and light direction inside and outside a cast shadow, then trace their constants and HDR consumers. Ramp and Blinn-style terms in the reproduction do not establish their use in every native character program. |
| NGA performance discussion links both a Zhihu answer and BrainChild's original ResetEra analysis. Treat the linked claims as having the original work's provenance. [NGA discussion](https://bbs.nga.cn/read.php?tid=40316220) · [Linked answer](https://www.zhihu.com/question/532613583/answer/2490512073) | The description of a hybrid deferred/light-prepass chain is consistent with the Wii U capture's material data, reduced-resolution lighting and full-resolution composition. The discussion's Switch performance explanations still need hardware measurements. |
| BrainChild's **BotW technical analysis** and the Bilibili Chinese translation by m4k1s3. The original discussion uses observed gameplay and symbols to propose mechanisms, including environment color transfer. [Original thread](https://www.resetera.com/threads/zelda-breath-of-the-wild-the-technical-analysis.8197/) · [Chinese translation](https://www.bilibili.com/opus/283686760320058651) | Wii U frame 3830 shows writes to individual cubemap faces and subsequent samples of the processed cube. That does not settle probe placement, SH encoding, LOD streaming, or multi-bounce transfer. Test frame-to-frame cube updates with separate camera and player movements. |

### Other implementations to compare

For the Resident haze follow-up, the supplied articles give two comparison
points. GuardHei's [TotK frame study](https://zhuanlan.zhihu.com/p/674148942)
shows particle masks and several reduced-resolution atmosphere effects in a
Switch frame. Its ground fog raymarch is a separate comparison from the
Wii U BotW mesh overlay. [BrainChild's analysis](https://www.resetera.com/threads/zelda-breath-of-the-wild-the-technical-analysis.8197/)
and its [Chinese translation](https://www.bilibili.com/opus/283686760320058651)
describe distant-fog color and appearance from gameplay. The Nintendo
[CEDEC layer talk](https://cedec.cesa.or.jp/2017/session/VA/s58eed7182c81b.html)
establishes the design goal of layering space and time. These sources
give no emitter or uniform identification for E13765. The relevant
implementation evidence is in the
[Wii U frame checkpoint](wiiu-paired-tarrey-3830.md#camera-relative-mask-emitter).

- **BotW Switch graphics notes** from the `nxnative_patch` project document
  an NVN translation path and shader-container observations. Use the pinned
  notes to choose equivalent Switch resources or draws to inspect. Treat the
  host implementation and its limitations as context when comparing with
  Wii U and Cemu. [Pinned graphics notes](https://git.dukantic.fr/dukantic/nxnative_patch/src/commit/9c1e449470421d6af8d09f0edd9c2b3447dd80ab/docs/agents/04-graphics.md)

## First follow-up search

Materials are the next open module in the
[original capture checklist](wiiu-deferred-shading.md#original-wii-u-capture-checklist).
The initial survey used `uking_mat`, `uking_terrain`, `BFRES`, `GX2`,
`Latte`, and the Japanese conference title and terms `3Dグラフィックス`
and `エフェクト`. Start by reusing the existing BFRES material-to-option-key
mapping. Trace one terrain patch, leaf, skin material and piece of equipment
through the texture slots and channel decoding, shader selection, G-buffer
output and final lighting. Read the resulting per-material evidence before
generalizing a feature to the whole frame.
