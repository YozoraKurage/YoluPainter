// 正本の C# Core に人工入力を通す。出力は全バイトの SHA-256。行の種類は mat・edge（マテリアルの塗り）、mask-mat・mask-edge（マスクの塗り）、
// lock・lockmask（レイヤーのロックを立てたレイヤーへの塗り・グラデーション・三角形の塗り・ストローク。断ったレイヤー・持ち主・ロックと、断ったあとの文書、通ったときの Undo/Redo まで。
// チャンネルごとに透明な画素が違う画素と、文書の選択範囲、マスクの無いレイヤーも含む）、locknr（塗りつぶし・調整・グループのレイヤーに書いたとき、型の拒否とロックの拒否のどちらが先か）、
// id・uv・uvx（ID の色・UV の配置）、tri（三角形の和集合）、rollback・rollback-fill（ストロークの巻き戻しのバイト数。ハッシュでなく数）。Rust 側の試験（crates/yolu-core/tests/reference/material_golden.rs）が行の鍵を数えて照らす。
using System;
using System.IO;
using System.Linq;
using System.Diagnostics;
using System.Security.Cryptography;
using Yozolab.YoluPainter.Core;
using Yozolab.YoluPainter.Core.MeshMaps;
class MaterialGolden {
    /// <summary>キャンバスの場面。mat はキャンバスがタイルの整数倍、edge は端が欠けたタイル（37×29・タイル 8）に画素と三角形が掛かる。</summary>
    sealed class Scene {
        public string Name; public int W, H;
        public (double, double, double, double, double, double)[] Triangles;
        public (double X, double Y, double P, double T)[] Points;
        public double Cx, Cy, Rx, Ry, Gx0, Gy0, Gx1, Gy1;
    }
    static readonly Scene[] Scenes = {
        new Scene { Name = "mat", W = 32, H = 24,
            Triangles = new[] { (1.2, 2.3, 30.2, 3.1, 25.4, 21.6), (1.2, 2.3, 25.4, 21.6, 2.1, 22.2), (9.7, 0.1, 31.8, 19.7, 4.9, 17.2) },
            Points = new[] { (3.2, 4.3, .4, 0.0), (16.7, 13.2, 1.0, .1), (29.1, 20.6, .7, .2) },
            Cx = 17.3, Cy = 11.7, Rx = 12.4, Ry = 8.6, Gx0 = 3.2, Gy0 = 5.7, Gx1 = 27.1, Gy1 = 18.9 },
        new Scene { Name = "edge", W = 37, H = 29,
            Triangles = new[] { (2.5, 1.5, 44.0, 3.0, 30.5, 35.5), (-6.2, 10.0, 14.1, -4.4, 20.3, 31.8), (30.1, 5.2, 36.9, 24.1, 34.4, 28.7) },
            Points = new[] { (2.2, 3.1, .4, 0.0), (21.7, 15.2, 1.0, .1), (35.1, 27.6, .7, .2) },
            Cx = 28.3, Cy = 19.7, Rx = 11.4, Ry = 8.6, Gx0 = 33.5, Gy0 = 25.0, Gx1 = 3.0, Gy1 = 2.5 },
    };
    static ChannelPaint[] Material()=>Enumerable.Range(0,6).Select(i=>new ChannelPaint((PaintChannel)i,new Rgba32((byte)(31+i*32),79,133,211))).ToArray();
    static byte[] Run(Scene sc,int mode,int v) {
        int w=sc.W,h=sc.H;
        var d=new PaintDocument(w,h,8); var layer=d.AddLayer("paint"); var material=Material();
        for(int c=0;c<6;c++) {
            var s=layer.GetChannel((PaintChannel)c);
            for(int y=0;y<h;y++)for(int x=0;x<w;x++) s.SetPixel(x,y,new Rgba32((byte)((x*17+c*31)%256),(byte)((y*23+c*11)%256),(byte)((x*7+y*13)%256),(byte)((x+y)%5==0?0:((x+y)*19)%256)));
        }
        if(v%2==1)d.SetSelection(SelectionMask.Ellipse(d,sc.Cx,sc.Cy,sc.Rx,sc.Ry));
        var region=SelectionMask.FromTriangles(d,sc.Triangles);
        bool erase=v>=2;
        if(mode==0) {
            var b=new BrushSettings{Radius=5,Hardness=.4,Spacing=.2,Opacity=.85,Flow=.45,Seed=1234,Erase=erase};
            if(v%2==1){b.HueJitter=.4;b.BrightnessJitter=.2;b.SizeJitter=.3;b.CurveInterpolation=true;b.TaperOut=4;}
            using(var s=d.BeginMaterialStroke(layer.Id,material,b)) {foreach(var p in sc.Points)s.Add(new BrushSample(p.X,p.Y,p.P,p.T));s.Commit();}
        } else if(mode==1) d.FillMaterial(layer.Id,material,.63,region,erase);
        else if(mode==2 || mode==3) {
            var g=new GradientSettings{X0=sc.Gx0,Y0=sc.Gy0,X1=sc.Gx1,Y1=sc.Gy1,Opacity=.73,Shape=v%2==0?GradientShape.Linear:GradientShape.Radial};
            if(mode==2)d.GradientMaterial(layer.Id,material,g,region,erase);
            else d.GradientMaterial(layer.Id,material,material.Reverse().Select(m=>new ChannelPaint(m.Channel,new Rgba32(213,47,(byte)(13+(int)m.Channel*27),83))).ToArray(),g,region,erase);
        } else if(mode==4) {
            var t=sc.Triangles;
            var f=d.BeginMaterialTriangleFill(layer.Id,material,.63,erase);
            f.Add(new[]{t[1]});f.Add(new[]{t[0],t[1]});f.Add(new[]{t[2]});f.Stroke.Commit();
        } else {
            // キャンバスぴったりの四角を 2 回に分けて: 端が欠けたタイルもキャンバスの中が全部覆われた印（全覆いのタイル）になる
            var f=d.BeginMaterialTriangleFill(layer.Id,material,.63,erase);
            f.Add(new[]{(0.0,0.0,(double)w,0.0,(double)w,(double)h)});f.Add(new[]{(0.0,0.0,(double)w,(double)h,0.0,(double)h)});f.Stroke.Commit();
        }
        var bytes=new byte[w*h*4*6];
        for(int c=0;c<6;c++)for(int y=0;y<h;y++)for(int x=0;x<w;x++) {
            var p=layer.GetChannel((PaintChannel)c).GetPixel(x,y);int o=(c*w*h+y*w+x)*4;bytes[o]=p.R;bytes[o+1]=p.G;bytes[o+2]=p.B;bytes[o+3]=p.A;
        }
        return bytes;
    }
    /// <summary>マスクの塗り: mode 0 は GradientMask（範囲は三角形）、1 は BeginMaskTriangleFill。v のビット 0 は文書の選択範囲、ビット 1 は見せる側。</summary>
    static byte[] RunMask(Scene sc,int mode,int v) {
        int w=sc.W,h=sc.H;
        var d=new PaintDocument(w,h,8); var layer=d.AddLayer("paint"); d.AddLayerMask(layer.Id);
        var mask=layer.Mask.Surface;
        for(int y=0;y<h;y++)for(int x=0;x<w;x++) mask.SetPixel(x,y,new Rgba32(0,0,0,(byte)((x*29+y*41)%7==0?0:(x*11+y*17+40)%256)));
        if(v%2==1)d.SetSelection(SelectionMask.Ellipse(d,sc.Cx,sc.Cy,sc.Rx,sc.Ry));
        bool reveal=v>=2;
        if(mode==0) {
            var g=new GradientSettings{X0=sc.Gx0,Y0=sc.Gy0,X1=sc.Gx1,Y1=sc.Gy1,Opacity=.73,From=new Rgba32(31,79,133,200),To=new Rgba32(213,47,13,40),Shape=v%2==0?GradientShape.Linear:GradientShape.Radial};
            d.GradientMask(layer.Id,g,SelectionMask.FromTriangles(d,sc.Triangles),reveal);
        } else {
            var t=sc.Triangles;
            var f=d.BeginMaskTriangleFill(layer.Id,.63,reveal);
            f.Add(new[]{t[1]});f.Add(new[]{t[0],t[1]});f.Add(new[]{t[2]});f.Stroke.Commit();
        }
        var bytes=new byte[w*h];
        for(int y=0;y<h;y++)for(int x=0;x<w;x++) bytes[y*w+x]=mask.GetPixel(x,y).A;
        return bytes;
    }
    /// <summary>ロックの場面（edge のキャンバス）。レイヤーは 6 チャンネルとも画素あり。透明な画素はチャンネルごとに違う（アルファの式にチャンネル番号が入る）。
    /// lockCase: 0 ロックなし・1 透明部分・2 画像・3 すべて・4 位置（塗りは通る）、
    /// 5〜7 はレイヤーを 1 つだけ含むグループに 透明部分・画像・すべて。off は 2 つのチャンネルを無効にしておく（拒否が有効化を残さないことを見る）。
    /// selected は文書の選択範囲（楕円）を立てる。</summary>
    static PaintDocument LockDoc(int lockCase,bool off,bool withMask,bool selected,out PaintLayer layer) {
        var sc=Scenes[1]; int w=sc.W,h=sc.H;
        var d=new PaintDocument(w,h,8); layer=d.AddLayer("paint");
        for(int c=0;c<6;c++) {
            var s=layer.GetChannel((PaintChannel)c);
            for(int y=0;y<h;y++)for(int x=0;x<w;x++) s.SetPixel(x,y,new Rgba32((byte)((x*17+c*31)%256),(byte)((y*23+c*11)%256),(byte)((x*7+y*13)%256),(byte)((x+y+c)%5==0?0:((x+y)*19+c*23)%256)));
        }
        if(withMask) {
            d.AddLayerMask(layer.Id); var mask=layer.Mask.Surface;
            for(int y=0;y<h;y++)for(int x=0;x<w;x++) mask.SetPixel(x,y,new Rgba32(0,0,0,(byte)((x*29+y*41)%7==0?0:(x*11+y*17+40)%256)));
        }
        if(selected)d.SetSelection(SelectionMask.Ellipse(d,sc.Cx,sc.Cy,sc.Rx,sc.Ry));
        if(off){d.SetChannelEnabled(layer.Id,(PaintChannel)2,false);d.SetChannelEnabled(layer.Id,(PaintChannel)4,false);}
        var holder=layer;
        if(lockCase>=5) holder=d.GroupLayers(new[]{layer.Id},"g");
        var locks=new[]{LayerLocks.None,LayerLocks.Transparency,LayerLocks.Pixels,LayerLocks.All,LayerLocks.Position,LayerLocks.Transparency,LayerLocks.Pixels,LayerLocks.All};
        d.SetLayerLocks(holder.Id,locks[lockCase]);
        d.ClearHistory();
        return d;
    }
    static void LockState(BinaryWriter b,PaintLayer l,int w,int h,bool maskOnly) {
        if(maskOnly){b.Write(l.Mask!=null);if(l.Mask!=null)for(int y=0;y<h;y++)for(int x=0;x<w;x++)b.Write(l.Mask.Surface.GetPixel(x,y).A);return;}
        for(int c=0;c<6;c++) {
            b.Write(l.IsChannelEnabled((PaintChannel)c));
            bool has=l.TryGetChannel((PaintChannel)c,out var s); b.Write(has);
            if(has)for(int y=0;y<h;y++)for(int x=0;x<w;x++){var p=s.GetPixel(x,y);b.Write(p.R);b.Write(p.G);b.Write(p.B);b.Write(p.A);}
        }
    }
    /// <summary>ロックを立てたレイヤーへの書き込み。断ったレイヤー・持ち主・ロック（番号は文書のレイヤーの並びの位置）か、通ったか（0）、そのほかの拒否（2）と、
    /// 続く Undo/Redo の可否、レイヤーの全チャンネル（有効か・面があるか・画素）。通って履歴ができたときは、Undo の後と Redo の後も。</summary>
    static byte[] RunLock(int kind,int lockCase,int v) {
        var sc=Scenes[1]; int w=sc.W,h=sc.H; bool erase=(v&1)!=0,off=(v&2)!=0,selected=(v&4)!=0;
        var d=LockDoc(lockCase,off,false,selected,out var layer);
        var single=(PaintChannel)(off?2:0); // 単チャンネルの入口は、off のときは無効にしたチャンネルへ書く（無効の拒否とロックの拒否の順を見る）
        var material=Material(); var t=sc.Triangles;
        var region=SelectionMask.FromTriangles(d,t);
        var g=new GradientSettings{X0=sc.Gx0,Y0=sc.Gy0,X1=sc.Gx1,Y1=sc.Gy1,Opacity=.73,From=new Rgba32(31,79,133,200),To=new Rgba32(213,47,13,40),Shape=GradientShape.Linear};
        var ms=new MemoryStream(); var b=new BinaryWriter(ms); bool ok=false;
        try {
            if(kind==0) {
                var brush=new BrushSettings{Radius=5,Hardness=.4,Spacing=.2,Opacity=.85,Flow=.45,Seed=1234,Erase=erase};
                using(var s=d.BeginMaterialStroke(layer.Id,material,brush)) {foreach(var p in sc.Points)s.Add(new BrushSample(p.X,p.Y,p.P,p.T));s.Commit();}
            } else if(kind==1) d.FillMaterial(layer.Id,material,.63,region,erase);
            else if(kind==2) d.GradientMaterial(layer.Id,material,g,region,erase);
            else if(kind==3) d.GradientMaterial(layer.Id,material,material.Reverse().Select(m=>new ChannelPaint(m.Channel,new Rgba32(213,47,(byte)(13+(int)m.Channel*27),83))).ToArray(),g,region,erase);
            else if(kind==4) {
                var f=d.BeginMaterialTriangleFill(layer.Id,material,.63,erase);
                f.Add(new[]{t[1]});f.Add(new[]{t[0],t[1]});f.Add(new[]{t[2]});f.Stroke.Commit();
            } else if(kind==5) d.Gradient(layer.Id,single,g,region,erase);
            else if(kind==6) {
                var f=d.BeginTriangleFill(layer.Id,single,new Rgba32(213,47,13,211),.63,erase);
                f.Add(new[]{t[1]});f.Add(new[]{t[0],t[1]});f.Add(new[]{t[2]});f.Stroke.Commit();
            } else {
                var brush=new BrushSettings{Color=new Rgba32(213,47,13,211),Radius=5,Hardness=.4,Spacing=.2,Opacity=.85,Flow=.45,Seed=1234,Erase=erase};
                using(var s=d.BeginStroke(layer.Id,single,brush)) {foreach(var p in sc.Points)s.Add(new BrushSample(p.X,p.Y,p.P,p.T));s.Commit();}
            }
            ok=true; b.Write((byte)0);
        }
        catch(LayerLockedException e) { var all=d.Layers.ToList(); b.Write((byte)1); b.Write(all.FindIndex(l=>l.Id==e.LayerId)); b.Write(all.FindIndex(l=>l.Id==e.LockedBy)); b.Write((int)e.Lock); }
        catch(Exception) { b.Write((byte)2); }
        b.Write(d.CanUndo); b.Write(d.CanRedo); LockState(b,layer,w,h,false);
        if(ok && d.CanUndo) { d.Undo(); b.Write(d.CanUndo); b.Write(d.CanRedo); LockState(b,layer,w,h,false); d.Redo(); b.Write(d.CanUndo); b.Write(d.CanRedo); LockState(b,layer,w,h,false); }
        b.Flush(); return ms.ToArray();
    }
    /// <summary>ロックを立てたレイヤーのマスクへの書き込み（画像・透明部分のロックでは通り、すべてのロックでだけ断る）。v のビット 0 は見せる側、ビット 1 はレイヤーにマスクが無い
    /// （どのロックでも「マスクが無い」で断る。ロックの拒否より先）、ビット 2 は文書の選択範囲。</summary>
    static byte[] RunLockMask(int kind,int lockCase,int v) {
        var sc=Scenes[1]; int w=sc.W,h=sc.H; bool reveal=(v&1)!=0,noMask=(v&2)!=0,selected=(v&4)!=0;
        var d=LockDoc(lockCase,false,!noMask,selected,out var layer);
        var t=sc.Triangles; var region=SelectionMask.FromTriangles(d,t);
        var g=new GradientSettings{X0=sc.Gx0,Y0=sc.Gy0,X1=sc.Gx1,Y1=sc.Gy1,Opacity=.73,From=new Rgba32(31,79,133,200),To=new Rgba32(213,47,13,40),Shape=GradientShape.Linear};
        var ms=new MemoryStream(); var b=new BinaryWriter(ms); bool ok=false;
        try {
            if(kind==0) d.GradientMask(layer.Id,g,region,reveal);
            else if(kind==1) {
                var f=d.BeginMaskTriangleFill(layer.Id,.63,reveal);
                f.Add(new[]{t[1]});f.Add(new[]{t[0],t[1]});f.Add(new[]{t[2]});f.Stroke.Commit();
            } else if(kind==2) d.FillMask(layer.Id,.63,region,reveal);
            else {
                var brush=new BrushSettings{Radius=5,Hardness=.4,Spacing=.2,Opacity=.85,Flow=.45,Seed=1234,Erase=reveal};
                using(var s=d.BeginMaskStroke(layer.Id,brush)) {foreach(var p in sc.Points)s.Add(new BrushSample(p.X,p.Y,p.P,p.T));s.Commit();}
            }
            ok=true; b.Write((byte)0);
        }
        catch(LayerLockedException e) { var all=d.Layers.ToList(); b.Write((byte)1); b.Write(all.FindIndex(l=>l.Id==e.LayerId)); b.Write(all.FindIndex(l=>l.Id==e.LockedBy)); b.Write((int)e.Lock); }
        catch(Exception) { b.Write((byte)2); }
        b.Write(d.CanUndo); b.Write(d.CanRedo); LockState(b,layer,w,h,true);
        if(ok && d.CanUndo) { d.Undo(); b.Write(d.CanUndo); b.Write(d.CanRedo); LockState(b,layer,w,h,true); d.Redo(); b.Write(d.CanUndo); b.Write(d.CanRedo); LockState(b,layer,w,h,true); }
        b.Flush(); return ms.ToArray();
    }
    /// <summary>塗りつぶし・調整・グループのレイヤーの場面（edge のキャンバス）。レイヤーに全体のマスクを付け、ロックの置き方は LockDoc と同じ（lockCase 5〜7 はレイヤーを含むグループに掛ける）。
    /// target: 0 塗りつぶし（Color と Emission の値）・1 調整（反転）・2 グループ。</summary>
    static PaintDocument LockKindDoc(int target,int lockCase,out PaintLayer layer) {
        var sc=Scenes[1]; int w=sc.W,h=sc.H;
        var d=new PaintDocument(w,h,8);
        if(target==0) layer=d.AddFillLayer("n",new System.Collections.Generic.Dictionary<PaintChannel,Rgba32>{{PaintChannel.Color,new Rgba32(10,20,30,255)},{PaintChannel.Emission,new Rgba32(40,50,60,255)}});
        else if(target==1) layer=d.AddAdjustmentLayer("n",AdjustmentSettings.Invert());
        else layer=d.AddGroup("n");
        d.AddLayerMask(layer.Id); var mask=layer.Mask.Surface;
        for(int y=0;y<h;y++)for(int x=0;x<w;x++) mask.SetPixel(x,y,new Rgba32(0,0,0,(byte)((x*29+y*41)%7==0?0:(x*11+y*17+40)%256)));
        var holder=layer;
        if(lockCase>=5) holder=d.GroupLayers(new[]{layer.Id},"g");
        var locks=new[]{LayerLocks.None,LayerLocks.Transparency,LayerLocks.Pixels,LayerLocks.All,LayerLocks.Position,LayerLocks.Transparency,LayerLocks.Pixels,LayerLocks.All};
        d.SetLayerLocks(holder.Id,locks[lockCase]);
        d.ClearHistory();
        return d;
    }
    static void LockKindState(BinaryWriter b,PaintLayer l,int w,int h) {
        for(int c=0;c<6;c++) b.Write(l.IsChannelEnabled((PaintChannel)c));
        for(int y=0;y<h;y++)for(int x=0;x<w;x++)b.Write(l.Mask.Surface.GetPixel(x,y).A);
    }
    /// <summary>塗りつぶし・調整・グループのレイヤーへの書き込み。entry: 0 BeginStroke・1 BeginMaterialStroke・2 FillMaterial・3 GradientMaterial・4 BeginMaterialTriangleFill・
    /// 5 Fill・6 Gradient・7 BeginTriangleFill（5〜7 は Color）・8 GradientMask・9 BeginMaskTriangleFill・10 FillMask・11 BeginMaskStroke。v のビット 0 は消す（マスクでは見せる）。
    /// 先頭は 0 通った・1 ロックで断った（レイヤー・持ち主・ロックの並びの位置）・2 そのほかの拒否（型）。続いて Undo/Redo の可否・各チャンネルの有効・マスクの隠す量。</summary>
    static byte[] RunLockKind(int entry,int target,int lockCase,int v) {
        var sc=Scenes[1]; int w=sc.W,h=sc.H; bool flag=(v&1)!=0;
        var d=LockKindDoc(target,lockCase,out var layer);
        var material=Material(); var t=sc.Triangles;
        var region=SelectionMask.FromTriangles(d,t);
        var g=new GradientSettings{X0=sc.Gx0,Y0=sc.Gy0,X1=sc.Gx1,Y1=sc.Gy1,Opacity=.73,From=new Rgba32(31,79,133,200),To=new Rgba32(213,47,13,40),Shape=GradientShape.Linear};
        var ms=new MemoryStream(); var b=new BinaryWriter(ms); bool ok=false;
        try {
            var brush=new BrushSettings{Radius=5,Hardness=.4,Spacing=.2,Opacity=.85,Flow=.45,Seed=1234,Erase=flag};
            switch(entry) {
            case 0: brush.Color=new Rgba32(213,47,13,211); using(var s=d.BeginStroke(layer.Id,PaintChannel.Color,brush)) {foreach(var p in sc.Points)s.Add(new BrushSample(p.X,p.Y,p.P,p.T));s.Commit();} break;
            case 1: using(var s=d.BeginMaterialStroke(layer.Id,material,brush)) {foreach(var p in sc.Points)s.Add(new BrushSample(p.X,p.Y,p.P,p.T));s.Commit();} break;
            case 2: d.FillMaterial(layer.Id,material,.63,region,flag); break;
            case 3: d.GradientMaterial(layer.Id,material,g,region,flag); break;
            case 4: { var f=d.BeginMaterialTriangleFill(layer.Id,material,.63,flag); f.Add(new[]{t[1]});f.Add(new[]{t[0],t[1]});f.Add(new[]{t[2]});f.Stroke.Commit(); break; }
            case 5: d.Fill(layer.Id,PaintChannel.Color,new Rgba32(213,47,13,211),.63,region,flag); break;
            case 6: d.Gradient(layer.Id,PaintChannel.Color,g,region,flag); break;
            case 7: { var f=d.BeginTriangleFill(layer.Id,PaintChannel.Color,new Rgba32(213,47,13,211),.63,flag); f.Add(new[]{t[1]});f.Add(new[]{t[0],t[1]});f.Add(new[]{t[2]});f.Stroke.Commit(); break; }
            case 8: d.GradientMask(layer.Id,g,region,flag); break;
            case 9: { var f=d.BeginMaskTriangleFill(layer.Id,.63,flag); f.Add(new[]{t[1]});f.Add(new[]{t[0],t[1]});f.Add(new[]{t[2]});f.Stroke.Commit(); break; }
            case 10: d.FillMask(layer.Id,.63,region,flag); break;
            default: using(var s=d.BeginMaskStroke(layer.Id,brush)) {foreach(var p in sc.Points)s.Add(new BrushSample(p.X,p.Y,p.P,p.T));s.Commit();} break;
            }
            ok=true; b.Write((byte)0);
        }
        catch(LayerLockedException e) { var all=d.Layers.ToList(); b.Write((byte)1); b.Write(all.FindIndex(l=>l.Id==e.LayerId)); b.Write(all.FindIndex(l=>l.Id==e.LockedBy)); b.Write((int)e.Lock); }
        catch(Exception) { b.Write((byte)2); }
        b.Write(d.CanUndo); b.Write(d.CanRedo); LockKindState(b,layer,w,h);
        if(ok && d.CanUndo) { d.Undo(); b.Write(d.CanUndo); b.Write(d.CanRedo); LockKindState(b,layer,w,h); d.Redo(); b.Write(d.CanUndo); b.Write(d.CanRedo); LockKindState(b,layer,w,h); }
        b.Flush(); return ms.ToArray();
    }
    /// <summary>1 つの点で 1 回塗ったときのストロークの巻き戻しのバイト数（PeakWorkingBytes − 確保量 − 履歴）。C# は被覆（覆い）を全チャンネルで
    /// 1 枚共有し、元のタイルの写しだけをチャンネルごとに持つ。tile はキャンバスの 1 タイルの辺（キャンバスは 6×5 タイル）、existing は全チャンネルに画素があるレイヤー。</summary>
    static long Rollback(int tile,bool existing,int channels) {
        int w=tile*6,h=tile*5;
        var d=new PaintDocument(w,h,tile); var layer=d.AddLayer("paint");
        if(existing) for(int c=0;c<6;c++){ var s=layer.GetChannel((PaintChannel)c); for(int y=0;y<h;y++)for(int x=0;x<w;x++) s.SetPixel(x,y,new Rgba32((byte)(x*7),(byte)(y*5),(byte)(x+y),200)); }
        using(var st=d.BeginMaterialStroke(layer.Id,Material().Take(channels).ToArray(),new BrushSettings{Radius=tile*0.75})) {
            st.Add(new BrushSample(tile*2.5,tile*2.5,1,0));
            long bytes=d.PeakWorkingBytes-d.AllocatedBytes-d.HistoryBytes;
            st.Cancel();
            return bytes;
        }
    }
    /// <summary>累積の三角形の塗りで、1 回の Add（edge の場面の 3 三角形）のあとのストロークの巻き戻しのバイト数。選択範囲の外のタイルも、
    /// 写しを取って数える。全チャンネルに画素があるレイヤー（Run と同じ画素）。</summary>
    static long RollbackFill(bool selected,int channels) {
        var sc=Scenes[1]; int w=sc.W,h=sc.H;
        var d=new PaintDocument(w,h,8); var layer=d.AddLayer("paint");
        for(int c=0;c<6;c++) {
            var s=layer.GetChannel((PaintChannel)c);
            for(int y=0;y<h;y++)for(int x=0;x<w;x++) s.SetPixel(x,y,new Rgba32((byte)((x*17+c*31)%256),(byte)((y*23+c*11)%256),(byte)((x*7+y*13)%256),(byte)((x+y)%5==0?0:((x+y)*19)%256)));
        }
        if(selected)d.SetSelection(SelectionMask.Ellipse(d,sc.Cx,sc.Cy,sc.Rx,sc.Ry));
        var f=d.BeginMaterialTriangleFill(layer.Id,Material().Take(channels).ToArray(),.63,false);
        f.Add(sc.Triangles);
        long bytes=d.PeakWorkingBytes-d.AllocatedBytes-d.HistoryBytes;
        f.Stroke.Cancel();
        return bytes;
    }
    static string Hash(byte[] b)=>BitConverter.ToString(SHA256.Create().ComputeHash(b)).Replace("-","").ToLowerInvariant();
    static byte[] Ids(int v) {
        const int w=32,h=24;var data=new ushort[w*h*3];var coverage=new byte[w*h];
        for(int i=0;i<w*h;i++){coverage[i]=(byte)(i%7==0?0:1+i%2);for(int c=0;c<3;c++)data[i*3+c]=(ushort)((i*1193+c*13451)%65536);}
        var map=new BakedMeshMap(new MeshMapProvenance(MeshMapKind.Id,1,"synthetic","synthetic",0,w,h,-1,0,1,"test","SnapshotWorld","StaticSnapshot","Self",new[]{-1.0,-2.0,-3.0},new[]{2.0,3.0,1.0}),data,coverage);
        var doc=new PaintDocument(w,h,8);var m=SelectionMask.FromIdColors(doc,map,v==0?new int[0]:new[]{0,0x0800F7,0x325678},new[]{0,0,7,8,63,255}[v]);
        var bytes=new byte[w*h];for(int y=0;y<h;y++)for(int x=0;x<w;x++)bytes[y*w+x]=m[x,y];return bytes;
    }
    static byte[] UvBytes(UvTriangle[] before,UvTriangle[] after,int resolution) {
        UvComparison r;
        try { r=UvLayout.Compare(before,after,resolution); } catch(ArgumentOutOfRangeException) { return new byte[]{255}; }
        return new[]{(byte)(r.Same?1:0)}.Concat(BitConverter.GetBytes(r.Kept)).Concat(BitConverter.GetBytes(r.Added)).ToArray();
    }
    static byte[] Uvs(int v) {
        var a=new UvTriangle(0,0,1,0,0,1);var b=new UvTriangle(.1,.2,.8,.1,.6,.7);
        UvTriangle[] before={a,b};UvTriangle[] after;
        if(v==0)after=new[]{b,a};else if(v==1)after=new[]{new UvTriangle(1,0,0,1,0,0),b};
        else if(v==2)after=new[]{new UvTriangle(0,0,0,1,1,0),b};else if(v==3)after=new[]{a};
        else if(v==4)after=new UvTriangle[0];else after=new[]{new UvTriangle(.2,.2,.9,.2,.2,.9)};
        return UvBytes(before,after,32);
    }
    /// <summary>解像度の境界・NaN と無限大の三角形（同じ判定でも被覆でも飛ばす）・画面の外へはみ出す UV。1 バイトの 255 は解像度の拒否。</summary>
    static byte[] UvExtra(int v) {
        var a=new UvTriangle(0,0,1,0,0,1);var b=new UvTriangle(.1,.2,.8,.1,.6,.7);
        var nan=new UvTriangle(double.NaN,0,1,0,0,1);var inf=new UvTriangle(0,double.PositiveInfinity,1,0,0,1);
        switch(v) {
            case 0: return UvBytes(new[]{a,b},new[]{a,b},0);
            case 1: return UvBytes(new[]{a,b},new[]{a,b},4097);
            case 2: return UvBytes(new[]{a,b},new[]{b},1);
            case 3: return UvBytes(new[]{a,b},new[]{b},4096);
            case 4: return UvBytes(new[]{nan},new[]{nan},16);
            case 5: return UvBytes(new[]{nan,b},new[]{b,inf},16);
            case 6: return UvBytes(new[]{a},new[]{new UvTriangle(-.5,-.5,1.5,-.5,.25,1.5)},16);
            case 7: return UvBytes(new[]{nan,inf},new UvTriangle[0],16);
            case 8: return UvBytes(new[]{a},new[]{a,a},16);
            default: return UvBytes(new[]{a},new[]{new UvTriangle(1,1,0,1,1,0)},4); // 斜めの辺を共有する向かいの半分: 13 テクセルのうち 10 が重なる
        }
    }
    static readonly (double,double,double,double,double,double)[][] TriangleCases = {
        new[]{(50.0,5.0,60.0,5.0,55.0,20.0)},                                   // 0 右の外
        new[]{(5.0,-20.0,15.0,-20.0,10.0,-3.0)},                               // 1 下の外
        new[]{(-6.2,10.0,14.1,-4.4,20.3,31.8)},                                // 2 左・下・上へはみ出す
        new[]{(1.0,1.0,10.0,10.0,20.0,20.0)},                                  // 3 一直線（面積 0）
        new[]{(1.0,1.0,1.000001,1.0,1.0,1.0000001)},                           // 4 面積 1e-13（飛ばす）
        new[]{(1.375,1.375,1.375001,1.375,1.375,1.375004)},                    // 5 頂点がサンプルの位置の極小
        new[]{(-1e6,-1e6,1e6,-1e6,0.0,1e6)},                                   // 6 キャンバスを含む巨大
        new[]{(-1e9,-1e9,1e9,-1e9,0.0,1e9)},                                   // 7 1e9 の座標
        new[]{(0.0,0.0,37.0,0.0,37.0,29.0),(0.0,0.0,37.0,29.0,0.0,29.0)},      // 8 キャンバスぴったりの 2 枚（継ぎ目なし）
        new[]{(0.0,0.0,37.0,0.0,0.0,29.0)},                                    // 9 角から角
        new[]{(8.0,8.0,16.0,8.0,8.0,16.0)},                                    // 10 タイルの境目の頂点
    };
    static byte[] Tri(int v) {
        const int w=37,h=29;var d=new PaintDocument(w,h,8);var m=SelectionMask.FromTriangles(d,TriangleCases[v]);
        var bytes=new byte[w*h];for(int y=0;y<h;y++)for(int x=0;x<w;x++)bytes[y*w+x]=m[x,y];return bytes;
    }
    static void Main(string[] args) {
        var timer=Stopwatch.StartNew(); int runs=0;
        foreach(int degree in new[]{1,4}) {
            CoreParallelism.MaxDegreeOfParallelism=degree;
            foreach(var sc in Scenes) {
                for(int mode=0;mode<6;mode++)for(int v=0;v<4;v++){Console.WriteLine($"{sc.Name} {degree} {mode} {v} {Hash(Run(sc,mode,v))}");runs++;}
                for(int mode=0;mode<2;mode++)for(int v=0;v<4;v++){Console.WriteLine($"mask-{sc.Name} {degree} {mode} {v} {Hash(RunMask(sc,mode,v))}");runs++;}
            }
        }
        for(int v=0;v<6;v++){Console.WriteLine($"id {v} {Hash(Ids(v))}");Console.WriteLine($"uv {v} {Hash(Uvs(v))}");runs+=2;}
        for(int v=0;v<10;v++){Console.WriteLine($"uvx {v} {Hash(UvExtra(v))}");runs++;}
        for(int v=0;v<TriangleCases.Length;v++){Console.WriteLine($"tri {v} {Hash(Tri(v))}");runs++;}
        CoreParallelism.MaxDegreeOfParallelism=1;
        foreach(int tile in new[]{8,128})foreach(int existing in new[]{0,1})foreach(int channels in new[]{1,6}){Console.WriteLine($"rollback {tile} {existing} {channels} {Rollback(tile,existing==1,channels)}");runs++;}
        foreach(int selected in new[]{0,1})foreach(int channels in new[]{1,6}){Console.WriteLine($"rollback-fill {selected} {channels} {RollbackFill(selected==1,channels)}");runs++;}
        foreach(int degree in new[]{1,4}) {
            CoreParallelism.MaxDegreeOfParallelism=degree;
            for(int kind=0;kind<8;kind++)for(int lockCase=0;lockCase<8;lockCase++)for(int v=0;v<8;v++){Console.WriteLine($"lock {degree} {kind} {lockCase} {v} {Hash(RunLock(kind,lockCase,v))}");runs++;}
            for(int kind=0;kind<4;kind++)for(int lockCase=0;lockCase<8;lockCase++)for(int v=0;v<8;v++){Console.WriteLine($"lockmask {degree} {kind} {lockCase} {v} {Hash(RunLockMask(kind,lockCase,v))}");runs++;}
        }
        CoreParallelism.MaxDegreeOfParallelism=1; // 断るだけの事例が主（通るのはマスクの書き込みで、並列度はここでは見ない）
        for(int entry=0;entry<12;entry++)for(int target=0;target<3;target++)for(int lockCase=0;lockCase<8;lockCase++)for(int v=0;v<2;v++){Console.WriteLine($"locknr {entry} {target} {lockCase} {v} {Hash(RunLockKind(entry,target,lockCase,v))}");runs++;}
        Console.Error.WriteLine($"C# マテリアル {runs} 実行 {timer.Elapsed.TotalMilliseconds:F3} ms");
    }
}
