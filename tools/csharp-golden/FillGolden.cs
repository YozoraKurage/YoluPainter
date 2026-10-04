// 人工データだけで C# Core の塗りつぶし投影を評価する。計測は入力・ミップ準備を含まない。
using System;
using System.IO;
using System.Diagnostics;
using System.Globalization;
using System.Threading.Tasks;
using Yozolab.YoluPainter.Core;
using Yozolab.YoluPainter.Core.MeshMaps;
using Yozolab.YoluPainter.Core.Shelf;
static class FillGolden
{
    static byte[] Picture(int w, int h, bool shape=false)
    {
        var d=new byte[w*h*4];
        for(int y=0;y<h;y++) for(int x=0;x<w;x++) {
            int i=(y*w+x)*4; d[i]=(byte)(x*37+y*11); d[i+1]=(byte)(x*13+y*43);d[i+2]=(byte)(x*19+y*7+61);
            d[i+3]=(byte)(shape ? (x*17+y*23)%256 : (x+y)%7==0 ? 0 : (x*3+y)%5==0 ? 128 : 255);
        } return d;
    }
    static BakedMeshMap[] Maps(int w,int h)
    {
        var pos=new ushort[w*h*3];var nor=new ushort[w*h*3];var pc=new byte[w*h];var nc=new byte[w*h];
        for(int y=0;y<h;y++) for(int x=0;x<w;x++) {
            int i=y*w+x;
            pos[i*3]=(ushort)((long)x*65535/Math.Max(1,w-1));pos[i*3+1]=(ushort)((long)y*65535/Math.Max(1,h-1));pos[i*3+2]=(ushort)((x*197+y*101+13000)%65536);
            nor[i*3]=(ushort)((x*311+y*17+4000)%65536);nor[i*3+1]=(ushort)((x*37+y*211+21000)%65536);nor[i*3+2]=(ushort)((x*71+y*97+1000)%65536);
            pc[i]=(byte)(x>w-4 && y>h-4 ? 0 : 1);nc[i]=(byte)(x==2 && y==3 ? 0 : pc[i]);
        }
        BakedMeshMap Make(MeshMapKind kind,ushort[] data,byte[] coverage) => new BakedMeshMap(new MeshMapProvenance(kind,2,"synthetic","synthetic",0,w,h,0,0,1,"","SnapshotWorld","StaticSnapshot","Self",new[]{-1.0,-1.0,-.5},new[]{1.0,1.0,.5}),data,coverage);
        return new[]{Make(MeshMapKind.WorldNormal,nor,nc),Make(MeshMapKind.Position,pos,pc)};
    }
    static FillProjection Projection(int mode,int variant)
    {
        var p=FillProjection.Default.WithMode((FillProjectionMode)mode).WithWrap((FillWrap)(variant%3));
        if(variant!=0) p=p.WithTiles(variant==3?40:variant==4?.25:2.3,variant==3?35:variant==4?.75:1.7).WithOffset(.17,-.23).WithRotation(variant==4?-90:27);
        p=p.WithBlendWidth(variant%3==0?0:variant%3==1?.3:1);
        p=p.WithPlacement(new ShapeVolume(GeneratorShape.Box,.1,-.2,0,variant==0?0:20,variant==0?0:-35,variant==0?0:10,1.6,1.8,.9,0));
        if(mode==5) p=p.WithCulling(variant==0?1:.3,variant==0?180:120,variant==0?1:.4);
        return p;
    }
    static GeneratorModelFrame Frame(int variant) => variant%2==0 ? GeneratorModelFrame.Identity : new GeneratorModelFrame(.13,-.07,.09,0,Math.Sin(Math.PI/12),0,Math.Cos(Math.PI/12));
    static FillImageSampler Sampler(int mode,int variant,int w,int h,BakedMeshMap[] maps)
    {
        int iw=variant==5?1:31,ih=variant==5?9:23;
        var chain=ImageMipChain.Build(ImageContent.FromPixels(Picture(iw,ih),iw,ih),variant==6?FillImageConversion.LinearToSrgb:FillImageConversion.None,variant==7);
        var shape=ImageMipChain.Build(ImageContent.FromPixels(Picture(13,17,true),13,17),FillImageConversion.None,false);
        var fallback=new Rgba32(17,81,203,149);var p=Projection(mode,variant);
        return mode==5 ? FillImageSampler.BindDecal(p,variant==7?null:chain,null,variant>=6?shape:null,fallback,w,h,maps,null,Frame(variant),null)
            : FillImageSampler.Bind(p,chain,fallback,w,h,maps,null,Frame(variant));
    }
    static byte[] Render(FillImageSampler sampler,int w,int h,int threads)
    {
        var data=new byte[w*h*4];Parallel.For(0,h,new ParallelOptions{MaxDegreeOfParallelism=threads},y=>sampler.FillRows(0,0,w,y,y+1,data,w));return data;
    }
    // デカールの ApplyDecalToValue（画素ごとの値へ箱・奥行き・向き・形の被覆をかける）。値は座標だけから決まる人工データ
    static Rgba32 Value(int x,int y) => new Rgba32((byte)(x*29+y*7+3),(byte)(x*5+y*31+90),(byte)(x*3+y*3+200),(byte)((x*11+y*13)%256));
    static byte[] RenderValues(FillImageSampler sampler,int w,int h,int threads)
    {
        var data=new byte[w*h*4];
        Parallel.For(0,h,new ParallelOptions{MaxDegreeOfParallelism=threads},y=>{for(int x=0;x<w;x++){var p=sampler.ApplyDecalToValue(x,y,Value(x,y));int o=(y*w+x)*4;data[o]=p.R;data[o+1]=p.G;data[o+2]=p.B;data[o+3]=p.A;}});
        return data;
    }
    static void Contracts(string output)
    {
        var lines=new System.Collections.Generic.List<string>();
        Action[] bad={
            ()=>FillProjection.Default.WithTiles(0,1),
            ()=>FillProjection.Default.WithTiles(double.NaN,1),
            ()=>FillProjection.Default.WithRotation(400),
            ()=>FillProjection.Default.WithOffset(2e4,0),
            ()=>FillProjection.Default.WithBlendWidth(1.5),
            ()=>FillProjection.Default.WithMode((FillProjectionMode)9),
            ()=>FillProjection.Default.WithWrap((FillWrap)9),
            ()=>FillProjection.Default.WithPlacement(FillProjection.DefaultPlacement.WithSize(0,1,1)),
            ()=>FillProjection.Default.WithCulling(1,90,1),
            ()=>FillProjection.DecalAt(FillProjection.DefaultPlacement).WithCulling(-.1,90,1),
            ()=>FillProjection.DecalAt(FillProjection.DefaultPlacement).WithCulling(1,181,1),
            ()=>new GeneratorModelFrame(0,0,0,0,0,0,0)
        };
        for(int i=0;i<bad.Length;i++) {
            bool refused=false;try{bad[i]();}catch(ArgumentOutOfRangeException){refused=true;}
            if(!refused)throw new Exception("不正設定を受け入れた: "+i);lines.Add("invalid-"+i+":refused");
        }
        var resources=new ProjectResources();
        var image=resources.Add("Synthetic",ImageContent.FromPixels(Picture(5,3),5,3),ResourceOrigin.None,ResourceColorSpace.Srgb,out _);
        foreach(long limit in new long[]{11,12}) {
            var doc=new PaintDocument(4,4,4);doc.ImageResources=resources;doc.FillImageCacheBudgetBytes=limit;
            var fill=doc.AddFillLayer("Fill",new System.Collections.Generic.Dictionary<PaintChannel,Rgba32>{{PaintChannel.Color,new Rgba32(1,2,3,4)}});
            bool refused=false;try{doc.SetFillImage(fill.Id,PaintChannel.Color,image.Id);}catch(ResourceRefusedException e){if(e.Refusal!=ResourceRefusal.OverBudget)throw;refused=true;}
            lines.Add("mip-budget-"+limit+":"+(refused?"refused":"accepted"));
        }
        lines.Add("mip-bytes:"+ImageMipChain.ExtraBytes(5,3));
        File.WriteAllLines(Path.Combine(output,"contracts.txt"),lines);
    }
    static void Main(string[] args)
    {
        CultureInfo.CurrentCulture=CultureInfo.InvariantCulture;
        bool bench=args[0]=="bench";int w=bench?4096:37,h=bench?4096:29;var maps=Maps(w,h);
        int threads=bench?int.Parse(args[2]):1;CoreParallelism.MaxDegreeOfParallelism=threads;
        if(bench) {
            int runs=int.Parse(args[1]);
            for(int mode=0;mode<6;mode++) {
                var sampler=Sampler(mode,1,w,h,maps);Render(sampler,w,h,threads);
                for(int r=0;r<runs;r++) {GC.Collect();var sw=Stopwatch.StartNew();var data=Render(sampler,w,h,threads);sw.Stop();Console.WriteLine("{0},{1},{2:F3},{3}",(FillProjectionMode)mode,r,sw.Elapsed.TotalMilliseconds,data[data.Length/2]);}
            }return;
        }
        Directory.CreateDirectory(args[1]);
        Contracts(args[1]);
        for(int mode=0;mode<6;mode++) for(int v=0;v<8;v++) {
            var sampler=Sampler(mode,v,w,h,maps);var one=Render(sampler,w,h,1);var many=Render(sampler,w,h,4);
            for(int i=0;i<one.Length;i++) if(one[i]!=many[i]) throw new Exception("並列度で異なる");
            File.WriteAllBytes(Path.Combine(args[1],mode+"-"+v+".rgba"),one);
        }
        for(int v=0;v<8;v++) {
            var sampler=Sampler(5,v,w,h,maps);var one=RenderValues(sampler,w,h,1);var many=RenderValues(sampler,w,h,4);
            for(int i=0;i<one.Length;i++) if(one[i]!=many[i]) throw new Exception("デカールの値で並列度により異なる");
            File.WriteAllBytes(Path.Combine(args[1],"decal-values-"+v+".rgba"),one);
        }
        Console.WriteLine("48 事例とデカールの値 8 事例を書き出し、C# 並列度 1/4 の全バイト一致を確認");
    }
}
