"""Compare the 4188/12902 Tarrey Town pair on four matched Vulkan replays."""
import json, math, os, struct, sys, traceback
from pathlib import Path
import renderdoc as rd
from collections import Counter

out = Path(os.environ["BOTW_FOG_OUT"])
reference = Path(os.environ["BOTW_DAY_NIGHT_DIR"])
stage = rd.ShaderStage.Pixel
check_ao = os.environ.get("BOTW_COMPARE_EXPAND_INPUTS") == "1"
original_events = {
    4188: (10327,11389,11296,13867),
    12902: (8677,9763,9670,12258),
}

def walk(nodes):
    for node in nodes:
        yield node
        yield from walk(node.children)

def native_cascade_projection(replay, state):
    def vectors(block, first, count):
        buf = state.GetConstantBlock(stage, block, 0).descriptor
        data = replay.GetBufferData(buf.resource, buf.byteOffset + first * 16, count * 16)
        return list(struct.iter_unpack("<4f", bytes(data)))

    def dot(a, b):
        return sum(x * y for x, y in zip(a, b))

    def unit(v):
        n = math.sqrt(dot(v, v))
        return tuple(x / n for x in v)

    light = unit(vectors(1, 4, 1)[0][:3])
    forward = (0.0, 0.0, -1.0)
    project = unit(tuple(x - dot(forward, light) * y for x, y in zip(forward, light)))
    matrices = vectors(0, 42, 12)
    cascades = []
    for index in range(3):
        rows = matrices[4 * index:4 * index + 4]
        w = rows[3]
        cascades.append({
            "w_row": w,
            "projected_forward_component_error": max(abs(a - b) for a, b in zip(w[:3], project)),
            "uv_derivative_along_light_before_divide": [dot(row[:3], light) for row in rows[:2]],
            "w_derivative_along_light": dot(w[:3], light),
        })
    return {"projection": cascades, "split_m": vectors(0, 58, 1)[0][:3]}


def ao_input(replay, state, native):
    textures = {tex.resourceId: tex for tex in replay.GetTextures()}
    def texture(predicate):
        for access in state.GetReadOnlyResources(rd.ShaderStage.Pixel):
            descriptor = access.descriptor
            tex = textures.get(descriptor.resource)
            if tex and predicate(tex):
                data = replay.GetTextureData(
                    tex.resourceId, rd.Subresource(descriptor.firstMip, descriptor.firstSlice, 0)
                )
                return bytes(data)
        raise ValueError("Expected depth or rotation input is absent")

    noise = texture(lambda tex: (tex.width, tex.height) == (4, 4) and tex.format.Name() == "R8G8_UNORM")
    w, h, fmt = (640, 360, "R32_FLOAT") if native else (1280, 720, "D32")
    depth = texture(lambda tex: (tex.width, tex.height, tex.format.Name()) == (w, h, fmt))
    return noise, tuple(value for value, in struct.iter_unpack("<f", depth))


def clamp(value):
    return min(1.0, max(0.0, value))


def pair(biased, plus, minus, gain):
    p, m = biased - plus, biased - minus
    tp, tm = clamp(gain * p + 0.5), clamp(gain * m + 0.5)
    fp, fm = clamp(1.0 - p / 1.2), clamp(1.0 - m / 1.2)
    up, um = 0.5 + fp * (0.5 - tp), 0.5 + fm * (0.5 - tm)
    return um + (tp - um) * fp + up + (tm - up) * fm


def offsets_for_noise(noise):
    # Both UV axes become offsets in pixels using the active viewport height.
    offsets = []
    for i in range(0, len(noise), 2):
        r, s = (channel * (2.0 / 255.0) - 1.0 for channel in noise[i:i+2])
        offsets.append(tuple(
            (360 * (r * x - s * y), 360 * (s * x + r * y))
            for x, y in ((-0.0036606, 0.0364835), (0.0199001, 0.0019967))
        ))
    return offsets


