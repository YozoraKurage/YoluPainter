// 人工データを実 C# Core に通す。Rust 側 crates/yolu-core/tests/reference/docops_golden.rs と同じ台本・出力順。
using System;
using System.IO;
using System.Linq;
using System.Collections.Generic;
using System.Globalization;
using System.Diagnostics;
using Yozolab.YoluPainter.Core;
static class DocOpsGolden
{
    static Rgba32 Pattern(int x,int y,int c,int seed) {
        int n=x*17+y*31+c*13+seed*7;
        return new Rgba32((byte)(n*3+1),(byte)(n*5+2),(byte)(n*7+3),(byte)(n%5==0?0:n%5==1?255:n%256));
    }
    static PaintDocument Make(int seed) {
        var d=new PaintDocument(17,13,new[]{4,8,16}[seed%3],256L<<20);
        for(int l=0;l<2;l++) {
            var a=d.AddLayer("layer"+l);
            foreach(PaintChannel c in Enum.GetValues(typeof(PaintChannel))) {
                var s=a.GetChannel(c);
                for(int y=0;y<13;y++) for(int x=0;x<17;x++) if((x+y+seed+l)%4!=0) s.SetPixel(x,y,Pattern(x,y,(int)c,seed+l));
            }
            var m=d.AddLayerMask(a.Id);
            for(int y=0;y<13;y++) for(int x=0;x<17;x++) if((x+y)%5==0) m.Surface.SetPixel(x,y,new Rgba32(0,0,0,(byte)(x*11+y*7)));
        }
        d.ClearHistory(); return d;
    }
    static void Pixel(BinaryWriter b,Rgba32 p) { b.Write(p.R);b.Write(p.G);b.Write(p.B);b.Write(p.A); }
    static void Snapshot(BinaryWriter b,PaintDocument d) {
        b.Write(d.Width);b.Write(d.Height);b.Write(d.Layers.Count);b.Write(d.NormalSettings.Strength);
        b.Write(d.Selection!=null);if(d.Selection!=null)for(int y=0;y<d.Height;y++)for(int x=0;x<d.Width;x++)b.Write(d.Selection[x,y]);
        foreach(var l in d.Layers) {
            b.Write((int)l.Kind); b.Write(l.ParentId==Guid.Empty?-1:d.Layers.ToList().FindIndex(p=>p.Id==l.ParentId));
            b.Write(l.Visible);b.Write(l.Opacity);b.Write((int)l.BlendMode);b.Write(l.Clipping);b.Write((int)l.Locks);
            foreach(PaintChannel c in Enum.GetValues(typeof(PaintChannel))) {
                b.Write(l.IsChannelEnabled(c)); b.Write(l.Channels.ContainsKey(c)); b.Write((int)l.BlendModeIn(c));b.Write(l.OpacityIn(c));
                for(int y=0;y<d.Height;y++) for(int x=0;x<d.Width;x++) Pixel(b,l.GetPixel(c,x,y));
            }
            b.Write(l.Mask!=null);
            if(l.Mask!=null) {
                b.Write(l.Mask.Enabled);b.Write(l.Mask.Inverted);b.Write(l.Mask.Density);
                for(int y=0;y<d.Height;y++) for(int x=0;x<d.Width;x++) b.Write(l.Mask.Surface.GetPixel(x,y).A);
            }
        }
        foreach(PaintChannel c in Enum.GetValues(typeof(PaintChannel))) b.Write(d.Composite(c));
    }
    static Affine2D Transform(int n) {
        switch(n) {
            case 0:return Affine2D.Translation(3,-2);
            case 1:return Affine2D.Translation(.25,-.75);
            case 2:return Affine2D.FromParts(8.5,6.5,0,0,90,1,1);
            case 3:return Affine2D.FromParts(8.5,6.5,0,0,180,1,1);
            case 4:return Affine2D.FromParts(8.5,6.5,0,0,0,-1,1);
            case 5:return Affine2D.FromParts(8.5,6.5,1,-1,23,1.3,.7);
            case 6:return Affine2D.FromParts(0,0,0,0,0,2,2);
            default:return Affine2D.FromParts(8,6,0,0,-31,.5,1.7);
        }
    }
    static void Report(BinaryWriter b,LayerMergeReport r) {
        b.Write((int)r.Method);b.Write((int)r.Notes);b.Write(r.ComparedPixels);b.Write(r.ChangedPixels);b.Write(r.MaxDifference);b.Write(r.MaxVisibleDifference);
        foreach(PaintChannel c in Enum.GetValues(typeof(PaintChannel))) b.Write(r.ChangedByChannel.TryGetValue(c,out var n)?n:0L);
    }
    /// 結合がロックで断られる・断られないの照合。結果の型（0 成功・1 ロック・2 そのほかの拒否）、断った層・持ち主・ロック、断ったあとの文書（変わらない）を書く。
    static void MergeLock(PaintDocument d,int n,BinaryWriter output) {
        var a=d.Layers[0].Id;var b=d.Layers[1].Id;PaintLayer g=null;Func<LayerMergeReport> act=null;
        switch(n) {
            case 0:d.SetLayerLocks(b,LayerLocks.Pixels);act=()=>d.MergeDown(b,255);break;
            case 1:d.SetLayerLocks(b,LayerLocks.All);act=()=>d.MergeDown(b,255);break;
            case 2:d.SetLayerLocks(a,LayerLocks.Pixels);act=()=>d.MergeDown(b,255);break;
            case 3:d.SetLayerLocks(a,LayerLocks.All);act=()=>d.MergeDown(b,255);break;
            case 4:d.SetLayerLocks(a,LayerLocks.Transparency);act=()=>d.MergeDown(b,255);break;
            case 5:d.SetLayerLocks(b,LayerLocks.Transparency);act=()=>d.MergeDown(b,255);break;
            case 6:d.SetLayerLocks(a,LayerLocks.Position);act=()=>d.MergeDown(b,255);break;
            case 7:d.SetLayerClipping(b,true);d.SetLayerLocks(a,LayerLocks.Transparency);act=()=>d.MergeDown(b,255);break;
            case 8:g=d.GroupLayers(new[]{a,b},"g");d.SetLayerLocks(g.Id,LayerLocks.All);act=()=>d.MergeDown(b,255);break;
            case 9:g=d.GroupLayers(new[]{a,b},"g");d.SetLayerLocks(g.Id,LayerLocks.Pixels);act=()=>d.MergeDown(b,255);break;
            case 10:g=d.GroupLayers(new[]{a,b},"g");d.SetLayerLocks(g.Id,LayerLocks.Transparency);act=()=>d.MergeDown(b,255);break;
            case 11:d.SetLayerLocks(b,LayerLocks.Pixels);act=()=>d.MergeVisible("merged",tolerance:255);break;
            case 12:d.SetLayerLocks(b,LayerLocks.Pixels);d.SetLayerVisibility(b,false);act=()=>d.MergeVisible("merged",tolerance:255);break;
            case 13:g=d.GroupLayers(new[]{a,b},"g");d.SetLayerLocks(g.Id,LayerLocks.All);act=()=>d.MergeVisible("merged",tolerance:255);break;
            case 14:d.SetLayerLocks(b,LayerLocks.Transparency);act=()=>d.MergeVisible("merged",tolerance:255);break;
            case 15:d.SetLayerLocks(a,LayerLocks.Pixels);act=()=>d.MergeLayers(new[]{a,b},255);break;
            case 16:d.SetLayerLocks(b,LayerLocks.All);act=()=>d.MergeLayers(new[]{a,b},255);break;
            case 17:g=d.GroupLayers(new[]{a,b},"g");d.SetLayerLocks(g.Id,LayerLocks.All);act=()=>d.MergeLayers(new[]{a,b},255);break;
            case 18:d.SetLayerLocks(a,LayerLocks.Transparency);act=()=>d.MergeLayers(new[]{a,b},255);break;
            case 19:g=d.GroupLayers(new[]{a,b},"g");d.SetLayerLocks(g.Id,LayerLocks.Pixels);act=()=>d.MergeGroup(g.Id,255);break;
            case 20:g=d.GroupLayers(new[]{a,b},"g");d.SetLayerLocks(a,LayerLocks.All);act=()=>d.MergeGroup(g.Id,255);break;
            case 21:g=d.GroupLayers(new[]{a,b},"g");d.SetLayerLocks(g.Id,LayerLocks.Transparency);act=()=>d.MergeGroup(g.Id,255);break;
            case 22:g=d.GroupLayers(new[]{a,b},"g");d.SetLayerLocks(g.Id,LayerLocks.Position);act=()=>d.MergeGroup(g.Id,255);break;
            case 23:act=()=>d.MergeGroup(a,255);break;
            case 24:g=d.AddGroup("empty");act=()=>d.MergeGroup(g.Id,255);break;
            case 25:g=d.GroupLayers(new[]{b},"g");act=()=>d.MergeLayers(new[]{a,b},255);break;
            case 26:d.SetLayerVisibility(b,false);act=()=>d.MergeLayers(new[]{a,b},255);break;
            case 27:g=d.GroupLayers(new[]{a},"g");act=()=>d.MergeDown(g.Id,255);break;
            case 28:g=d.GroupLayers(new[]{a},"g");act=()=>d.MergeDown(b,255);break;
            case 29:d.SetLayerVisibility(b,false);act=()=>d.MergeDown(b,255);break;
            case 30:d.SetLayerVisibility(a,false);d.SetLayerVisibility(b,false);act=()=>d.MergeVisible("merged",tolerance:255);break;
            case 31:act=()=>d.MergeDown(a,255);break;
            case 32:d.SetLayerClipping(b,true);d.SetLayerLocks(b,LayerLocks.Pixels);act=()=>d.MergeDown(b,255);break;
            default:g=d.GroupLayers(new[]{a,b},"g");d.SetLayerClipping(b,true);d.SetLayerLocks(g.Id,LayerLocks.Transparency);act=()=>d.MergeDown(b,255);break;
        }
        d.ClearHistory();
        LayerMergeReport report=null;
        try { report=act(); output.Write((byte)0);Report(output,report); }
        catch(LayerLockedException e) { output.Write((byte)1);var all=d.Layers.ToList();output.Write(all.FindIndex(l=>l.Id==e.LayerId));output.Write(all.FindIndex(l=>l.Id==e.LockedBy));output.Write((int)e.Lock); }
        catch(LayerOpException e) { output.Write((byte)2);output.Write(e.Reason.ToString()); }
        output.Write(d.HistoryBytes);Snapshot(output,d);
        if(report!=null){d.Undo();output.Write(d.HistoryBytes);Snapshot(output,d);d.Redo();output.Write(d.HistoryBytes);Snapshot(output,d);}
    }
    static void Run(string op,int n,int seed,BinaryWriter output) {
        var d=Make(seed);var a=d.Layers[0].Id;var b=d.Layers[1].Id; LayerMergeReport report=null;
        if(op.StartsWith("selected_")) {
            op=op.Substring(9);var selection=SelectionMask.None(d);
            for(int y=2;y<11;y++)for(int x=3;x<14;x++)selection.Surface.SetPixel(x,y,new Rgba32(0,0,0,(byte)((x*17+y*29)%256)));
            d.SetSelection(selection);d.ClearHistory();
        }
        if(op=="region") { d.Transform(a,Transform(n%8),SelectionMask.Rectangle(d,5,1,16,8),Resampling.Bilinear); }
        else if(op=="fill") {
            if(n<4)d.SetLayerLocks(a,LayerLocks.Transparency);d.ClearHistory();
            d.Fill(a,PaintChannel.Color,new Rgba32(70,210,120,123),.65,n%2==0?null:SelectionMask.Rectangle(d,2,1,10,9),n>=6);
        }
        else if(op=="transform") { d.Transform(a,Transform(n%8),resampling:n<8?Resampling.Bilinear:Resampling.Nearest,includeMask:n%3!=0); }
        else if(op=="resize") {
            var sizes=new[]{(34,26),(8,6),(5,3),(19,11),(1,1),(17,26)}; var size=sizes[n%6];
            d=d.Resampled(size.Item1,size.Item2,(CanvasResampling)(n/6)).Document; Snapshot(output,d);return;
        }
        else if(op=="bounds") {
            PaintLayer target=d.Layers[n%2==0?0:1]; SelectionMask region=null;
            if(n==2||n==3)region=SelectionMask.Rectangle(d,5,1,16,8);
            if(n==4)target=d.AddLayer("empty");
            if(n==5){target=d.AddLayer("maskonly");var m=d.AddLayerMask(target.Id);m.Surface.SetPixel(3,4,new Rgba32(0,0,0,200));m.Surface.SetPixel(9,10,new Rgba32(0,0,0,7));}
            if(n==6){target=d.AddLayer("clear");target.GetChannel(PaintChannel.Color).SetPixel(5,5,new Rgba32(10,20,30,0));}
            if(n==7){region=SelectionMask.None(d);for(int y=2;y<11;y++)for(int x=3;x<14;x++)region.Surface.SetPixel(x,y,new Rgba32(0,0,0,(byte)((x*17+y*29)%256)));}
            d.ClearHistory();
            var bounds=d.TransformBounds(target.Id,region);
            output.Write(bounds.HasValue);if(bounds.HasValue){output.Write(bounds.Value.x0);output.Write(bounds.Value.y0);output.Write(bounds.Value.x1);output.Write(bounds.Value.y1);}
            output.Write(d.HistoryBytes);Snapshot(output,d);return;
        }
        else if(op=="mergelock") { MergeLock(d,n,output);return; }
        else if(op=="effect") {
            d.SetLayerLocks(a,LayerLocks.Transparency);d.ClearHistory();
            var brush=new BrushSettings {Radius=4,Hardness=.6,Spacing=.2,Color=new Rgba32(200,90,30),Opacity=.7,Flow=.6,PressureSize=false,PressureOpacity=false,Effect=(BrushEffect)(n+1),BlurRadius=2,SmudgeStrength=.6,CloneOffsetX=-2.25,CloneOffsetY=.5};
            using(var stroke=d.BeginStroke(a,PaintChannel.Color,brush)){stroke.Add(new BrushSample(5,5));stroke.Add(new BrushSample(7,6));stroke.Add(new BrushSample(11,7));stroke.Commit();}
        }
        else if(op=="lock") {
            d.SetLayerLocks(a,LayerLocks.Transparency);d.ClearHistory();
            var brush=new BrushSettings { Radius=80,Hardness=n==0?1:.4,Color=new Rgba32(220,30,80,128),Opacity=.7,Flow=.8,PressureSize=false,PressureOpacity=false };
            using(var stroke=d.BeginStroke(a,PaintChannel.Color,brush)) { stroke.Add(new BrushSample(8,6));stroke.Add(new BrushSample(9,6));stroke.Commit(); }
        }
        else if(op=="multi") {
            var c=d.AddLayer("empty");var g=d.GroupLayers(new[]{a},"group");d.ClearHistory();
            switch(n) {
                case 0:d.DuplicateLayers(new[]{a,g.Id,c.Id});break;
                case 1:d.RemoveLayers(new[]{a,g.Id,c.Id});break;
                case 2:d.SetLayersVisibility(new[]{a,g.Id,b},false);break;
                case 3:d.MoveLayers(new[]{b,c.Id},g.Id,0);break;
                case 4:d.StepLayers(new[]{g.Id,b},true);break;
                case 5:d.ChangeLayerLocks(new[]{g.Id,b,b},LayerLocks.Position|LayerLocks.Pixels,true);break;
                default:d.TransformLayers(new[]{g.Id,b,a},Transform(5));break;
            }
        }
        else {
            if(n%8==0) d.SetLayerClipping(b,true);
            if(n%8==1) { d.SetLayerOpacity(a,.4);d.SetLayerBlendMode(a,LayerBlendMode.Multiply); }
            if(n%8==2) { var bg=d.AddFillLayer("background",new Dictionary<PaintChannel,Rgba32>{{PaintChannel.Color,new Rgba32(100,200,70)}}); d.MoveLayer(bg.Id,0); }
            if(n%8==3) d.SetLayerBlendMode(b,LayerBlendMode.Screen);
            if(n%8==4) d.SetChannelEnabled(a,PaintChannel.Normal,false);
            if(n%8==5) d.SetChannelEnabled(b,PaintChannel.Metallic,false);
            if(n%8==6) {d.SetLayerMaskInverted(a,true);d.SetLayerMaskDensity(a,.35);}
            if(n%8==7) {d.SetChannelBlend(a,PaintChannel.Color,new ChannelBlend(LayerBlendMode.Multiply,.65));d.SetLayerOpacity(b,.55);}
            if(op=="merge_down") {d.ClearHistory();report=d.MergeDown(b,255);}
            else if(op=="merge_visible") { var h=d.DuplicateLayer(a);d.SetLayerVisibility(h.Id,false);d.GroupLayers(new[]{a,h.Id},"group");d.ClearHistory();report=d.MergeVisible("merged",tolerance:255); }
            else if(op=="merge_group") {var g=d.GroupLayers(new[]{a,b},"group");d.SetLayerBlendMode(g.Id,LayerBlendMode.Multiply);d.ClearHistory();report=d.MergeGroup(g.Id,255);}
            else {d.ClearHistory();report=d.MergeLayers(new[]{a,b},255);}
        }
        if(report!=null) Report(output,report);
        output.Write(d.HistoryBytes);Snapshot(output,d);d.Undo();output.Write(d.HistoryBytes);Snapshot(output,d);d.Redo();output.Write(d.HistoryBytes);Snapshot(output,d);
    }
    static void Bench() {
        foreach(int degree in new[]{1,4}) { CoreParallelism.MaxDegreeOfParallelism=degree;
            foreach(int side in new[]{512,1024}) foreach(string op in new[]{"変形","面積縮小","下へ結合"}) {
                var times=new List<double>();long pixels=0,history=0;
                for(int repeat=0;repeat<3;repeat++) {
                    var d=new PaintDocument(side,side,128);PaintLayer last=null;
                    for(int l=0;l<2;l++) {last=d.AddLayer("人工層");var surface=last.GetChannel(PaintChannel.Color);
                        for(int ty=0;ty<side/128;ty++)for(int tx=0;tx<side/128;tx++) {
                            var bytes=new byte[128*128*4];for(int y=0;y<128;y++)for(int x=0;x<128;x++) {
                                int n=(tx*128+x)*17+(ty*128+y)*31+l*7,o=(y*128+x)*4;
                                bytes[o]=(byte)(n*3+1);bytes[o+1]=(byte)(n*5+2);bytes[o+2]=(byte)(n*7+3);bytes[o+3]=(byte)(n%5==0?0:128);
                            }surface.ImportTile(new TileCoord(tx,ty),bytes);
                        }
                    }d.ClearHistory();var watch=Stopwatch.StartNew();
                    if(op=="変形")d.Transform(last.Id,Affine2D.FromParts(side/2.0,side/2.0,1,-1,23,1.3,.7));
                    else if(op=="面積縮小")d=d.Resampled(side/2,side/2,CanvasResampling.Area).Document;
                    else d.MergeDown(last.Id,255);
                    watch.Stop();times.Add(watch.Elapsed.TotalMilliseconds);pixels=d.AllocatedBytes;history=d.HistoryBytes;
                }times.Sort();Console.WriteLine(side+"² "+op+" 並列度="+degree+" 中央値="+times[1].ToString("0.000",CultureInfo.InvariantCulture)+" ms 画素="+pixels+" B 履歴="+history+" B");
            }
        }
    }
    static int Main(string[] args) {
        try {
            if(args.Length==1 && args[0]=="bench"){Bench();return 0;}
            if(args.Length!=3 || args[0]!="golden")throw new ArgumentException("golden <台本> <出力>");
            Directory.CreateDirectory(args[2]);var index=new List<string>{"# "+Environment.GetEnvironmentVariable("GOLDEN_SOURCE")};var watch=Stopwatch.StartNew();
            foreach(var line in File.ReadLines(args[1])) {
                if(string.IsNullOrWhiteSpace(line)||line.StartsWith("#"))continue;var p=line.Split(' ');
                using(var stream=File.Create(Path.Combine(args[2],p[0]+".bin")))using(var b=new BinaryWriter(stream))Run(p[1],int.Parse(p[2]),int.Parse(p[3]),b);
                index.Add(line);
            }
            File.WriteAllLines(Path.Combine(args[2],"index.txt"),index);Console.WriteLine("照合用 "+(index.Count-1)+" 事例、"+watch.ElapsedMilliseconds+" ms");return 0;
        } catch(Exception e) {Console.Error.WriteLine(e);return 1;}
    }
}
