"""Hold mesh, HDR and depth constant while replacing two haze fragment inputs."""
import json, os, struct, subprocess, sys, traceback
from pathlib import Path
import renderdoc as rd

out=Path(os.environ["BOTW_FOG_OUT"])
stage=rd.ShaderStage.Pixel

def read(replay,tex,x,y):
    b=bytes(replay.GetTextureData(tex.resourceId,rd.Subresource(0,0,0)))
    return struct.unpack_from("<4e",b,8*(y*tex.width+x))

def sample(replay,draw,compose,effect,hdr):
    replay.SetFrameEvent(draw,True)
    haze=read(replay,effect,320,22)
    replay.SetFrameEvent(compose,True)
    return {"effect":haze,"hdr":read(replay,hdr,640,45)[:3]}

def compare(replay,params):
    draw,compose,mask=params
    textures={t.resourceId:t for t in replay.GetTextures()}
    replay.SetFrameEvent(compose,True)
    hdr=textures[replay.GetPipelineState().GetOutputTargets()[0].resource]
    replay.SetFrameEvent(draw,True)
    p=replay.GetPipelineState()
    old=p.GetShader(stage)
    ps=p.GetShaderReflection(stage)
    assert ps.entryPoint=="live_haze_effect"
    effect=textures[p.GetOutputTargets()[0].resource]
    source=subprocess.check_output(["spirv-cross","-","--vulkan-semantics"],input=bytes(ps.rawBytes)).decode()
    factor="_29._m5.x * clamp"
    mask_ref="clamp(0.5 * _74.x, 0.0, 1.0)"
    assert source.count(factor)==1 and source.count(mask_ref)==1
    variants={
        "factor_only":source.replace(factor,"0.125 * clamp"),
        "mask_only":source.replace(mask_ref,"clamp(0.5 * %.9f, 0.0, 1.0)"%mask),
    }
    variants["both"]=variants["mask_only"].replace(factor,"0.125 * clamp")
    report={"renderer":sample(replay,draw,compose,effect,hdr)}
    for label,code in variants.items():
        shader,errors=replay.BuildTargetShader("main",rd.ShaderEncoding.GLSL,code.encode(),rd.ShaderCompileFlags(),stage)
        assert int(shader),errors
        replay.ReplaceResource(old,shader)
        try:
            report[label]=sample(replay,draw,compose,effect,hdr)
        finally:
            replay.RemoveReplacement(old)
            replay.FreeTargetResource(shader)
    return report

with (out/"day-night-haze-ab.log").open("w",buffering=1) as log:
    sys.stdout=sys.stderr=log
    try:
        result={}
        for frame,params in ((4188,(24339,24351,0.673462)),(12902,(27183,27195,1.475342))):
            path=out/("renderer-paired-%d_capture.rdc"%frame)
            cap=rd.OpenCaptureFile()
            c=None
            try:
                assert cap.OpenFile(str(path),"",None)==rd.ResultCode.Succeeded
                status,c=cap.OpenCapture(rd.ReplayOptions(),None)
                assert status==rd.ResultCode.Succeeded,status
                result[frame]=compare(c,params)
                print(frame,result[frame])
            finally:
                if c: c.Shutdown()
                cap.Shutdown()
        (out/"day-night-haze-ab.json").write_text(json.dumps(result,indent=2),encoding="utf-8")
    except:
        traceback.print_exc()
        sys.exit(1)
    sys.exit(0)
