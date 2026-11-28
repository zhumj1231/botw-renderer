"""Check that a real particle capture reuses the reduced cloud target."""
import json
import os
import struct
import sys
import traceback
from pathlib import Path

import renderdoc as rd

out = Path(os.environ["BOTW_FOG_OUT"])
capture = out / os.environ.get("BOTW_SHARED_REDUCED_CAPTURE", "renderer-paired-4188-shared-reduced_capture.rdc")
stage = rd.ShaderStage.Pixel
sub = rd.Subresource(0, 0, 0)


def actions(items):
    for item in items:
        yield item
        yield from actions(item.children)


def check(replay):
    textures = {t.resourceId: t for t in replay.GetTextures()}
    draws = list(actions(replay.GetRootActions()))
    effect = None
    for draw in reversed(draws):
        if not draw.flags & rd.ActionFlags.Drawcall or not draw.outputs:
            continue
        desc = textures.get(draw.outputs[0])
        if not desc or (desc.width, desc.height, desc.format.Name()) != (640, 360, "R16G16B16A16_FLOAT"):
            continue
        replay.SetFrameEvent(draw.eventId, False)
        shader = replay.GetPipelineState().GetShaderReflection(stage)
        if shader and shader.entryPoint == "live_haze_effect":
            effect = draw
            break
    assert effect, "Live indexed haze pass wasn't found"
    replay.SetFrameEvent(effect.eventId, True)
    mask_id = next(access.descriptor.resource
                   for access in replay.GetPipelineState().GetReadOnlyResources(stage)
                   if access.descriptor.resource in textures
                   and textures[access.descriptor.resource].format.Name() == "R16_FLOAT")
    mask_writes = {u.eventId for u in replay.GetUsage(mask_id)
                   if u.usage == rd.ResourceUsage.ColorTarget and u.eventId < effect.eventId}
    assert mask_writes, "The effect doesn't read a live mask"
    mask_draws = []
    for x, y in ((79, 5), (80, 5), (79, 6), (80, 6)):
        history = [m for m in replay.PixelHistory(mask_id, x, y, sub, rd.CompType.Typeless)
                   if m.eventId in mask_writes and m.Passed()
                   and m.preMod.col.floatValue[0] != m.postMod.col.floatValue[0]]
        mask_draws.append({
            "xy": [x, y],
            "blended_fragments": len(history),
            "value": history[-1].postMod.col.floatValue[0] if history else None,
        })
    rid = effect.outputs[0]
    usage = replay.GetUsage(rid)
    writers = [u.eventId for u in usage if u.usage == rd.ResourceUsage.ColorTarget and u.eventId < effect.eventId]
    readers = [u.eventId for u in usage if u.usage == rd.ResourceUsage.PS_Resource and u.eventId > effect.eventId]
    assert writers and readers, "The effect target has no earlier writer or downstream consumer"
    by_id = {draw.eventId: draw for draw in draws}
    active_particles = 0
    for event in sorted(mask_writes):
        replay.SetFrameEvent(event, True)
        draw = by_id[event]
        assert draw.numIndices % 144 == 0
        # The Haze_Far primitive has 33 vertices and 144 indices.
        mesh = replay.GetPostVSData(0, 0, rd.MeshDataStage.VSOut)
        count = draw.numIndices // 144
        stride = mesh.vertexByteStride
        vertices = bytes(replay.GetBufferData(mesh.vertexResourceId, mesh.vertexByteOffset,
                                              count * 33 * stride))
        assert len(vertices) == count * 33 * stride
        for i in range(count):
            clip = struct.unpack_from("<4f", vertices, 33 * i * stride)
            active_particles += clip != (0.0, 0.0, 2.0, 1.0)
    cloud = by_id[writers[-1]]
    compose = by_id[readers[0]]
    assert cloud.numIndices == 3 and effect.numIndices > 3
    replay.SetFrameEvent(compose.eventId, True)
    p = replay.GetPipelineState()
    blend = p.GetColorBlends()[0].colorBlend
    actual_blend = (str(blend.source), str(blend.destination))
    assert actual_blend == ("BlendMultiplier.One", "BlendMultiplier.SrcAlpha"), actual_blend
    hdr = p.GetOutputTargets()[0].resource
    assert (textures[hdr].width, textures[hdr].height) == (1280, 720)
    assert len([event for event in readers if event != compose.eventId]) == 0, readers

    def pixel(resource, event, x, y):
        replay.SetFrameEvent(event, True)
        desc = textures[resource]
        assert desc.format.Name() == "R16G16B16A16_FLOAT"
        pixels = bytes(replay.GetTextureData(resource, sub))
        return struct.unpack_from("<4e", pixels, (y * desc.width + x) * 8)

    replay.SetFrameEvent(compose.eventId, True)
    at_compose = next(
        mod for mod in replay.PixelHistory(hdr, 640, 45, sub, rd.CompType.Typeless)
        if mod.eventId == compose.eventId and mod.Passed()
    )
    # The draw contributes to main HDR before bloom and display.
    return {
        "events": {"reduced_clouds": cloud.eventId, "live_haze": effect.eventId, "full_composite": compose.eventId},
        "effect_indices": effect.numIndices,
        "active_mask_particles": active_particles,
        "mask_indices": sum(by_id[e].numIndices for e in mask_writes),
        "mask_footprint": mask_draws,
        "reduced_buffer": {"before_haze": pixel(rid, cloud.eventId, 320, 22),
                           "after_haze": pixel(rid, effect.eventId, 320, 22)},
        "full_hdr": {"before_composite": list(at_compose.preMod.col.floatValue),
                     "after_composite": list(at_compose.postMod.col.floatValue)},
    }


with (out / "shared-reduced-validation.log").open("w", buffering=1) as log:
    sys.stdout = sys.stderr = log
    file = rd.OpenCaptureFile()
    replay = None
    try:
        assert file.OpenFile(str(capture), "", None) == rd.ResultCode.Succeeded
        status, replay = file.OpenCapture(rd.ReplayOptions(), None)
        assert status == rd.ResultCode.Succeeded, status
        result = check(replay)
        print(json.dumps(result, indent=2))
    except Exception:
        traceback.print_exc()
        sys.exit(1)
    finally:
        if replay:
            replay.Shutdown()
        file.Shutdown()
    sys.exit(0)
