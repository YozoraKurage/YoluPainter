// 合成データだけで C# の実 FilterEngine を走らせる。式の写しを正解にしない。
using System;
using System.IO;
using System.Diagnostics;
using System.Globalization;
using System.Collections.Generic;
using System.IO.Compression;
using System.Text;
using Yozolab.YoluPainter.Core;
using Yozolab.YoluPainter.Core.MeshMaps;
// Generator 段の事例は、実 FilterEngine が解決した値（BoundGenerator.TryValue とランプの評価）を段ごとに <名前>.s<段> へ書く。
// 1 画素 9 バイト: 種類（0 = 値なし、1 = スカラー、2 = ランプ適用後の RGBA）+ 8 バイト（f64 または RGBA と 0 詰め）。
// Rust 側はその値を入力にして evaluate し、実 FilterEngine の出力（<名前>.rgba）と全バイトで比べる。
// 照合の外はマップからの値の計算（Rust へ移していない）。
class GoldenInputs : IGeneratorInputs
{
    readonly Dictionary<MeshMapKind,BakedMeshMap> maps=new Dictionary<MeshMapKind,BakedMeshMap>();
    public GoldenInputs(int w,int h) {
        maps[MeshMapKind.Thickness]=FilterGolden.MakeMap(MeshMapKind.Thickness,w,h,(x,y,c)=>((x*13+y*29+7)%101)/100.0,(x,y)=>(x*3+y*5)%17==0?MeshTexelCoverage.Empty:MeshTexelCoverage.Covered);
        maps[MeshMapKind.Position]=FilterGolden.MakeMap(MeshMapKind.Position,w,h,(x,y,c)=>c==0?x/(double)Math.Max(1,w-1):c==1?y/(double)Math.Max(1,h-1):((x*5+y*3)%64)/63.0,(x,y)=>(x*7+y*2)%19==0?MeshTexelCoverage.Empty:MeshTexelCoverage.Covered);
    }
    public long Revision { get { return 1; } }
    public bool TryGetMap(MeshMapKind kind,out BakedMeshMap map,out string reason) { reason=maps.ContainsKey(kind)?null:kind+" はない"; return maps.TryGetValue(kind,out map); }
}
class FilterGolden
{
    static double D(string s) => double.Parse(s,CultureInfo.InvariantCulture);
    static FilterSettings Settings(string[] p) {
        switch(p[1]) {
        case "blur": return FilterSettings.GaussianBlur(int.Parse(p[2]));
        case "sharpen": return FilterSettings.Sharpen(int.Parse(p[2]),D(p[3]),int.Parse(p[4]));
        case "noise": return FilterSettings.Noise(D(p[2]),int.Parse(p[3]),p[4]=="1");
        case "levels": return FilterSettings.Levels(D(p[2]),D(p[3]),D(p[4]),D(p[5]),D(p[6]));
        case "invert": return FilterSettings.Invert();
        case "normalize": return FilterSettings.Normalize();
        case "generator": return FilterSettings.FromGenerator(Generator(p));
        default: throw new Exception("種類が不正");
        }
    }
    // generator <blend> <thick|shape> <ramp|none>。shape だけランプを付けられる（C# の GeneratorSettings の決まり）
    static GeneratorSettings Generator(string[] p) {
        var blend=(GeneratorBlend)Enum.Parse(typeof(GeneratorBlend),p[2],true);
        if(p[3]=="thick") return GeneratorSettings.Default(GeneratorType.Thickness).WithLevels(.1,.85,.4).WithBlend(blend);
        var g=GeneratorSettings.Default(GeneratorType.ShapeGradient).WithVolume(new ShapeVolume(GeneratorShape.Sphere,.5,.5,.5,0,0,0,.9,.9,.9,.6)).WithBlend(blend);
        return p[4]=="ramp"?g.WithRamp(new GradientRamp(
            new[]{new GradientStop(0,new Rgba32(200,30,20,255)),new GradientStop(.45,new Rgba32(20,180,90,255),.3),new GradientStop(1,new Rgba32(30,60,230,255))},
            new[]{new GradientOpacityStop(0,.25),new GradientOpacityStop(.6,1,.7),new GradientOpacityStop(1,.5)},
            new[]{new GradientCurvePoint(0,0),new GradientCurvePoint(.5,.3),new GradientCurvePoint(1,1)})):g;
    }
    // 値を式で決めた合成のマップ。Unity 版の試験（TestMeshMaps）と同じく、保存形式のバイト列に書いてから公開の読み手で読む
    internal static BakedMeshMap MakeMap(MeshMapKind kind,int width,int height,Func<int,int,int,double> value,Func<int,int,MeshTexelCoverage> coverage) {
        int channels=BakedMeshMap.ChannelCount(kind); long texels=(long)width*height; var raw=new byte[texels*(1+2*channels)];
        for(int y=0;y<height;y++)for(int x=0;x<width;x++)raw[(long)y*width+x]=(byte)coverage(x,y);
        for(int c=0;c<channels;c++) {
            long high=texels*(1+2*c),low=high+texels;
            for(int y=0;y<height;y++) {
                int previous=0;
                for(int x=0;x<width;x++) {
                    long i=(long)y*width+x; int v=(int)Math.Round(Math.Max(0,Math.Min(1,value(x,y,c)))*65535);
                    if(raw[i]==(byte)MeshTexelCoverage.Empty)v=0;
                    int delta=(v-previous)&0xFFFF; previous=v; raw[high+i]=(byte)(delta>>8); raw[low+i]=(byte)delta;
                }
            }
        }
        byte[] payload;
        using(var output=new MemoryStream()) { using(var deflate=new DeflateStream(output,CompressionLevel.Fastest,true))deflate.Write(raw,0,raw.Length); payload=output.ToArray(); }
        using(var stream=new MemoryStream()) {
            using(var w=new BinaryWriter(stream,new UTF8Encoding(false),true)) {
                Action<string> str=t=>{var b=Encoding.UTF8.GetBytes(t);w.Write(b.Length);w.Write(b);};
                w.Write(Encoding.ASCII.GetBytes("YLPMMAP\0")); w.Write(2); w.Write((int)kind); w.Write(MeshBaker.EngineVersion);
                str("mesh-"+width); str("topology"); w.Write(0); w.Write(width); w.Write(height); w.Write(0); w.Write(4); w.Write(1); w.Write(channels);
                str("test"); str(MeshBaker.Space); str(MeshBaker.Pose); str(MeshBaker.Source);
                for(int i=0;i<3;i++)w.Write(0.0); for(int i=0;i<3;i++)w.Write(1.0);
                w.Write(payload.Length); w.Write(payload);
            }
            return MeshMapBinary.Read(stream.ToArray());
        }
    }
    static string[] Words(string s) => s.Split(new[]{' '},StringSplitOptions.RemoveEmptyEntries);
    static PaintDocument Build(string line,out PaintLayer layer,out bool mask,out PaintChannel channel) {
        var all=Words(line); int w=int.Parse(all[2]),h=int.Parse(all[3]); mask=all[1]=="mask";
        channel=all[1]=="normal"?PaintChannel.Normal:all[1]=="scalar"?PaintChannel.Height:PaintChannel.Color;
        var d=new PaintDocument(w,h,128);d.FilterWorkingBudgetBytes=1024L*1024*1024;
        if(line.Contains(" generator "))d.GeneratorInputs=new GoldenInputs(w,h);
        layer=d.AddLayer("合成入力"); var s=mask?d.AddLayerMask(layer.Id).Surface:layer.GetChannel(channel);
        for(int y=0;y<h;y++)for(int x=0;x<w;x++) {
            byte r=(byte)((x*17+y*31+3)%256),g=(byte)((x*7+y*13+71)%256),b=(byte)((x*43+y*5+191)%256);
            byte a=(x+y)%7==0?(byte)0:(x+y)%5==0?(byte)255:(byte)((x*19+y*23+41)%256);
            if(all[1]=="scalar")g=b=r;
            s.SetPixel(x,y,new Rgba32(r,g,b,a));
        }
        string stages=string.Join(" ",all,4,all.Length-4);
        foreach(var stage in stages.Split(';')) {var p=Words(stage);d.AddFilter(layer.Id,mask?FilterTarget.Mask:FilterTarget.Content,Settings(p),mask?null:new[]{channel},strength:D(p[0]));}
        d.ClearHistory();return d;
    }
    static FilterEngine.Source SourceOf(PaintLayer l,bool mask,PaintChannel channel) => mask?FilterEngine.MaskSource(l.Mask):FilterEngine.ContentSource(l,channel);
    static byte[] Eval(PaintDocument d,PaintLayer l,bool mask,PaintChannel channel) {
        var source=SourceOf(l,mask,channel);
        var engine=new FilterEngine(d);var output=new byte[d.Width*d.Height*4];
        // 実Coreのブロック並列経路を呼ぶ。原文は変更せず非公開メソッドを反射で束縛する。
        var method=typeof(FilterEngine).GetMethod("EvaluateRects",System.Reflection.BindingFlags.Instance|System.Reflection.BindingFlags.NonPublic);
        var rects=new List<FilterEngine.Rect>();
        for(int y=0;y<d.Height;y+=256)for(int x=0;x<d.Width;x+=256)
            rects.Add(new FilterEngine.Rect(x,y,Math.Min(d.Width,x+256),Math.Min(d.Height,y+256)));
        for(int start=0;start<rects.Count;start+=4) {
            var batch=rects.GetRange(start,Math.Min(4,rects.Count-start));
            var buffers=(byte[][])method.Invoke(engine,new object[]{source,source.Chain.Length,batch});
            for(int k=0;k<batch.Count;k++) {
                var r=batch[k];for(int j=0;j<r.H;j++)Buffer.BlockCopy(buffers[k],j*r.W*4,output,((r.Y0+j)*d.Width+r.X0)*4,r.W*4);
            }
        }
        if(mask)for(int i=0;i<output.Length;i+=4){output[i+3]=output[i];output[i]=output[i+1]=output[i+2]=0;}
        return output;
    }
    // BoundGenerator.Combine だけを実物にした 7 モード × 色/マスク × 強さの密な照合。外側（透明画素を飛ばす・マスクの変換）はここの写しなので、
    // 実 Core を通した照合は cases.txt の generator の事例（SampleGolden）が担う。
    static void CombineGolden(string path) {
        using(var f=File.Create(Path.Combine(path,"generator-combine.rgba")))
        foreach(GeneratorBlend blend in Enum.GetValues(typeof(GeneratorBlend)))
        foreach(bool mask in new[]{false,true})
        foreach(double t in new[]{0.01,0.37,0.5,1.0})
        for(int x=0;x<256;x++) {
            double v=(x%7)/6.0; byte[] p={(byte)x,(byte)(255-x),(byte)((x*17)%256),(byte)x};
            if(mask) {byte hide=(byte)(255-MathUtil.ToByte(BoundGenerator.Combine(blend,1-p[3]/255.0,v,t)));p=new byte[]{0,0,0,hide};}
            else if(p[3]>0)for(int c=0;c<3;c++)p[c]=MathUtil.ToByte(BoundGenerator.Combine(blend,p[c]/255.0,v,t));
            f.Write(p,0,4);
        }
    }
    // 実 FilterEngine が使った Generator の値（段 k の BoundGenerator と、ランプがあればその評価）を全画素書く
    static void SampleGolden(string path,string name,PaintDocument d,PaintLayer l,bool mask,PaintChannel channel) {
        var source=SourceOf(l,mask,channel); int kinds=0,mapped=0,shown=0;
        for(int k=0;k<source.Chain.Length;k++) {
            var g=source.Chain[k].Settings.Generator; if(g==null)continue;
            var bound=BoundGenerator.Bind(g,source.Maps,source.Frame,d.Width,d.Height,null,out var reason);
            if(bound==null)throw new Exception("Generator を束ねられない: "+reason);
            var bytes=new byte[d.Width*d.Height*9];
            for(int y=0;y<d.Height;y++)for(int x=0;x<d.Width;x++) {
                int i=(y*d.Width+x)*9;
                if(!bound.TryValue(x,y,out double v)) continue;
                if(g.Ramp==null){bytes[i]=1;Buffer.BlockCopy(BitConverter.GetBytes(v),0,bytes,i+1,8);if(v>0&&v<1)mapped++;}
                else{var m=g.Ramp.Evaluate(v,mask||(channel!=PaintChannel.Color&&channel!=PaintChannel.Emission));bytes[i]=2;bytes[i+1]=m.R;bytes[i+2]=m.G;bytes[i+3]=m.B;bytes[i+4]=m.A;if(m.A>0&&m.A<255)mapped++;}
                shown++;
            }
            kinds++;
            File.WriteAllBytes(Path.Combine(path,name+".s"+k),bytes);
            Console.Error.WriteLine("  "+name+".s"+k+" 値あり "+shown+" 画素、中間の値 "+mapped+" 画素");
            shown=mapped=0;
        }
    }
    static void Main(string[] args) {
        CoreParallelism.MaxDegreeOfParallelism=4;
        if(args[0]=="filter-bench") {
            Console.WriteLine("C# Mono / 4096² / block=256 / degree=4 / warmup=1 / samples=3");
            foreach(string spec in new[]{"blur 8","sharpen 3 1.7 12","noise 0.47 -39 1","noise 0.83 39 0","levels 0.1 0.91 1.7 0.05 0.93","invert","normalize"}) {
                var d=Build("bench color 4096 4096 1 "+spec,out var l,out var mask,out var c);
                Eval(d,l,mask,c);var times=new List<double>();long sum=0;
                for(int i=0;i<3;i++){var sw=Stopwatch.StartNew();var b=Eval(d,l,mask,c);sw.Stop();times.Add(sw.Elapsed.TotalMilliseconds);sum+=b[b.Length/2];}
                times.Sort();Console.WriteLine(spec+"\t"+times[1].ToString("F3",CultureInfo.InvariantCulture)+"\t"+sum);
            }
        } else {
            Directory.CreateDirectory(args[2]);var index=new List<string>{"# "+Environment.GetEnvironmentVariable("GOLDEN_SOURCE")};
            foreach(string line in File.ReadAllLines(args[1])) {
                if(line.Trim().Length==0||line.StartsWith("#"))continue;
                var d=Build(line,out var l,out var mask,out var c);var b=Eval(d,l,mask,c);string name=Words(line)[0];
                File.WriteAllBytes(Path.Combine(args[2],name+".rgba"),b);index.Add(name+" "+b.Length);
                if(line.Contains(" generator "))SampleGolden(args[2],name,d,l,mask,c);
            }
            CombineGolden(args[2]);
            index.Add("generator-combine 57344");
            File.WriteAllLines(Path.Combine(args[2],"index.txt"),index);
        }
    }
}