def compare_ao(original, renderer, points):
    native_noise, native_depth = original["ao_inputs"]
    host_noise, host_depth = renderer["ao_inputs"]
    assert native_noise == host_noise, "Rotation textures differ"
    offsets = offsets_for_noise(native_noise)

    def half_index(x, y):
        return min(359, max(0, int(y))) * 640 + min(639, max(0, int(x)))

    def ao(depth, index, taps, cutoff):
        z = depth[index]
        if z >= cutoff:
            return 255
        bias = 0.995 * z + 0.005
        a = pair(bias, depth[taps[0]], depth[taps[1]], 14.70871)
        b = pair(bias, depth[taps[2]], depth[taps[3]], 10.9109)
        return round(255 * (1.0 - clamp(6.0 * (0.17764 * a + 0.254544 * b - 0.4321855))))

    native = tuple(1.0 + 24999.0 * depth for depth in native_depth)
    host = {}
    for sy in range(2):
        for sx in range(2):
            depths = host_depth[sy * 1280 + sx:]
            host[sx, sy] = tuple(
                1.0 / d if d > 0.0 else math.inf
                for y in range(360)
                for d in depths[y * 2560:y * 2560 + 1280:2]
            )
    current = host[renderer["ao_corner"]]
    # The matching pair uses native 120 m AO reach and the host's selected
    # cascade range. Compare baseline and split-override captures separately.
    tasks = {
        "native_inputs_vs_native": (native, 120.0, original),
        "renderer_inputs_vs_renderer": (current, renderer["ao_reach"], renderer),
    }
    for (sx, sy), depth in host.items():
        tasks["renderer_corner_%d_%d_vs_native" % (sx, sy)] = (depth, 120.0, original)
    counts = {key: [0, 0, 0] for key in tasks}
    native_only_occluders = Counter()
    severe = []
    for x, y in points:
        i = y * 640 + x
        taps = []
        for dx, dy in offsets[(y % 4) * 4 + x % 4]:
            taps.extend((half_index(x + 0.5 + dx, y + 0.5 + dy),
                         half_index(x + 0.5 - dx, y + 0.5 - dy)))
        for key, (depth, cutoff, capture) in tasks.items():
            captured = capture["expand"][3][4 * i + 2]
            modeled = ao(depth, i, taps, cutoff)
            err = abs(modeled - captured)
            counts[key][0] += err
            counts[key][1] += err > 1
            counts[key][2] += modeled < 255
        n = original["expand"][3][4 * i + 2]
        h = renderer["expand"][3][4 * i + 2]
        if n >= 250 or h < 250:
            continue
        for q in taps:
            if native[q] >= native[i] - 0.05 or current[q] <= native[q] + 0.05:
                continue
            pair_key = tuple(struct.unpack_from("<H", image["class"][3], 2 * q)[0] for image in (original, renderer))
            native_only_occluders[pair_key] += 1
            if n == 0 and h == 255 and pair_key == (1028, 1028):
                # Retain a small set of receiver and blocker points for raster history.
                severe.append((current[q] - native[q], [x, y], [q % 640, q // 640], native[i], current[i], native[q], current[q]))
    return {
        "matched_points": len(points),
        "noise_bytes_match": True,
        "replays": {key: {"mean_abs_codes": total / len(points), "over_one_code": errors, "active_pixels": active}
                    for key, (total, errors, active) in counts.items()},
        "native_only_nearer_taps_by_class": native_only_occluders.most_common(),
        "same_class_pixel_history_candidates": sorted(severe, reverse=True)[:4],
    }


def read_replay(c, frame, native):
    result = {}
    textures = {t.resourceId:t for t in c.GetTextures()}
    def read(binding):
        t = textures[binding.resource]
        data = bytes(c.GetTextureData(t.resourceId,rd.Subresource(binding.firstMip,binding.firstSlice,0)))
        return (t.width,t.height,t.format.Name(),data)
    def selected(nid, entry, before=100000, instance=None, fmt=None):
        if native:
            event = nid
        else:
            for a in reversed(actions):
                if a.eventId>=before or a.numIndices!=3 or not a.flags&rd.ActionFlags.Drawcall:
                    continue
                if instance is not None and a.instanceOffset!=instance:
                    continue
                target=textures.get(a.outputs[0]) if a.outputs else None
                if fmt and (not target or target.format.Name()!=fmt):
                    continue
                c.SetFrameEvent(a.eventId,False)
                s=c.GetPipelineState().GetShaderReflection(stage)
                if s and s.entryPoint==entry:
                    event=a.eventId
                    break
            else:
                raise AssertionError(entry)
        c.SetFrameEvent(event,True)
        return event,c.GetPipelineState()
    def matching_input(pipe,predicate):
        accesses=pipe.GetReadOnlyResources(stage)
        return next(a.descriptor for a in accesses if a.descriptor.resource in textures and predicate(textures[a.descriptor.resource]))
    actions=list(walk(c.GetRootActions()))
    ex,pr,an,ds=original_events[frame]
    e,p=selected(ex,"expand",instance=4)
    result["expand"]=read(p.GetOutputTargets()[5 if native else 0])
    result["class"]=read(p.GetDepthTarget())
    if not native:
        result["depth"]=read(matching_input(p,lambda t:t.width==1280 and t.height==720 and t.format.Name()=="D32"))
    result["expand_event"]=e
    if check_ao:
        result["ao_inputs"] = ao_input(c, p, native)
        if native:
            result["shadow_projection"] = native_cascade_projection(c, p)
        else:
            result["ao_reach"] = float(os.environ.get("BOTW_RENDERER_AO_REACH", "200"))
            result["ao_corner"] = tuple(int(value) for value in os.environ.get(
                "BOTW_RENDERER_AO_CORNER", "1,1"
            ).split(","))

    e,p=selected(pr,"preshade",instance=4)
    if native:
        ref=p.GetShaderReflection(stage)
        binding=next(a.descriptor for a in p.GetReadOnlyResources(stage)
                     if ref.readOnlyResources[a.access.index].name=="textureUnitPS6")
        result["depth"]=read(binding)
    for index,key in enumerate(("shadow","fog","diffuse")):
        result[key]=read(p.GetOutputTargets()[index])
    result["pre_event"]=e
    e,p=selected(ds,"hdr_compose")
    if native:
        ref=p.GetShaderReflection(stage)
        b=next(a.descriptor for a in p.GetReadOnlyResources(stage)
               if ref.readOnlyResources[a.access.index].name=="textureUnitPS1")
    else:
        b=matching_input(p,lambda t:t.width==1280 and t.height==720 and t.format.Name()=="R16G16B16A16_FLOAT")
    result["hdr"]=read(b)
    result["hdr_event"]=e
    result["sky_overlay"] = sky_overlay(c, actions, textures, e, native)
    return result


def sky_overlay(replay, actions, textures, display_event, native):
    """Identify the pre-display depth-guided overlay and isolate one sky sample."""
    candidates = set()
    if not native:
        for tex in textures.values():
            if (tex.width, tex.height, tex.format.Name()) != (640, 360, "R16G16B16A16_FLOAT"):
                continue
            candidates.update(usage.eventId for usage in replay.GetUsage(tex.resourceId)
                              if usage.usage == rd.ResourceUsage.PS_Resource)
    for action in reversed(actions):
        if not native and action.eventId not in candidates:
            continue
        if (action.eventId >= display_event or (native and action.numIndices != 3)
                or not action.flags & rd.ActionFlags.Drawcall or not action.outputs):
            continue
        target = textures.get(action.outputs[0])
        if not target or (target.width, target.height) != (1280, 720):
            continue
        replay.SetFrameEvent(action.eventId, False)
        pipe = replay.GetPipelineState()
        shader = pipe.GetShaderReflection(stage)
        if not shader:
            continue
        inputs = {
            shader.readOnlyResources[r.access.index].name: textures[r.descriptor.resource]
            for r in pipe.GetReadOnlyResources(stage) if r.descriptor.resource in textures
        }
        if native:
            expected = {
                "textureUnitPS0": "R16G16B16A16_FLOAT",
                "textureUnitPS1": "R16_FLOAT",
                "textureUnitPS2": "R32_FLOAT",
            }
            if any(name not in inputs or inputs[name].format.Name() != fmt for name, fmt in expected.items()):
                continue
            blend = pipe.GetColorBlends()[0].colorBlend
            if (str(blend.source), str(blend.destination)) != ("BlendMultiplier.One", "BlendMultiplier.SrcAlpha"):
                continue
            source = target
        elif shader.entryPoint == "composite_overlay":
            source = next(t for t in inputs.values()
                          if t.width == 1280 and t.format.Name() == "R16G16B16A16_FLOAT")
        else:
            sizes = {(t.width, t.height, t.format.Name()) for t in inputs.values()}
            expected = {(640, 360, "R16G16B16A16_FLOAT"), (640, 360, "R32_FLOAT"),
                        (1280, 720, "R16_FLOAT")}
            if not expected.issubset(sizes):
                continue
            blend = pipe.GetColorBlends()[0].colorBlend
            if (str(blend.source), str(blend.destination)) != ("BlendMultiplier.One", "BlendMultiplier.SrcAlpha"):
                continue
            replay.SetFrameEvent(action.eventId, True)
            history = replay.PixelHistory(target.resourceId, 640, 45, rd.Subresource(0, 0, 0),
                                          rd.CompType.Typeless)
            mod = next(m for m in history if m.eventId == action.eventId and m.Passed())
            return {"event": action.eventId, "before": list(mod.preMod.col.floatValue)[:3],
                    "after": list(mod.postMod.col.floatValue)[:3]}

        def color(event, tex):
            replay.SetFrameEvent(event, True)
            data = bytes(replay.GetTextureData(tex.resourceId, rd.Subresource(0, 0, 0)))
            return pixel((tex.width, tex.height, tex.format.Name(), data), 640, 45)[:3]

        return {"event": action.eventId,
                "before": color(action.eventId - 1, source), "after": color(action.eventId, target)}
    raise RuntimeError("Depth-guided overlay draw not found")

def component(v,bits):
    e=v>>bits
    fraction = (v & ((1<<bits)-1)) / (1<<bits)
    return math.ldexp(fraction if e==0 else 1+fraction,-14 if e==0 else e-15)

def pixel(image,x,y):
    w,h,fmt,data=image
    stride=len(data)//(w*h)
    offset=(y*w+x)*stride
    if fmt=="R11G11B10_FLOAT":
        v=struct.unpack_from("<I",data,offset)[0]
        return (component(v&2047,6),component((v>>11)&2047,6),component(v>>22,5))
    if fmt=="R16G16B16A16_FLOAT":
        return struct.unpack_from("<4e",data,offset)
    if fmt in ("R32_FLOAT","D32"):
        return struct.unpack_from("<f",data,offset)[0]
    if fmt=="D16":
        return struct.unpack_from("<H",data,offset)[0]
    if fmt=="R8G8B8A8_UNORM":
        return tuple(data[offset:offset+4])
    raise ValueError(fmt)

def compare(a,b):
    errors=[0.]*7
    shadow_ranges={label:[0,0,0,0,0] for label in ("under_20m","20_to_120m","over_120m")}
    ao_active=[0,0,0,0,0]
    classes = {"native_terrain":0,"both_terrain":0,"depth_matched":0}
    depth_points=((272,237),(236,188),(493,154))
    sample=[]
    matched_points=[]
    def safe_depth(x,y):
        if pixel(a["class"],x,y)!=1028 or pixel(b["class"],x,y)!=1028:
            return None
        z=1+24999*pixel(a["depth"],x,y)
        rev=pixel(b["depth"],2*x+1,2*y+1)
        if rev<=0:
            return None
        return z,1/rev
    for y in range(20,340):
        for x in range(20,620):
            if pixel(a["class"],x,y)!=1028:
                continue
            classes["native_terrain"]+=1
            if pixel(b["class"],x,y)!=1028:
                continue
            classes["both_terrain"]+=1
            z=safe_depth(x,y)
            if not z or abs(z[0]-z[1])>max(0.5,0.02*z[0]):
                continue
            classes["depth_matched"]+=1
            if check_ao:
                matched_points.append((x,y))
            ax,bx=pixel(a["expand"],x,y),pixel(b["expand"],x,y)
            shadow_delta=bx[0]-ax[0]
            bucket=shadow_ranges["under_20m" if z[0]<20 else ("20_to_120m" if z[0]<120 else "over_120m")]
            for j,v in enumerate((1,shadow_delta,abs(shadow_delta),ax[0],bx[0])):
                bucket[j]+=v
            if min(ax[2],bx[2])<255:
                for j,v in enumerate((1,abs(ax[2]-bx[2]),ax[2]<255,bx[2]<255,bx[2]-ax[2])):
                    ao_active[j]+=v
            as_,bs=pixel(a["shadow"],x,y),pixel(b["shadow"],x,y)
            samples=[abs(ax[0]-bx[0]),abs(ax[2]-bx[2]),abs(as_[0]-bs[0]),abs(as_[2]-bs[2])]
            for stage in ("fog","diffuse","hdr"):
                nx,ny=(2*x+1,2*y+1) if stage=="hdr" else (x,y)
                samples.append(sum(abs(v-w) for v,w in zip(pixel(a[stage],nx,ny)[:3],pixel(b[stage],nx,ny)[:3]))/3)
            for i,value in enumerate(samples):
                errors[i]+=value
    for x,y in depth_points:
        sample.append({"xy":[x,y],"depth":safe_depth(x,y),"native_fog":pixel(a["fog"],x,y),"host_fog":pixel(b["fog"],x,y)})
    keys=("expand_red_codes","ao_blue_codes","preshade_shadow_red_codes","transmission_codes","fog_rgb","diffuse_rgb","display_input_hdr_rgb")
    n=classes["depth_matched"]
    assert n>0,classes
    report = {"events":{side:{k:v for k,v in data.items() if k.endswith("_event")} for side,data in (("original",a),("renderer",b))},
            "layouts": {side:{k:v[:3] for k,v in data.items() if isinstance(v, tuple) and len(v) == 4} for side,data in (("original",a),("renderer",b))},
            "coverage":classes,"mean_abs":dict(zip(keys,[v/n for v in errors])),
            "shadow_by_distance":{k:{"samples":n,"signed_codes":sg/n,"mean_abs_codes":ma/n,"original_mean":oa/n,"renderer_mean":ob/n} for k,(n,sg,ma,oa,ob) in shadow_ranges.items() if n},
            "ao_active_union":{"samples":ao_active[0],"mean_abs_codes":ao_active[1]/max(1,ao_active[0]),"original_active":ao_active[2],"renderer_active":ao_active[3],"signed_codes":ao_active[4]/max(1,ao_active[0])},
            "fog_points":sample,
            "sky_hdr_at_640_45":[pixel(a["hdr"],640,45),pixel(b["hdr"],640,45)],
            "sky_overlay": {"original": a["sky_overlay"], "renderer": b["sky_overlay"]}}

    if check_ao:
        report["ao_depth_replay"] = compare_ao(a, b, matched_points)
        report["native_shadow_projection"] = a["shadow_projection"]
    return report


with (out/"day-night-stage-validation.log").open("w",encoding="utf-8",buffering=1) as log:
    sys.stdout=sys.stderr=log
    try:
        report={}
        capture_pattern = os.environ.get("BOTW_RENDERER_CAPTURE_PATTERN", "renderer-paired-%d_capture.rdc")
        frames = tuple(int(value) for value in os.environ.get("BOTW_DAY_NIGHT_FRAMES", "4188,12902").split(","))
        for frame in frames:
            data=[]
            for native,path in ((True,next(reference.glob("*frame%d.rdc"%frame))),
                                (False,out/(capture_pattern%frame))):
                cap=rd.OpenCaptureFile()
                c=None
                try:
                    assert cap.OpenFile(str(path),"",None)==rd.ResultCode.Succeeded
                    status,c=cap.OpenCapture(rd.ReplayOptions(),None)
                    assert status==rd.ResultCode.Succeeded,status
                    data.append(read_replay(c,frame,native))
                    print(frame,path.name,"exported")
                finally:
                    if c: c.Shutdown()
                    cap.Shutdown()
            report[frame]=compare(*data)
            print(frame,json.dumps(report[frame]))
        (out/"day-night-stage-validation.json").write_text(json.dumps(report,indent=2),encoding="utf-8")
    except Exception:
        traceback.print_exc()
        sys.exit(1)
    sys.exit(0)
