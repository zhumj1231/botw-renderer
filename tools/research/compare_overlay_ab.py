import json, math, struct, sys
import os
root=sys.argv[1]
with open(os.path.join(root,"overlay-ab-export.json")) as file:
    meta=json.load(file)

def half(code):
    exp=(code>>10)&31
    f=(code&1023)/1024.0
    return (-1 if code&32768 else 1)*math.ldexp((1 if exp else 0)+f,max(1,exp)-15)
half_table=[half(i) for i in range(65536)]

def pixels(name, fmt):
    with open(os.path.join(root,"overlay-ab-"+name+".bin"),"rb") as file:
        data=file.read()
    half_fmt="e" in fmt
    decoder=struct.Struct(fmt.replace("e","H"))
    rows=[decoder.unpack_from(data,i) for i in range(0,len(data),decoder.size)]
    if half_fmt:
        return [tuple(half_table[v] for v in row) for row in rows]
    return rows
def hdr_lut(bits):
    return [math.ldexp(((i&((1<<bits)-1))/float(1<<bits))+(1 if i>>bits else 0),max(1,i>>bits)-15)
            for i in range(31<<bits)]
lut6,lut5=hdr_lut(6),hdr_lut(5)
def decode(row):
    v=row[0]
    return (lut6[v&2047],lut6[(v>>11)&2047],lut5[v>>22])
before=[decode(row) for row in pixels("hdr-pre","<I")]
actual=[decode(row) for row in pixels("hdr-post","<I")]
depth=[row[0] for row in pixels("depth-full","<e")]
guide=[row[0] for row in pixels("depth-half","<f")]
post=pixels("effect-post","<4e")
pre=pixels("effect-pre","<4e")[:640*360]
gpu=pixels("gpu","<4e") if "--gpu" in sys.argv[2:] else None
if gpu is not None:
    assert len(gpu)==len(actual)
gpu_errors={}
b,scale=meta["constants"][0][0],meta["constants"][1][3]
stats={label:[0,0.0,0.0,0.0,0.0,0.0,0.0,0] for label in ("all","sky","surface","sky_geometry_edge","far_slope_box")}
sample={}
worst=(0,None)
edge_bilinear_error=0.0
def clip(v,hi):
    return max(0,min(hi,v))
def composite(image, ids, weights, base):
    c0,c1,c2,c3=(image[k] for k in ids)
    w0,w1,w2,w3=weights
    alpha=c0[3]*w0+c1[3]*w1+c2[3]*w2+c3[3]*w3
    return tuple(c0[k]*w0+c1[k]*w1+c2[k]*w2+c3[k]*w3+base[k]*alpha for k in range(3))
def quantize(value,bits):
    mantissa, exponent=math.frexp(value)
    step=math.ldexp(1,max(-14,exponent-1)-bits)
    return step*math.floor(value/step)
for y in range(720):
    fy=y*0.5-.25
    j=int(math.floor(fy))
    y0,y1=clip(j,359)*640,clip(j+1,359)*640
    by=fy-j
    for x in range(1280):
        fx=x*0.5-.25
        i=int(math.floor(fx))
        x0,x1=clip(i,639),clip(i+1,639)
        bx=fx-i
        ids=(y0+x0,y0+x1,y1+x0,y1+x1)
        k=y*1280+x
        z=depth[k]
        tl,tr,bl,br=(abs(guide[t]-z) for t in ids)
        factor=scale/(z+b)
        hx=clip(bx+factor*(tl+bl-tr-br),1)
        hy=clip(by+factor*(tl+tr-bl-br),1)
        ws=((1-hx)*(1-hy),hx*(1-hy),(1-hx)*hy,hx*hy)
        base=before[k]
        final=composite(post,ids,ws,base)
        without=composite(pre,ids,ws,base)
        quant=tuple(quantize(f,n) for f,n in zip(final,(6,6,5)))
        err=tuple(f-a for f,a in zip(quant,actual[k]))
        delta=tuple(f-a for f,a in zip(final,without))
        groups=["all","sky" if z>=1 else "surface"]
        ds=[guide[t] for t in ids]
        if min(ds)<0.999 and max(ds)>=0.99999:
            groups.append("sky_geometry_edge")
            bilinear=((1-bx)*(1-by),bx*(1-by),(1-bx)*by,bx*by)
            unguided=composite(post,ids,bilinear,base)
            edge_bilinear_error+=sum(abs(quantize(f,n)-a) for f,n,a in zip(unguided,(6,6,5),actual[k]))/3
        if 848<=x<=894 and 64<=y<=83 and z<1:
            groups.append("far_slope_box")
        if gpu is not None:
            raw_error=tuple(abs(f-a) for f,a in zip(gpu[k],final))
            # Compare to the native target after mapping the renderer's RGBA16F to R11G11B10F.
            gpu_quant=tuple(quantize(f,n) for f,n in zip(gpu[k],(6,6,5)))
            native_error=tuple(abs(f-a) for f,a in zip(gpu_quant,actual[k]))
            for g in groups:
                a=gpu_errors.setdefault(g,[0,0.0,0.0,0.0,0.0,0])
                a[0]+=1
                a[1]+=sum(raw_error)/3
                a[2]=max(a[2],max(raw_error))
                a[3]+=sum(native_error)/3
                a[4]=max(a[4],max(native_error))
                a[5]+=gpu_quant==actual[k]
        ee=sum(map(abs,err))/3
        dd=sum(map(abs,delta))/3
        if max(map(abs,err)) > worst[0]: worst=(max(map(abs,err)),(x,y,base,quant,actual[k],z,ds))
        for g in groups:
            a=stats[g]
            a[0]+=1
            a[1]+=ee
            a[2]=max(a[2],max(map(abs,err)))
            a[3]+=dd
            for j in range(3):
                a[4+j]+=delta[j]
            a[7]+=quant==actual[k]
        if (x,y) in [(873,75),(640,45),(425*2,49*2),(640,350),(10,600)]:
            sample[str((x,y))]=dict(depth=z,full=actual[k],model=quant,no_draw=without,delta=delta)
report={g:dict(pixels=a[0], error_mean=a[1]/a[0],error_max=a[2],delta_mean=a[3]/a[0],
               delta_rgb=[s/a[0] for s in a[4:7]],exact_rgb=a[7]/float(a[0])) for g,a in stats.items() if a[0]}
report["samples"]=sample
report["worst"]=worst
if gpu_errors:
    report["gpu"]={
        g:dict(pixels=a[0],mean_vs_unquantized_cpu=a[1]/a[0],max_vs_unquantized_cpu=a[2],
               mean_vs_native=a[3]/a[0],max_vs_native=a[4],exact_native_rgb=a[5]/float(a[0]))
        for g,a in gpu_errors.items()
    }
if stats["sky_geometry_edge"][0]:
    report["sky_geometry_edge"]["bilinear_error_mean"]=edge_bilinear_error/stats["sky_geometry_edge"][0]
print(json.dumps(report,indent=2))
with open(os.path.join(root,"overlay-ab-report.json"),"w") as file: json.dump(report,file,indent=2)
