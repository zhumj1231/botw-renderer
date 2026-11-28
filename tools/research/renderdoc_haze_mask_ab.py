"""Verify the haze fragment formulas and mask contribution in frame 3830."""

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

MASK_FRAGMENT = """#version 430
layout(set=1, binding=0) uniform sampler2D maskTexture;
layout(set=1, binding=1, std140) uniform Uniforms { float alphaRef; };
layout(location=0) in vec4 color0;
layout(location=1) in vec4 color1;
layout(location=2) in vec4 fade;
layout(location=3) in vec4 tint;
layout(location=4) in vec4 uv;
layout(location=0) out vec4 result;
void main() {
    float mask = texture(maskTexture, uv.xy).r;
    precise float alpha = fade.x * clamp(color0.a * (tint.a * mask), 0.0, 1.0);
    if (!(alpha > alphaRef)) discard;
    precise vec3 color = (color1.rgb + (color0.rgb - color1.rgb) * (mask * mask)) * tint.rgb;
    result = vec4(color, alpha);
}
"""

HAZE_FRAGMENT = """#version 430
layout(set=1, binding=0) uniform sampler2D maskTexture;
layout(set=1, binding=1, std140) uniform Uniforms { vec4 palette; float alphaRef; };
layout(location=0) in vec4 color;
layout(location=1) in vec4 projected;
layout(location=2) in vec4 fade;
layout(location=3) in vec4 tint;
layout(location=0) out vec4 result;
void main() {
    float mask = texture(maskTexture, projected.xy * (1.0 / projected.w)).r;
    precise float alpha = (fade.x * clamp(color.a * tint.a, 0.0, 1.0)) * clamp(mask * 0.5, 0.0, 1.0);
    if (!(alpha > alphaRef)) discard;
    precise vec3 rgb = (color.rgb * tint.a) * (palette.rgb * palette.a);
    result = vec4(rgb, alpha);
}
"""


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

def active_mask_difference(original, rebuilt):
    differences = []
    for y in range(90):
        for x in range(160):
            offset = 2 * (192 * y + x)
            a = struct.unpack_from("<e", original, offset)[0]
            b = struct.unpack_from("<e", rebuilt, offset)[0]
            differences.append(abs(a - b))
    return {
        "exact": sum(d == 0 for d in differences),
        "pixels": len(differences),
        "mean_abs": sum(differences) / len(differences),
        "max_abs": max(differences),
    }



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


def export_draws(replay, out, textures, baseline):
    """Keep post-VS inputs in a local fixture for an independent raster pass."""
    out.mkdir(parents=True, exist_ok=True)
    for key in ("mask_base", "mask_final", "effect_base", "effect_haze"):
        (out / (key + ".bin")).write_bytes(baseline[key])

    events = list(range(10745, 10820, 2)) + [13765]
    draws = []
    for event in events:
        replay.SetFrameEvent(event, True)
        pipe = replay.GetPipelineState()
        vk = replay.GetVulkanPipelineState()
        ps = rd.ShaderStage.Pixel
        mesh = replay.GetPostVSData(0, 0, rd.MeshDataStage.VSOut)
        assert mesh.indexByteStride == 4, event
        indices = bytes(replay.GetBufferData(
            mesh.indexResourceId, mesh.indexByteOffset, 4 * mesh.numIndices
        ))
        assert len(indices) == 4 * mesh.numIndices, event
        vertex_count = 1 + max(row[0] for row in struct.iter_unpack("<I", indices))
        vertices = bytes(replay.GetBufferData(
            mesh.vertexResourceId,
            mesh.vertexByteOffset,
            vertex_count * mesh.vertexByteStride,
        ))
        assert len(vertices) == vertex_count * mesh.vertexByteStride, event
        (out / ("vertices-%d.bin" % event)).write_bytes(vertices)
        (out / ("indices-%d.bin" % event)).write_bytes(indices)
        vp = pipe.GetViewport(0)
        block = pipe.GetConstantBlocks(ps)[0].descriptor
        uniform = bytes(replay.GetBufferData(block.resource, block.byteOffset, block.byteSize))
        (out / ("fragment-uniform-%d.bin" % event)).write_bytes(uniform)
        signature = pipe.GetShaderReflection(rd.ShaderStage.Vertex).outputSignature
        draws.append({
            "event": event,
            "vertex_stride": mesh.vertexByteStride,
            "vertex_count": vertex_count,
            "index_count": mesh.numIndices,
            "viewport": [vp.x, vp.y, vp.width, vp.height],
            "cull": str(vk.rasterizer.cullMode),
            "front_ccw": vk.rasterizer.frontCCW,
            "vertex_output_signature": [
                [element.varName, element.regIndex, element.compCount, str(element.systemValue)]
                for element in signature
            ],
        })

    inputs = []
    for key, address in (("noise_bc4", 27375), ("mask_r16f", 12607)):
        tex = textures["ResourceId::%d" % address]
        for mip in range(tex.mips):
            data = bytes(replay.GetTextureData(tex.resourceId, rd.Subresource(mip, 0, 0)))
            (out / ("%s-mip%d.bin" % (key, mip))).write_bytes(data)
        inputs.append({
            "name": key, "format": tex.format.Name(), "size": [tex.width, tex.height],
            "mips": tex.mips,
        })
    (out / "draws.json").write_text(json.dumps({"draws": draws, "inputs": inputs}, indent=2))
    return len(draws)


