"""Compare the 4188/12902 Tarrey Town pair on four matched Vulkan replays."""
import json, math, os, struct, sys, traceback
from pathlib import Path
import renderdoc as rd

out = Path(os.environ["BOTW_FOG_OUT"])
reference = Path(os.environ["BOTW_DAY_NIGHT_DIR"])
stage = rd.ShaderStage.Pixel
original_events = {
    4188: (10327,11389,11296,13867),
    12902: (8677,9763,9670,12258),
}

def walk(nodes):
    for node in nodes:
        yield node
        yield from walk(node.children)

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
    for action in reversed(actions):
        if (action.eventId >= display_event or action.numIndices != 3
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
            continue

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
    return {"events":{side:{k:v for k,v in data.items() if k.endswith("_event")} for side,data in (("original",a),("renderer",b))},
            "layouts": {side:{k:v[:3] for k,v in data.items() if isinstance(v, tuple)} for side,data in (("original",a),("renderer",b))},
            "coverage":classes,"mean_abs":dict(zip(keys,[v/n for v in errors])),
            "shadow_by_distance":{k:{"samples":n,"signed_codes":sg/n,"mean_abs_codes":ma/n,"original_mean":oa/n,"renderer_mean":ob/n} for k,(n,sg,ma,oa,ob) in shadow_ranges.items() if n},
            "ao_active_union":{"samples":ao_active[0],"mean_abs_codes":ao_active[1]/max(1,ao_active[0]),"original_active":ao_active[2],"renderer_active":ao_active[3],"signed_codes":ao_active[4]/max(1,ao_active[0])},
            "fog_points":sample,
            "sky_hdr_at_640_45":[pixel(a["hdr"],640,45),pixel(b["hdr"],640,45)],
            "sky_overlay": {"original": a["sky_overlay"], "renderer": b["sky_overlay"]}}

with (out/"day-night-stage-validation.log").open("w",encoding="utf-8",buffering=1) as log:
    sys.stdout=sys.stderr=log
    try:
        report={}
        for frame in (4188,12902):
            data=[]
            for native,path in ((True,next(reference.glob("*frame%d.rdc"%frame))),
                                (False,out/("renderer-paired-%d_capture.rdc"%frame))):
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
