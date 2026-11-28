import json, os, struct, sys, traceback
from pathlib import Path
import renderdoc as rd

out = Path(os.environ["BOTW_FOG_OUT"])
with (out / "overlay-ab-export.log").open("w",encoding="utf-8",buffering=1) as log:
    sys.stdout=sys.stderr=log
    cap=rd.OpenCaptureFile()
    c=None
    try:
        assert cap.OpenFile(os.environ["BOTW_FOG_ORIGINAL"],"",None)==rd.ResultCode.Succeeded
        status,c=cap.OpenCapture(rd.ReplayOptions(),None)
        assert status==rd.ResultCode.Succeeded,status
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