def compare_renderer(replay, fixture):
    textures = {t.resourceId: t for t in replay.GetTextures()}
    pending = {
        "haze_mask": ("mask_final.bin", 160, 90, 192, 2),
        "haze_effect": ("effect_haze.bin", 640, 360, 640, 8),
    }

    def flatten(actions):
        for action in actions:
            yield action
            yield from flatten(action.children)

    report = {}
    for action in reversed(list(flatten(replay.GetRootActions()))):
        if not (action.flags & rd.ActionFlags.Drawcall) or action.numIndices not in (144, 480):
            continue
        replay.SetFrameEvent(action.eventId, False)
        pipe = replay.GetPipelineState()
        shader = pipe.GetShaderReflection(rd.ShaderStage.Pixel)
        if not shader or shader.entryPoint not in pending:
            continue
        name, w, h, pitch, stride = pending.pop(shader.entryPoint)
        target = textures[pipe.GetOutputTargets()[0].resource]
        assert (target.width, target.height) == (w, h)
        replay.SetFrameEvent(action.eventId, True)
        gpu = bytes(replay.GetTextureData(target.resourceId, rd.Subresource(0, 0, 0)))
        padded = (fixture / name).read_bytes()
        row_bytes = pitch * stride
        native = b"".join(padded[row * row_bytes:row * row_bytes + w * stride] for row in range(h))
        assert len(gpu) == len(native) == w * h * stride
        differences = [abs(a - b) for (a,), (b,) in
                       zip(struct.iter_unpack("<e", native), struct.iter_unpack("<e", gpu))]
        report[shader.entryPoint] = {
            "event": action.eventId, "pixels": w * h,
            "changed_texels": changed(native, gpu, stride),
            "mean_abs": sum(differences) / len(differences),
            "max_abs": max(differences),
        }
        (fixture / ("renderer-" + name)).write_bytes(gpu)
        if not pending:
            return report
    raise RuntimeError("Missing renderer pass: " + ", ".join(pending))


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
    def replay_with(shader, source, keys):
        replacement, errors = replay.BuildTargetShader(
            "main", rd.ShaderEncoding.GLSL, source, rd.ShaderCompileFlags(), stage
        )
        assert int(replacement), errors
        replay.ReplaceResource(shader, replacement)
        try:
            return {key: read(*READBACKS[key]) for key in keys}
        finally:
            replay.RemoveReplacement(shader)
            replay.FreeTargetResource(replacement)

    bypass = replay_with(shader, bypass_source, READBACKS)

    for key in ("mask_base", "effect_base", "hdr_base"):
        assert baseline[key] == bypass[key], key
    assert bypass["mask_final"] == baseline["mask_base"]
    rebuilt_mask = replay_with(shader, MASK_FRAGMENT.encode("ascii"), ("mask_base", "mask_final"))
    assert rebuilt_mask["mask_base"] == baseline["mask_base"]
    replay.SetFrameEvent(13765, True)
    haze = replay.GetPipelineState().GetShader(stage)
    rebuilt_haze = replay_with(haze, HAZE_FRAGMENT.encode("ascii"), ("effect_base", "effect_haze"))
    assert rebuilt_haze["effect_base"] == baseline["effect_base"]
    report = {
        "changed_mask_texels": changed(baseline["mask_final"], bypass["mask_final"], 2),
        "changed_effect_texels": changed(baseline["effect_haze"], bypass["effect_haze"], 8),
        "hdr": hdr_deltas(baseline["hdr_haze"], bypass["hdr_haze"], full_depth),
        "mask_fragment_reconstruction": active_mask_difference(
            baseline["mask_final"], rebuilt_mask["mask_final"]
        ),
        "haze_fragment_reconstruction": {
            "changed_rgba16f_texels": changed(baseline["effect_haze"], rebuilt_haze["effect_haze"], 8),
        },
    }
    fixture = os.environ.get("BOTW_HAZE_EXPORT")
    if fixture:
        report["exported_draws"] = export_draws(replay, Path(fixture), textures, baseline)
    return report


out = Path(os.environ["BOTW_FOG_OUT"])
with (out / "haze-mask-ab-verified.log").open("w", encoding="utf-8", buffering=1) as log:
    sys.stdout = sys.stderr = log
    capture = rd.OpenCaptureFile()
    replay = None
    try:
        renderer = os.environ.get("BOTW_HAZE_RENDERER_CAPTURE")
        capture_path = renderer or os.environ["BOTW_FOG_ORIGINAL"]
        assert capture.OpenFile(capture_path, "", None) == rd.ResultCode.Succeeded
        status, replay = capture.OpenCapture(rd.ReplayOptions(), None)
        assert status == rd.ResultCode.Succeeded, status
        report = compare_renderer(replay, Path(os.environ["BOTW_HAZE_EXPORT"])) if renderer else compare(replay)
        print(json.dumps(report, indent=2))
    except Exception:
        traceback.print_exc()
        sys.exit(1)
    finally:
        if replay:
            replay.Shutdown()
        capture.Shutdown()
    sys.exit(0)
