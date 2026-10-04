// 正本の Core を変更せず、合成した画像で Generator・ランプ・Anchor を照合する。
using System;
using System.IO;
using System.Linq;
using System.Collections.Generic;
using System.Diagnostics;
using System.Globalization;
using System.Reflection;
using System.Security.Cryptography;
using Yozolab.YoluPainter.Core;
using Yozolab.YoluPainter.Core.MeshMaps;
class GeneratorGolden
{
    static readonly MethodInfo Generate=typeof(FilterEngine).GetMethod("Generate",BindingFlags.Instance|BindingFlags.NonPublic);
    static GradientRamp Ramp()=>new GradientRamp(new[]
    {
        new GradientStop(.1,new Rgba32(231,19,47,0),.17),new GradientStop(.57,new Rgba32(11,207,59,255),.81),new GradientStop(.94,new Rgba32(29,43,249,64))
    }
    ,new[]
    {
        new GradientOpacityStop(0,.9,.24),new GradientOpacityStop(.63,0,.73),new GradientOpacityStop(1,.7)
    }
    ,new[]
    {
        new GradientCurvePoint(0,.1),new GradientCurvePoint(.23,.9),new GradientCurvePoint(.61,.2),new GradientCurvePoint(1,1)
    }
    );
    static ushort Raw(int x,int y,int c,MeshMapKind k)=>(ushort)(((long)x*1193+(long)y*3571+c*13451+(int)k*7919)%65536);
    static int Id(int x,int y)
    {
        var d=new[]
        {
            Raw(x,y,0,MeshMapKind.Id),Raw(x,y,1,MeshMapKind.Id),Raw(x,y,2,MeshMapKind.Id)
        };
        return IdMapColors.Rgb(d,0);
    }
    static GeneratorSettings Settings(GeneratorType kind,int v)
    {
        var s=GeneratorSettings.Default(kind).WithLevels(.07,.89,.37).WithInvert(v%2==1).WithNoise(v%3==0?0:.73,.071,v%2==0?int.MinValue:19381,v%3==1?GeneratorNoiseSpace.Uv:GeneratorNoiseSpace.Model).WithBlend((GeneratorBlend)(v%7));
        if(kind==GeneratorType.Dirt)s=s.WithBalance(new[]
        {
            0,.3,1
        }
        [v%3]);
        if(kind==GeneratorType.PositionGradient)s=s.WithAxis(v%3);
        if(kind==GeneratorType.Direction)s=s.WithDirection(.31,-.71,.19).WithBentNormal(v%2==1);
        if(kind==GeneratorType.ShapeGradient)
        {
            s=s.WithVolume(new ShapeVolume((GeneratorShape)(v%3),.2,-.1,.4,17,-31,43,2.3,3.1,4.7,v%2==0?0:.63));
            if(v>=3)s=s.WithRamp(Ramp());
        }
        if(kind==GeneratorType.IdColor)s=s.WithIdColors(new[]
        {
            Id(1,1),Id(3,4),0xabc123
        }
        ).WithIdTolerance(v%2==0?8:43);
        return s;
    }
    static BakedMeshMap[] Maps(GeneratorSettings g,int w,int h)
    {
        var maps=new BakedMeshMap[10];
        foreach(var k in g.UsedMaps)
        {
            int channels=BakedMeshMap.ChannelCount(k);
            var data=new ushort[w*h*channels];
            var coverage=new byte[w*h];
            for(int y=0;y<h;y++)for(int x=0;x<w;x++)
            {
                for(int c=0;c<channels;c++)data[(y*w+x)*channels+c]=Raw(x,y,c,k);
                coverage[y*w+x]=(byte)((x+3*y+(int)k)%17==0?0:1+x%2);
            }
            var p=new MeshMapProvenance(k,1,"synthetic","synthetic",0,w,h,-1,0,1,"test","SnapshotWorld","StaticSnapshot","Self",new[]
            {
                -1.0,-2.0,-3.0
            }
            ,new[]
            {
                2.0,3.0,1.0
            }
            );
            maps[(int)k]=new BakedMeshMap(p,data,coverage);
        }
        return maps;
    }
    static GeneratorModelFrame Frame()=>new GeneratorModelFrame(.13,-.27,.41,.17,-.31,.23,.89);
    static byte[] Pixels(int w,int h,int target)
    {
        var bytes=new byte[w*h*4];
        for(int y=0;y<h;y++)for(int x=0;x<w;x++)
        {
            int i=(y*w+x)*4;
            bytes[i]=(byte)((x*17+y*31+3)%256);
            bytes[i+1]=(byte)((x*7+y*13+71)%256);
            bytes[i+2]=(byte)((x*43+y*5+191)%256);
            bytes[i+3]=(byte)((x+y)%7==0?0:(x+y)%5==0?255:(x*19+y*23+41)%256);
            if(target==1)bytes[i+1]=bytes[i+2]=bytes[i];
            if(target==2)bytes[i]=bytes[i+1]=bytes[i+2]=0;
        }
        return bytes;
    }
    static AnchorSample Anchor(int v,int w,int h)
    {
        int tile=Math.Max(w,h);
        var data=new byte[tile*tile*4];
        var pixels=Pixels(w,h,0);
        for(int y=0;y<h;y++)Buffer.BlockCopy(pixels,y*w*4,data,y*tile*4,w*4);
        if(v<6)return AnchorSample.ForLayer(tile,0,0,1,1,new[]
        {
            data
        }
        ,v%3==1?PaintChannel.Height:PaintChannel.Color,v%3==2?AnchorRead.Coverage:AnchorRead.Value);
        var d=new PaintDocument(w,h);
        var l=d.AddLayer("mask");
        var mask=d.AddLayerMask(l.Id);
        d.SetLayerMaskEnabled(l.Id,v%4!=0);
        d.SetLayerMaskInverted(l.Id,v%2==1);
        d.SetLayerMaskDensity(l.Id,.63);
        return AnchorSample.ForMask(tile,0,0,1,1,new[]
        {
            data
        }
        ,mask);
    }
    static Func<byte[]> Prepare(GeneratorType kind,int v,int target,int w,int h)
    {
        var g=Settings(kind,v);
        var maps=Maps(g,w,h);
        var anchor=kind==GeneratorType.Anchor?Anchor(v,w,h):null;
        var d=new PaintDocument(w,h);
        var engine=new FilterEngine(d);
        var source=new FilterEngine.Source
        {
            Maps=maps,Frame=Frame(),Mask=target==2,Channel=target==1?PaintChannel.Height:PaintChannel.Color
        };
        var input=Pixels(w,h,target);
        return ()=>
        {
            var bytes=(byte[])input.Clone();
            if(target==2)for(int i=0;i<bytes.Length;i+=4)
            {
                bytes[i]=bytes[i+1]=bytes[i+2]=bytes[i+3];
                bytes[i+3]=255;
            }
            Generate.Invoke(engine,new object[]
            {
                source,bytes,new FilterEngine.Rect(0,0,w,h),g,v%2==0?1:.43,anchor
            }
            );
            if(target==2)for(int i=0;i<bytes.Length;i+=4)
            {
                bytes[i+3]=bytes[i];
                bytes[i]=bytes[i+1]=bytes[i+2]=0;
            }
            return bytes;
        };
    }
    static byte[] AnchorScene(int v)
    {
        const int w=41,h=29;
        var d=new PaintDocument(w,h,64);
        var bottom=d.AddLayer("bottom");
        var input=Pixels(w,h,0);
        for(int y=0;y<h;y++)for(int x=0;x<w;x++)
        {
            int i=(y*w+x)*4;
            bottom.GetChannel(PaintChannel.Color).SetPixel(x,y,new Rgba32(input[i],input[i+1],input[i+2],input[i+3]));
        }
        PaintLayer Fill(string name,Rgba32 c)=>d.AddFillLayer(name,new Dictionary<PaintChannel,Rgba32>
        {
            {
                PaintChannel.Color,c
            }
        }
        );
        var a=v>=24?d.AddAdjustmentLayer("a",AdjustmentSettings.Invert()):Fill("a",new Rgba32(200,60,20,173));
        var b=Fill("b",new Rgba32(20,180,230,121));
        var top=Fill("top",new Rgba32(245,7,191,231));
        var group=d.GroupLayers(new[]
        {
            a.Id,b.Id
        }
        ,"group");
        d.SetLayerBlendMode(group.Id,v%2==0?LayerBlendMode.PassThrough:LayerBlendMode.Normal);
        d.SetLayerOpacity(group.Id,.23);
        d.SetLayerVisibility(group.Id,v%7!=0);
        d.SetLayerClipping(group.Id,v%4==0);
        d.SetLayerOpacity(a.Id,.67);
        d.SetLayerBlendMode(a.Id,LayerBlendMode.Multiply);
        d.SetLayerClipping(a.Id,v%5==0);
        d.SetLayerVisibility(a.Id,v%6!=0);
        d.SetLayerClipping(b.Id,v%3==0);
        d.SetLayerClipping(top.Id,true);
        var mask=d.AddLayerMask(a.Id);
        for(int y=0;y<h;y++)for(int x=0;x<w;x++)mask.Surface.SetPixel(x,y,new Rgba32(0,0,0,input[(y*w+x)*4+3]));
        d.SetLayerMaskDensity(a.Id,.63);
        d.SetLayerMaskInverted(a.Id,v%2==1);
        if(v>=12)
        {
            var outer=d.GroupLayers(new[]
            {
                group.Id,top.Id
            }
            ,"outer");
            d.SetLayerBlendMode(outer.Id,v%3==0?LayerBlendMode.Normal:LayerBlendMode.PassThrough);
            d.SetLayerOpacity(outer.Id,.31);
            d.SetLayerVisibility(outer.Id,false);
        }
        var host=new[]
        {
            a,b,group,top
        }
        [v%4];
        var tile=CpuCompositor.CompositeAnchorTiles(d,PaintChannel.Color,host,new[]
        {
            new TileCoord(0,0)
        }
        )[0];
        var output=new byte[w*h*4];
        if(tile!=null)for(int y=0;y<h;y++)Buffer.BlockCopy(tile,y*64*4,output,y*w*4,w*4);
        return output;
    }
    static string Hash(byte[] bytes)
    {
        using(var sha=SHA256.Create())return string.Concat(sha.ComputeHash(bytes).Select(b=>b.ToString("x2")));
    }
    static void Main(string[] args)
    {
        CultureInfo.CurrentCulture=CultureInfo.InvariantCulture;
        CoreParallelism.MaxDegreeOfParallelism=int.Parse(Environment.GetEnvironmentVariable("GEN_THREADS")??"4");
        if(args[0]=="bench")
        {
            for(int kind=0;kind<11;kind++)
            {
                string name;
                Func<byte[]> run;
                if(kind<10)
                {
                    var k=(GeneratorType)(kind<8?kind:5);
                    int v=kind==5?5:kind==8?3:kind==9?4:2;
                    name=kind==8?"ShapeBoxRamp":kind==9?"ShapeSphereRamp":k.ToString();
                    run=Prepare(k,v,0,4096,4096);
                }
                else
                {
                    name="Ramp";
                    var r=Ramp();
                    run=()=>
                    {
                        var output=new byte[4096*4096*4];
                        System.Threading.Tasks.Parallel.For(0,4096,new System.Threading.Tasks.ParallelOptions
                        {
                            MaxDegreeOfParallelism=CoreParallelism.MaxDegreeOfParallelism
                        }
                        ,y=>
                        {
                            for(int x=0;x<4096;x++)
                            {
                                var p=r.Evaluate(((x+y*17)%65536)/65535.0);int i=(y*4096+x)*4;output[i]=p.R;output[i+1]=p.G;output[i+2]=p.B;output[i+3]=p.A;
                            }
                        }
                        );
                        return output;
                    }
                    ;
                }
                run();
                var sw=Stopwatch.StartNew();
                var b=run();
                sw.Stop();
                Console.WriteLine(name+" "+sw.Elapsed.TotalMilliseconds.ToString("F3")+" "+Hash(b));
                GC.Collect();
            }
            return;
        }
        Directory.CreateDirectory(args[1]);
        var lines=new List<string>
        {
            "# "+Environment.GetEnvironmentVariable("GOLDEN_SOURCE")
        };
        foreach(GeneratorType k in Enum.GetValues(typeof(GeneratorType)))for(int v=0;v<12;v++)for(int t=0;t<3;t++)
        {
            string name=((int)k)+"-"+v+"-"+t;
            var b=Prepare(k,v,t,41,29)();
            lines.Add(name+" "+Hash(b));
            File.WriteAllBytes(Path.Combine(args[1],name+".rgba"),b);
        }
        for(int scalar=0;scalar<2;scalar++)
        {
            var b=new List<byte>();
            for(int i=-10;i<=1010;i++)
            {
                var p=Ramp().Evaluate(i/1000.0,scalar==1);
                b.AddRange(new[]
                {
                    p.R,p.G,p.B,p.A
                }
                );
            }
            lines.Add("ramp-"+scalar+" "+Hash(b.ToArray()));
            File.WriteAllBytes(Path.Combine(args[1],"ramp-"+scalar+".rgba"),b.ToArray());
        }
        for(int v=0;v<48;v++)
        {
            var b=AnchorScene(v);
            lines.Add("anchor-"+v+" "+Hash(b));
            File.WriteAllBytes(Path.Combine(args[1],"anchor-"+v+".rgba"),b);
        }
        File.WriteAllLines(Path.Combine(args[1],"index.txt"),lines);
        Console.WriteLine("Generator の照合: "+(lines.Count-1)+" 事例");
    }
}
