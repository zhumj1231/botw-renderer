import json, os, struct, sys, traceback
from pathlib import Path
import renderdoc as rd

def walk(actions):
    for action in actions:
        yield action
        yield from walk(action.children)

def dump_renderer(c, out):
    textures = {t.resourceId: t for t in c.GetTextures()}
    stage = rd.ShaderStage.Pixel
    for action in reversed(list(walk(c.GetRootActions()))):
        if not (action.flags & rd.ActionFlags.Drawcall) or action.numIndices != 3:
            continue
        c.SetFrameEvent(action.eventId, False)
        pipe = c.GetPipelineState()
        shader = pipe.GetShaderReflection(stage)
        if not shader or shader.entryPoint != "composite_overlay":
            continue
        tex = textures[pipe.GetOutputTargets()[0].resource]
        assert (tex.width, tex.height, tex.format.Name()) == (1280, 720, "R16G16B16A16_FLOAT")
        c.SetFrameEvent(action.eventId, True)
        data = bytes(c.GetTextureData(tex.resourceId, rd.Subresource(0, 0, 0)))
        assert len(data) == 1280 * 720 * 8
        (out / "overlay-ab-gpu.bin").write_bytes(data)
        return action.eventId
    raise RuntimeError("Renderer capture has no composite_overlay draw")

out = Path(os.environ["BOTW_FOG_OUT"])
with (out / "overlay-ab-export.log").open("w",encoding="utf-8",buffering=1) as log:
    sys.stdout=sys.stderr=log
    cap=rd.OpenCaptureFile()
    c=None
    try:
        renderer = os.environ.get("BOTW_OVERLAY_RENDERER_CAPTURE")
        path = renderer or os.environ["BOTW_FOG_ORIGINAL"]
        assert cap.OpenFile(path,"",None)==rd.ResultCode.Succeeded
        status,c=cap.OpenCapture(rd.ReplayOptions(),None)
        assert status==rd.ResultCode.Succeeded,status
        if renderer:
            print("Exported renderer HDR at E%d" % dump_renderer(c, out))
            sys.exit(0)
        textures={str(t.resourceId):t for t in c.GetTextures()}
        def dump(eid, rid, label):
            c.SetFrameEvent(eid,True)
            t=textures[rid]
            (out / ("overlay-ab-"+label+".bin")).write_bytes(bytes(c.GetTextureData(t.resourceId,rd.Subresource(0,0,0))))
            return [t.width,t.height,t.format.Name()]
        c.SetFrameEvent(13778,True)
        p=c.GetPipelineState()
        fs=p.GetShaderReflection(rd.ShaderStage.Pixel)
        inputs={fs.readOnlyResources[r.access.index].name:str(r.descriptor.resource) for r in p.GetReadOnlyResources(rd.ShaderStage.Pixel)}
        target=str(p.GetOutputTargets()[0].resource)
        meta=dict(inputs=inputs,target=target,buffers={},
                  samplers=[(r.access.index,str(r.descriptor.filter.minify),str(r.descriptor.filter.magnify),str(r.descriptor.addressU),str(r.descriptor.addressV)) for r in p.GetSamplers(rd.ShaderStage.Pixel)])
        for r in p.GetConstantBlocks(rd.ShaderStage.Pixel):
            b=r.descriptor
            meta["constants"]=list(struct.iter_unpack("<4f",bytes(c.GetBufferData(b.resource,b.byteOffset,min(64,b.byteSize)))))
        vb=c.GetPostVSData(0,0,rd.MeshDataStage.VSOut)
        meta["vertices"]=list(struct.iter_unpack("<"+"f"*(vb.vertexByteStride//4),bytes(c.GetBufferData(vb.vertexResourceId,vb.vertexByteOffset,vb.vertexByteStride*vb.numIndices))))
        for eid,rid,name in [(13778,target,"hdr-post"),(13777,target,"hdr-pre"),
                             (13777,inputs["textureUnitPS0"],"effect-post"),
                             (13777,inputs["textureUnitPS1"],"depth-full"),
                             (13777,inputs["textureUnitPS2"],"depth-half"),
                             (13764,"ResourceId::20013","effect-pre")]:
            meta["buffers"][name]=dump(eid,rid,name)
        (out/"overlay-ab-export.json").write_text(json.dumps(meta,indent=2),encoding="utf-8")
        print(meta)
    except Exception:
        traceback.print_exc()
        sys.exit(1)
    finally:
        if c: c.Shutdown()
        cap.Shutdown()
    sys.exit(0)
