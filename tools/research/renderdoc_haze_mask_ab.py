"""Suppress the source-alpha mask draws in the Wii U frame 3830 replay."""

import json
import math
import os
from pathlib import Path
import struct
import subprocess
import sys
import traceback

import renderdoc as rd


READBACKS = {
    "mask_base": (10744, 12571),
    "mask_final": (10819, 12571),
    "effect_base": (13764, 20013),
    "effect_haze": (13765, 20013),
    "hdr_base": (13777, 12282),
    "hdr_haze": (13778, 12282),
}


def changed(before, after, stride):
    return sum(
        before[i:i + stride] != after[i:i + stride]
        for i in range(0, len(before), stride)
    )


def channel_lut(bits):
    return [
        math.ldexp(
            int(bool(i >> bits)) + (i & ((1 << bits) - 1)) / float(1 << bits),
            max(1, i >> bits) - 15,
        )
        for i in range(31 << bits)
    ]


def hdr_deltas(baseline, bypass, full_depth):
    t6, t5 = channel_lut(6), channel_lut(5)

    def rgb(word):
        return t6[word & 2047], t6[(word >> 11) & 2047], t5[word >> 22]

    captures = (struct.iter_unpack("<I", image) for image in (baseline, bypass))
    totals = {region: [0, 0, 0.0, 0.0, 0.0] for region in ("all", "sky", "far_slope_box")}
    for i, ((a,), (b,), (z,)) in enumerate(zip(*captures, struct.iter_unpack("<H", full_depth))):
        x, y = i % 1280, i // 1280
        regions = ["all"]
        if z == 0x3C00:
            regions.append("sky")
        if 848 <= x <= 894 and 64 <= y <= 83 and z < 0x3C00:
            regions.append("far_slope_box")
        delta = [u - v for u, v in zip(rgb(a), rgb(b))]
        for region in regions:
            row = totals[region]
            row[0] += 1
            row[1] += a != b
            for j, value in enumerate(delta):
                row[j + 2] += value
    return {
        region: {"pixels": n, "changed": count, "delta_rgb": [r / n, g / n, b / n]}
        for region, (n, count, r, g, b) in totals.items()
    }


def compare(replay):
    textures = {str(t.resourceId): t for t in replay.GetTextures()}

    def read(event, address):
        replay.SetFrameEvent(event, True)
        tex = textures["ResourceId::%d" % address]
        return bytes(replay.GetTextureData(tex.resourceId, rd.Subresource(0, 0, 0)))

    baseline = {key: read(*point) for key, point in READBACKS.items()}
    full_depth = read(13777, 12870)
    replay.SetFrameEvent(10819, True)
    p = replay.GetPipelineState()
    stage = rd.ShaderStage.Pixel
    shader = p.GetShader(stage)
    spirv = bytes(p.GetShaderReflection(stage).rawBytes)
    source = subprocess.check_output(
        ["spirv-cross", "-", "--vulkan-semantics"], input=spirv
    ).decode("utf-8")
    final_output = "passPixelColor0 = vec4(R1f.x, R1f.y, R1f.z, R1f.w);"
    assert source.count(final_output) == 1
    bypass_source = source.replace(final_output, "discard;").encode("utf-8")
    replacement, errors = replay.BuildTargetShader(
        "main", rd.ShaderEncoding.GLSL, bypass_source, rd.ShaderCompileFlags(), stage
    )
    assert int(replacement), errors
    replay.ReplaceResource(shader, replacement)
    try:
        bypass = {key: read(*point) for key, point in READBACKS.items()}
    finally:
        replay.RemoveReplacement(shader)
        replay.FreeTargetResource(replacement)

    for key in ("mask_base", "effect_base", "hdr_base"):
        assert baseline[key] == bypass[key], key
    assert bypass["mask_final"] == baseline["mask_base"]
    return {
        "changed_mask_texels": changed(baseline["mask_final"], bypass["mask_final"], 2),
        "changed_effect_texels": changed(baseline["effect_haze"], bypass["effect_haze"], 8),
        "hdr": hdr_deltas(baseline["hdr_haze"], bypass["hdr_haze"], full_depth),
    }


out = Path(os.environ["BOTW_FOG_OUT"])
with (out / "haze-mask-ab-verified.log").open("w", encoding="utf-8", buffering=1) as log:
    sys.stdout = sys.stderr = log
    capture = rd.OpenCaptureFile()
    replay = None
    try:
        assert capture.OpenFile(os.environ["BOTW_FOG_ORIGINAL"], "", None) == rd.ResultCode.Succeeded
        status, replay = capture.OpenCapture(rd.ReplayOptions(), None)
        assert status == rd.ResultCode.Succeeded, status
        print(json.dumps(compare(replay), indent=2))
    except Exception:
        traceback.print_exc()
        sys.exit(1)
    finally:
        if replay:
            replay.Shutdown()
        capture.Shutdown()
    sys.exit(0)
