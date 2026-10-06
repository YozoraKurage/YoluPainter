// 層の画素のコピー・カット・結合してコピー・ペースト・置き換えと、複数の編集のまとめ（Batch）を実 C# Core に通す。
// Rust 側 crates/yolu-core/tests/reference/clipboard_golden.rs と同じ台本・出力順。事例ごとの出力（結果の型・写し・履歴のバイト数・文書の
// 全状態と、Undo・Redo の後の状態）の SHA-256 を index.txt に書く（全部を保存すると、同じ人工の文書の状態が事例の数だけ積まれて
// 大きくなる）。食い違ったときは環境変数 GOLDEN_FULL=フォルダ で、事例ごとの出力そのものを <事例>.bin に書いて Rust 側の出力
// （CLIPBOARD_GOLDEN_DUMP=フォルダ）と比べる。
using System;
using System.IO;
using System.Linq;
using System.Collections.Generic;
using System.Security.Cryptography;
using System.Text;
using Yozolab.YoluPainter.Core;
static class ClipboardGolden
{
    // ───────── 人工の文書（DocOpsGolden.cs と同じ） ─────────
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
    /// 貼り先の文書: 1 つの層（Color に人工の画素）。kind 0 = 写し元と同じ大きさ、1 = 小さい、2 = 大きい、3 = 同じ大きさで選択範囲あり。
    static PaintDocument MakeTarget(int kind,int seed,bool grouped,out Guid baseId) {
        int w=kind==1?9:kind==2?25:17,h=kind==1?7:kind==2?21:13,tile=kind==0||kind==3?new[]{4,8,16}[seed%3]:8;
        var d=new PaintDocument(w,h,tile,256L<<20); var l=d.AddLayer("base"); var s=l.GetChannel(PaintChannel.Color);
        for(int y=0;y<h;y++) for(int x=0;x<w;x++) if((x+y)%3!=0) s.SetPixel(x,y,Pattern(x,y,0,seed+9));
        baseId=l.Id; if(grouped) d.GroupLayers(new[]{l.Id},"g");
        if(kind==3) d.SetSelection(SelectionMask.Rectangle(d,2,2,11,9));
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
    // ───────── 選択範囲・出力 ─────────
    /// 0 なし、1 長方形（内側）、2 量が (x*17+y*29)%256 の中ほど、3 長方形（右上の隅。タイルの余白に触れる）。
    static void ApplySelection(PaintDocument d,int variant) {
        if(variant==1) d.SetSelection(SelectionMask.Rectangle(d,3,2,12,9));
        else if(variant==2) {
            var selection=SelectionMask.None(d);
            for(int y=2;y<11;y++)for(int x=3;x<14;x++)selection.Surface.SetPixel(x,y,new Rgba32(0,0,0,(byte)((x*17+y*29)%256)));
            d.SetSelection(selection);
        }
        else if(variant==3) d.SetSelection(SelectionMask.Rectangle(d,15,11,17,13));
    }
    static void WriteClip(BinaryWriter b,PixelClipboard c) {
        b.Write(c.X);b.Write(c.Y);b.Write(c.Width);b.Write(c.Height);b.Write(c.DocumentWidth);b.Write(c.DocumentHeight);
        b.Write((int)c.Source);b.Write((int)c.Channel);b.Write(c.GetPixels());
    }
    /// 断りの型: 1 ロック（断った層・持ち主・ロック）、2 層の操作の断り（理由・バイト数・上限）、3 そのほかの InvalidOperation、4 引数の誤り。
    static void Fail(BinaryWriter b,PaintDocument d,Exception e) {
        var all=d.Layers.ToList();
        if(e is LayerLockedException l) { b.Write((byte)1);b.Write(all.FindIndex(x=>x.Id==l.LayerId));b.Write(all.FindIndex(x=>x.Id==l.LockedBy));b.Write((int)l.Lock); }
        else if(e is LayerOpException op) { b.Write((byte)2);b.Write(op.Reason.ToString());b.Write(op.Bytes);b.Write(op.Limit); }
        else if(e is ArgumentException) b.Write((byte)4);
        else if(e is InvalidOperationException) b.Write((byte)3);
        else throw e;
    }
    /// 結果のあとの文書。変わっていれば Undo・Redo のあとも。
    static void After(BinaryWriter b,PaintDocument d,bool changed) {
        b.Write(d.HistoryBytes);Snapshot(b,d);
        if(changed){d.Undo();b.Write(d.HistoryBytes);Snapshot(b,d);d.Redo();b.Write(d.HistoryBytes);Snapshot(b,d);}
    }
    static void Lock(PaintDocument d,Guid id,int variant) {
        if(variant==1) d.SetLayerLocks(id,LayerLocks.Transparency);
        else if(variant==2) d.SetLayerLocks(id,LayerLocks.Pixels);
        else if(variant==3) d.SetLayerLocks(id,LayerLocks.All);
        else if(variant==4) d.SetLayerLocks(id,LayerLocks.Position);
    }
    // ───────── 事例 ─────────
    static void Copy(int n,int seed,BinaryWriter o) {
        var d=Make(seed);var layer=d.Layers[n%2].Id;var channel=(PaintChannel)((n/2)%6);
        ApplySelection(d,(n/12)%4);d.ClearHistory();
        try { var c=d.CopyPixels(layer,channel);o.Write((byte)0);WriteClip(o,c); } catch(Exception e) { Fail(o,d,e); }
        After(o,d,false);
    }
    static void MaskCopy(int n,int seed,BinaryWriter o) {
        var d=Make(seed);var layer=d.Layers[n%2].Id;
        ApplySelection(d,(n/2)%4);d.ClearHistory();
        try { var c=d.CopyPixels(layer,PaintChannel.Color,fromMask:true);o.Write((byte)0);WriteClip(o,c); } catch(Exception e) { Fail(o,d,e); }
        After(o,d,false);
    }
    /// 写せる大きさの上限（上限を超えた時点の矩形の大きさ・タイルを読む順が出力に出る）。0〜23 は層、24〜47 は結合。
    static void Limit(int n,int seed,BinaryWriter o) {
        var d=Make(seed);var layer=d.Layers[n%2].Id;long limit=new long[]{100,300,700,1500}[(n/2)%4];
        ApplySelection(d,(n/8)%3);d.ClearHistory();
        try { var c=n<24?d.CopyPixels(layer,PaintChannel.Color,false,limit):d.CopyMerged(PaintChannel.Color,limit);o.Write((byte)0);WriteClip(o,c); } catch(Exception e) { Fail(o,d,e); }
        After(o,d,false);
    }
    static void Merged(int n,int seed,BinaryWriter o) {
        var d=Make(seed);var a=d.Layers[0].Id;var b=d.Layers[1].Id;
        if(n%8==0) d.SetLayerClipping(b,true);
        if(n%8==1) { d.SetLayerOpacity(a,.4);d.SetLayerBlendMode(a,LayerBlendMode.Multiply); }
        if(n%8==2) { var bg=d.AddFillLayer("background",new Dictionary<PaintChannel,Rgba32>{{PaintChannel.Color,new Rgba32(100,200,70)}}); d.MoveLayer(bg.Id,0); }
        if(n%8==3) d.SetLayerBlendMode(b,LayerBlendMode.Screen);
        if(n%8==4) d.SetChannelEnabled(a,PaintChannel.Normal,false);
        if(n%8==5) d.SetChannelEnabled(b,PaintChannel.Metallic,false);
        if(n%8==6) {d.SetLayerMaskInverted(a,true);d.SetLayerMaskDensity(a,.35);}
        if(n%8==7) {d.SetChannelBlend(a,PaintChannel.Color,new ChannelBlend(LayerBlendMode.Multiply,.65));d.SetLayerOpacity(b,.55);}
        var channel=(n/8)%2==0?PaintChannel.Color:PaintChannel.Normal;
        ApplySelection(d,(n/16)%4);d.ClearHistory();
        try { var c=d.CopyMerged(channel);o.Write((byte)0);WriteClip(o,c); } catch(Exception e) { Fail(o,d,e); }
        After(o,d,false);
    }
    static void Cut(int n,int seed,BinaryWriter o) {
        var d=Make(seed);var a=d.Layers[0].Id;var b=d.Layers[1].Id;
        Guid layer=a;var channel=PaintChannel.Color;bool mask=false;
        switch(n%14) {
            case 1:layer=b;channel=PaintChannel.Roughness;break;
            case 2:channel=PaintChannel.Normal;break;
            case 3:d.SetLayerLocks(a,LayerLocks.Transparency);break;
            case 4:d.SetLayerLocks(a,LayerLocks.Pixels);break;
            case 5:d.SetLayerLocks(a,LayerLocks.All);break;
            case 6:{var g=d.GroupLayers(new[]{a},"g");d.SetLayerLocks(g.Id,LayerLocks.Pixels);}break;
            case 7:mask=true;break;
            case 8:mask=true;d.SetLayerLocks(a,LayerLocks.Pixels);break;
            case 9:mask=true;d.SetLayerLocks(a,LayerLocks.All);break;
            case 10:layer=d.AddFillLayer("fill",new Dictionary<PaintChannel,Rgba32>{{PaintChannel.Color,new Rgba32(10,200,30,255)}}).Id;break;
            case 11:layer=d.AddGroup("empty").Id;break;
            case 12:d.SetChannelEnabled(a,PaintChannel.Roughness,false);channel=PaintChannel.Roughness;break;
            case 13:d.SetLayerLocks(a,LayerLocks.Position);break;
        }
        ApplySelection(d,(n/14)%4);d.ClearHistory();
        bool ok=false;
        try { var c=d.CutPixels(layer,channel,mask);o.Write((byte)0);WriteClip(o,c);ok=true; } catch(Exception e) { Fail(o,d,e); }
        After(o,d,ok&&d.UndoCount>0);
    }
    static void Paste(int n,int seed,BinaryWriter o) {
        var source=Make(seed);var channel=new[]{PaintChannel.Color,PaintChannel.Roughness,PaintChannel.Normal}[n%3];
        ApplySelection(source,(n/3)%3);source.ClearHistory();
        PixelClipboard clip;
        try { clip=source.CopyPixels(source.Layers[0].Id,channel); } catch(Exception e) { Fail(o,source,e);After(o,source,false);return; }
        int kind=(n/9)%4,above=(n/36)%3,budget=(n/108)%3;
        var t=MakeTarget(kind,seed,above==2,out var baseId);
        if(budget==1) t.ActiveStrokeBudgetBytes=100;
        if(budget==2) t.SourceBudgetBytes=t.AllocatedBytes+8;
        Guid? aboveId=above==0?(Guid?)null:baseId;
        bool ok=false;
        try {
            var r=t.PasteAsLayer(clip,channel,"pasted",aboveId);ok=true;
            o.Write((byte)0);o.Write(r.X);o.Write(r.Y);o.Write(r.Centered);o.Write(r.ClippedPixels);o.Write(t.Layers.ToList().FindIndex(l=>l.Id==r.Layer.Id));
        } catch(Exception e) { Fail(o,t,e); }
        After(o,t,ok);
    }
    static byte[] Image(int w,int h,int seed) {
        var bytes=new byte[w*h*4];
        for(int y=0;y<h;y++) for(int x=0;x<w;x++) { var c=Pattern(x,y,5,seed+3);int i=(y*w+x)*4;bytes[i]=c.R;bytes[i+1]=c.G;bytes[i+2]=c.B;bytes[i+3]=c.A; }
        return bytes;
    }
    static void Replace(int n,int seed,BinaryWriter o) {
        var d=Make(seed);var a=d.Layers[0].Id;
        var channel=(n/32)%2==0?PaintChannel.Color:PaintChannel.Normal;
        Lock(d,a,(n/8)%4);
        ApplySelection(d,n%4);d.ClearHistory();
        bool ok=false;
        try { bool changed=d.ReplacePixels(a,channel,Image(17,13,seed),(n/4)%2==0);o.Write((byte)0);o.Write(changed);ok=changed; } catch(Exception e) { Fail(o,d,e); }
        After(o,d,ok);
    }
    static void BatchCase(int n,int seed,BinaryWriter o) {
        var d=Make(seed);var a=d.Layers[0].Id;var b=d.Layers[1].Id;bool changed=false;
        if(n==2) ApplySelection(d,1);
        d.ClearHistory();
        if(n==1){d.SetLayerOpacity(a,.7);d.Undo();} // 1 段のやり直しを残す
        long history=d.HistoryBytes;
        try {
            switch(n) {
                case 0:
                    d.Batch(()=>{ d.Fill(a,PaintChannel.Color,new Rgba32(10,200,30,200),.8,null,false);d.SetLayerOpacity(b,.5);var l=d.AddLayer("added");d.SetChannelEnabled(l.Id,PaintChannel.Roughness,true); });
                    changed=true;break;
                case 1:
                    d.Batch(()=>{ d.Fill(a,PaintChannel.Color,new Rgba32(0,255,0,255),1,null,false);d.AddLayer("doomed");throw new InvalidOperationException("boom"); });
                    break;
                case 2:
                    d.Batch(()=>{ var clip=d.CutPixels(a,PaintChannel.Color,false);var r=d.PasteAsLayer(clip,PaintChannel.Color,"moved",a);d.SetLayerOpacity(r.Layer.Id,.5); });
                    changed=true;break;
                case 3:
                    d.Batch(()=>{ d.SetLayerOpacity(a,.3);d.Undo(); });
                    break;
                default:
                    d.Batch(()=>{ d.SetLayerOpacity(a,.3);d.Batch(()=>{}); });
                    break;
            }
            o.Write((byte)0);
        } catch(Exception e) { Fail(o,d,e); }
        o.Write(d.UndoCount);o.Write(d.CanRedo);o.Write(history);
        After(o,d,changed);
        if(n==1){d.Redo();o.Write(d.HistoryBytes);Snapshot(o,d);}
    }
    static void Run(string op,int n,int seed,BinaryWriter o) {
        switch(op) {
            case "clipcopy":Copy(n,seed,o);break;
            case "clipmask":MaskCopy(n,seed,o);break;
            case "cliplimit":Limit(n,seed,o);break;
            case "clipmerged":Merged(n,seed,o);break;
            case "clipcut":Cut(n,seed,o);break;
            case "clippaste":Paste(n,seed,o);break;
            case "clipreplace":Replace(n,seed,o);break;
            case "clipbatch":BatchCase(n,seed,o);break;
            default:throw new ArgumentException("未知の操作: "+op);
        }
    }
    static int Main(string[] args) {
        try {
            if(args.Length!=3 || args[0]!="golden")throw new ArgumentException("golden <台本> <出力>");
            Directory.CreateDirectory(args[2]);var full=Environment.GetEnvironmentVariable("GOLDEN_FULL");
            var index=new List<string>{"# "+Environment.GetEnvironmentVariable("GOLDEN_SOURCE")};var watch=System.Diagnostics.Stopwatch.StartNew();
            foreach(var line in File.ReadLines(args[1])) {
                if(string.IsNullOrWhiteSpace(line)||line.StartsWith("#"))continue;var p=line.Split(' ');
                var stream=new MemoryStream();using(var b=new BinaryWriter(stream)) { Run(p[1],int.Parse(p[2]),int.Parse(p[3]),b);b.Flush();
                    var bytes=stream.ToArray();
                    if(!string.IsNullOrEmpty(full)){Directory.CreateDirectory(full);File.WriteAllBytes(Path.Combine(full,p[0]+".bin"),bytes);}
                    var hash=new StringBuilder();foreach(var x in SHA256.Create().ComputeHash(bytes))hash.Append(x.ToString("x2"));
                    index.Add(line+" "+hash+" "+bytes.Length); }
            }
            File.WriteAllLines(Path.Combine(args[2],"index.txt"),index);Console.WriteLine("照合用 "+(index.Count-1)+" 事例、"+watch.ElapsedMilliseconds+" ms");return 0;
        } catch(Exception e) {Console.Error.WriteLine(e);return 1;}
    }
}
