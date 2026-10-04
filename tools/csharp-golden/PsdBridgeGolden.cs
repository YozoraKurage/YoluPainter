// 人工データを実 C# Core の PSD の写し（PsdBridge.Export・Import と PsdCodec）に通す。Rust 側 crates/yolu-io/tests/psd_golden.rs と同じ台本。
// 出力（<出力先>/）:
//   index.txt          事例の一覧（名前と種類）と出どころ
//   <事例>.psd         書き出し事例: PsdBridge.Export(Color) → PsdCodec.Write のバイト列。取り込み事例: PsdCodec.Write で組んだ PSD
//   <事例>.snap        そのバイト列を PsdCodec.Read → PsdBridge.Import した文書の中身（層の並び・属性・マスク・調整・塗りつぶし・画素の指紋・合成の指紋）
//   <事例>.refused     書き出しを断る事例: C# の断りの文（Rust も断ることを確かめる。文は照合しない）
using System;
using System.IO;
using System.Linq;
using System.Text;
using System.Collections.Generic;
using Yozolab.YoluPainter.Core;
using Yozolab.YoluPainter.Core.Persistence;
using Yozolab.YoluPainter.Core.Psd;

static class PsdBridgeGolden
{
    const int W = 24, H = 16;

    // ---- 台本の部品（Rust の psd_golden.rs と同じ） ----

    /// 層 n 番目（作った順、1 から）の ID: 先頭 4 バイトが PSD の層 ID（100 + n）、続く 4 バイトがグループの区切りの ID（300 + n）、残りは固定。
    static Guid Id(int n)
    {
        var b = new byte[16];
        BitConverter.GetBytes(100 + n).CopyTo(b, 0);
        BitConverter.GetBytes(300 + n).CopyTo(b, 4);
        for (int i = 8; i < 16; i++) b[i] = (byte)(i - 7);
        return new Guid(b);
    }
    static Rgba32 Gradient(int x, int y) => new Rgba32((byte)(x * 10 + 15), (byte)(y * 14 + 20), (byte)(200 - x * 5), 255);
    static Rgba32 Soft(int x, int y) => new Rgba32((byte)(255 - x * 9), (byte)(x * 7 + y * 5), (byte)(y * 13 + 40), (byte)(x < 3 ? 0 : 60 + x * 8 + y * 3));

    sealed class Doc
    {
        public readonly PaintDocument D = new PaintDocument(W, H, 8);
        int n;
        public Guid Next() => Id(++n);
        public PaintLayer Raster(string name, Func<int, int, Rgba32> px)
        {
            var l = D.AddLayer(name, Next()); var s = l.GetChannel(PaintChannel.Color);
            for (int y = 0; y < H; y++) for (int x = 0; x < W; x++) { var c = px(x, y); if (c != Rgba32.Transparent) s.SetPixel(x, y, c); }
            return l;
        }
        public PaintLayer Fill(string name, Rgba32 c) => D.AddFillLayer(name, new Dictionary<PaintChannel, Rgba32> { { PaintChannel.Color, c } }, Next());
        public PaintLayer Adjust(string name, AdjustmentSettings s) => D.AddAdjustmentLayer(name, s, null, Next());
        public PaintLayer Group(string name, params PaintLayer[] members) => D.GroupLayers(members.Select(m => m.Id).ToArray(), name, Next());
        public void Opacity(PaintLayer l, int b) => D.SetLayerOpacity(l.Id, b / 255.0);
        public void Mode(PaintLayer l, LayerBlendMode m) => D.SetLayerBlendMode(l.Id, m);
        public void Clip(PaintLayer l) => D.SetLayerClipping(l.Id, true);
        public void Hide(PaintLayer l) => D.SetLayerVisibility(l.Id, false);
        /// 行 y（下から）を amount だけ隠すマスク。
        public RasterMask HideRow(PaintLayer l, int y, int amount)
        {
            var m = D.AddLayerMask(l.Id);
            for (int x = 0; x < W; x++) m.Surface.SetPixel(x, y, new Rgba32(0, 0, 0, (byte)amount));
            return m;
        }
    }
    static AdjustmentSettings[] Exact() => new[]
    {
        AdjustmentSettings.Invert(),
        AdjustmentSettings.Levels(20 / 255.0, 230 / 255.0, 1.37, 10 / 255.0, 240 / 255.0),
        AdjustmentSettings.HueSaturation(-73, 0.42, -0.18),
    };

    // ---- 書き出し事例 ----

    static PaintDocument Plain()
    {
        var s = new Doc();
        s.Raster("bg", Gradient);
        var a = s.Raster("soft", Soft); s.Mode(a, LayerBlendMode.Multiply); s.Opacity(a, 183); s.Clip(a);
        var h = s.Raster("hidden", Gradient); s.Hide(h);
        var o = s.Raster("overlay", Soft); s.Mode(o, LayerBlendMode.SoftLight); s.Opacity(o, 99);
        return s.D;
    }
    static PaintDocument Masks()
    {
        var s = new Doc();
        var a = s.Raster("partial", Gradient); s.HideRow(a, 5, 255); s.D.SetLayerMaskEnabled(a.Id, false); s.D.SetLayerMaskDensity(a.Id, 40 / 255.0);
        var b = s.Raster("mostly hidden", Soft); var mb = s.D.AddLayerMask(b.Id);
        for (int y = 0; y < H; y++) for (int x = 0; x < W; x++) if (!(x >= 4 && x < 9 && y >= 2 && y < 6)) mb.Surface.SetPixel(x, y, new Rgba32(0, 0, 0, 255));
        var c = s.Raster("neutral", Gradient); s.D.AddLayerMask(c.Id);
        var e = s.Raster("graded", Soft); var me = s.D.AddLayerMask(e.Id);
        for (int y = 0; y < H; y++) for (int x = 0; x < W; x++) if ((x + y) % 5 == 0) me.Surface.SetPixel(x, y, new Rgba32(0, 0, 0, (byte)(x * 11 + y * 7)));
        s.D.SetLayerMaskDensity(e.Id, 160 / 255.0);
        var f = s.Raster("all hidden", Gradient); var mf = s.D.AddLayerMask(f.Id);
        for (int y = 0; y < H; y++) for (int x = 0; x < W; x++) mf.Surface.SetPixel(x, y, new Rgba32(0, 0, 0, 255));
        return s.D;
    }
    static PaintDocument Fills()
    {
        var s = new Doc();
        s.Raster("bg", Gradient);
        var paint = s.Fill("paint", new Rgba32(200, 100, 50, 255)); s.Opacity(paint, 128); s.Mode(paint, LayerBlendMode.Multiply);
        s.HideRow(paint, 5, 200); s.D.SetLayerMaskDensity(paint.Id, 160 / 255.0);
        s.Raster("shape", Soft);
        var clipped = s.Fill("clipped fill", new Rgba32(250, 20, 20, 255)); s.Clip(clipped); s.Opacity(clipped, 90);
        var hidden = s.Fill("hidden", new Rgba32(1, 2, 3, 255)); s.Hide(hidden);
        var over = s.Fill("overlay", new Rgba32(10, 200, 30, 255)); s.Mode(over, LayerBlendMode.Overlay);
        return s.D;
    }
    static PaintDocument Adjustments()
    {
        var s = new Doc();
        s.Raster("bg", Gradient);
        var settings = Exact();
        for (int k = 0; k < 3; k++)
        {
            var plain = s.Adjust("plain" + k, settings[k]); s.Opacity(plain, 160); s.HideRow(plain, 5, 200);
            s.Raster("shape" + k, Soft);
            var clipped = s.Adjust("clipped" + k, settings[k]); s.Clip(clipped); s.Mode(clipped, LayerBlendMode.Overlay);
        }
        var hidden = s.Adjust("hidden", AdjustmentSettings.Invert()); s.Hide(hidden);
        return s.D;
    }
    static PaintDocument Groups()
    {
        var s = new Doc(); var x = Exact();
        s.Raster("bg", Gradient);
        var a = s.Raster("a", Soft); var l2 = s.Adjust("levels", x[1]);
        var pass = s.Group("pass", a, l2); s.Opacity(pass, 190); s.HideRow(pass, 9, 90);
        var b = s.Raster("b", Soft); var h = s.Adjust("hue", x[2]); var inv = s.Adjust("invert", x[0]); s.Clip(inv); s.Opacity(inv, 100);
        var iso = s.Group("isolated", b, h, inv); s.Mode(iso, LayerBlendMode.Multiply);
        var f = s.Fill("fill in group", new Rgba32(30, 90, 160, 255)); var r2 = s.Raster("clipped in group", Soft); s.Clip(r2);
        s.Group("fills", f, r2);
        var inner = s.Raster("inner", Gradient); var gi = s.Group("inner group", inner); var go = s.Group("outer group", gi);
        s.Mode(go, LayerBlendMode.Screen); s.Opacity(go, 200);
        var empty = s.D.AddGroup("empty", s.Next()); s.Hide(empty);
        var hiddenGroup = s.Raster("in hidden group", Gradient); var hg = s.Group("hidden group", hiddenGroup); s.Hide(hg);
        return s.D;
    }
    static PaintDocument Locks()
    {
        var s = new Doc();
        var a = s.Raster("transparency", Gradient); var f = s.Fill("pixels", new Rgba32(1, 2, 3, 255)); var j = s.Adjust("position", AdjustmentSettings.Invert());
        var b = s.Raster("all", Soft); var inner = s.Raster("in locked folder", Gradient); var g = s.Group("locked folder", inner);
        var everything = s.Raster("everything", Soft);
        var plain = s.Raster("plain", Gradient);
        s.D.SetLayerLocks(a.Id, LayerLocks.Transparency); s.D.SetLayerLocks(f.Id, LayerLocks.Pixels); s.D.SetLayerLocks(j.Id, LayerLocks.Position);
        s.D.SetLayerLocks(b.Id, LayerLocks.All); s.D.SetLayerLocks(g.Id, LayerLocks.All | LayerLocks.Pixels);
        s.D.SetLayerLocks(everything.Id, LayerLocks.All | LayerLocks.Transparency | LayerLocks.Pixels | LayerLocks.Position);
        s.D.SetLayerLocks(plain.Id, LayerLocks.Transparency | LayerLocks.Position);
        return s.D;
    }
    static PaintDocument ChannelBlends()
    {
        var s = new Doc();
        s.Raster("bg", Gradient);
        var a = s.Raster("colour override", Soft); s.Opacity(a, 200);
        s.D.SetChannelBlend(a.Id, PaintChannel.Color, new ChannelBlend(LayerBlendMode.Multiply, 100 / 255.0));
        s.D.SetChannelBlend(a.Id, PaintChannel.Roughness, new ChannelBlend(LayerBlendMode.Multiply, 100 / 255.0));
        // グループと調整は、効くチャンネルの全部（グループは標準の 6 つ、調整は有効なチャンネル）が Color と同じ値のときだけ Rust が書く
        var inner = s.Raster("inner", Soft); var g = s.Group("group", inner); s.Opacity(g, 255);
        foreach (var channel in new[] { PaintChannel.Color, PaintChannel.Roughness, PaintChannel.Metallic, PaintChannel.Height, PaintChannel.Normal, PaintChannel.Emission })
            s.D.SetChannelBlend(g.Id, channel, new ChannelBlend(LayerBlendMode.Screen, 128 / 255.0));
        var adj = s.Adjust("adjust", AdjustmentSettings.Invert());
        foreach (var channel in new[] { PaintChannel.Color, PaintChannel.Roughness, PaintChannel.Metallic, PaintChannel.Height, PaintChannel.Normal, PaintChannel.Emission })
            if (adj.IsChannelEnabled(channel)) s.D.SetChannelOpacity(adj.Id, channel, 150 / 255.0);
        return s.D;
    }

    // ---- 書き出しを断る事例（C# も Rust も断る） ----

    static PaintDocument RefusedInvertedMask()
    {
        var s = new Doc(); var a = s.Raster("inv", Gradient); s.D.AddLayerMask(a.Id); s.D.SetLayerMaskInverted(a.Id, true); return s.D;
    }
    static PaintDocument RefusedClippedGroup()
    {
        var s = new Doc(); s.Raster("bg", Gradient); var c = s.Raster("inner", Soft); var g = s.Group("clipped group", c); s.Clip(g); return s.D;
    }
    static PaintDocument RefusedTranslucentFill()
    {
        var s = new Doc(); s.Raster("bg", Gradient); s.Fill("glass", new Rgba32(1, 2, 3, 128)); return s.D;
    }
    static PaintDocument RefusedAdjustment(AdjustmentSettings settings)
    {
        var s = new Doc(); s.Raster("bg", Gradient); s.Adjust("between", settings); return s.D;
    }

    // ---- 取り込み事例（PsdCodec.Write で組んだ PSD を、C# の取り込みが文書にした結果） ----

    static PsdRasterLayer Full(int id, string name, Func<int, int, Rgba32> px)
    {
        var l = new PsdRasterLayer { Id = id, Name = name, Left = 0, Top = 0, Width = W, Height = H, PixelsRgba = new byte[W * H * 4] };
        for (int y = 0; y < H; y++) for (int x = 0; x < W; x++) { var c = px(x, H - 1 - y); int n = (y * W + x) * 4; l.PixelsRgba[n] = c.R; l.PixelsRgba[n + 1] = c.G; l.PixelsRgba[n + 2] = c.B; l.PixelsRgba[n + 3] = c.A; }
        return l;
    }
    static byte[] ImportMasks()
    {
        var d = new PsdDocument { Width = W, Height = H, Layers = new List<PsdRasterLayer>() };
        // 一番下から: 既定 0 で矩形の外は全部隠す / 矩形が画布の外へはみ出して、外の値は既定と同じ 255 / 無効・濃度つき
        var inside = Full(1, "default zero", Gradient);
        var m1 = new PsdLayerMask { Left = 4, Top = 2, Width = 8, Height = 6, DefaultColor = 0, Enabled = true, Density = 255, Pixels = new byte[48] };
        for (int i = 0; i < 48; i++) m1.Pixels[i] = (byte)(i % 7 == 0 ? 0 : i % 5 == 0 ? 255 : i * 37);
        inside.Mask = m1;
        var spill = Full(2, "spills out", Soft);
        var m2 = new PsdLayerMask { Left = 20, Top = 10, Width = 8, Height = 8, DefaultColor = 255, Enabled = true, Density = 200, Pixels = new byte[64] };
        for (int y = 0; y < 8; y++) for (int x = 0; x < 8; x++) m2.Pixels[y * 8 + x] = (byte)(20 + x < W && 10 + y < H ? 30 + x * 20 + y * 3 : 255);
        spill.Mask = m2;
        var off = Full(3, "disabled", Gradient);
        off.Mask = new PsdLayerMask { Left = 0, Top = 0, Width = 1, Height = 1, DefaultColor = 255, Enabled = false, Density = 77, Pixels = new byte[] { 0 } };
        d.Layers.Add(off); d.Layers.Add(spill); d.Layers.Add(inside);
        return PsdCodec.Write(d);
    }
    static byte[] ImportGroups()
    {
        var d = new PsdDocument { Width = W, Height = H, Layers = new List<PsdRasterLayer>() };
        var leaf = Full(11, "leaf", Soft);
        var fill = new PsdRasterLayer { Id = 12, Name = "fill", FillColor = new Rgba32(200, 10, 90, 255), Clipping = true, Opacity = 140, BlendMode = LayerBlendMode.Overlay, PixelsRgba = new byte[0] };
        var adj = new PsdRasterLayer { Id = 13, Name = "adjust", Adjustment = AdjustmentSettings.Levels(20 / 255.0, 230 / 255.0, 1.37, 10 / 255.0, 240 / 255.0), Opacity = 99, PixelsRgba = new byte[0] };
        var inner = new PsdRasterLayer { Id = 14, Name = "inner", Children = new List<PsdRasterLayer> { adj, fill, leaf }, DividerId = 1500, BlendMode = LayerBlendMode.PassThrough, Opacity = 180, PixelsRgba = new byte[0] };
        var outer = new PsdRasterLayer { Id = 15, Name = "outer", Children = new List<PsdRasterLayer> { inner }, DividerId = 0, BlendMode = LayerBlendMode.Multiply, PixelsRgba = new byte[0] };
        var bg = Full(16, "bg", Gradient);
        var top = new PsdRasterLayer { Id = 17, Name = "hidden hue", Width = 0, Height = 0, Adjustment = AdjustmentSettings.HueSaturation(-73, 0.42, -0.18), Visible = false, PixelsRgba = new byte[0] };
        d.Layers.Add(top); d.Layers.Add(outer); d.Layers.Add(bg);
        return PsdCodec.Write(d);
    }

    // ---- 中身の文字列（Rust の tests/psd_golden.rs の snapshot と同じ形） ----

    static ulong Fnv(IEnumerable<byte> bytes)
    {
        ulong h = 0xcbf29ce484222325;
        foreach (var b in bytes) h = (h ^ b) * 0x100000001b3;
        return h;
    }
    static string Bits(double v) => BitConverter.DoubleToInt64Bits(v).ToString("x");
    static string Snapshot(PaintDocument d, string diagnostics)
    {
        var s = new StringBuilder();
        s.Append("canvas " + d.Width + " " + d.Height + "\n");
        s.Append("diagnostics " + diagnostics + "\n");
        var layers = d.Layers.ToList();
        for (int i = 0; i < layers.Count; i++)
        {
            var l = layers[i];
            int parent = l.ParentId == Guid.Empty ? -1 : layers.FindIndex(p => p.Id == l.ParentId);
            var locks = (l.Locks & LayerLocks.All) != 0 ? LayerLocks.All : l.Locks;
            s.Append(i + " name=" + l.Name + " kind=" + l.Kind + " parent=" + parent + " id=" + l.Id.ToString("N").Substring(0, 16)
                + " visible=" + (l.Visible ? 1 : 0) + " opacity=" + (int)Math.Round(l.Opacity * 255) + " mode=" + l.BlendMode + " clip=" + (l.Clipping ? 1 : 0) + " locks=" + (int)locks);
            if (l.Mask == null) s.Append(" mask=-");
            else
            {
                var hide = new List<byte>();
                for (int y = 0; y < d.Height; y++) for (int x = 0; x < d.Width; x++) hide.Add(l.Mask.Surface.GetPixel(x, y).A);
                s.Append(" mask=" + (l.Mask.Enabled ? 1 : 0) + "/" + (l.Mask.Inverted ? 1 : 0) + "/" + (int)Math.Round(l.Mask.Density * 255) + "/" + Fnv(hide).ToString("x16"));
            }
            if (l.Kind == LayerKind.Raster)
            {
                var px = new List<byte>();
                for (int y = 0; y < d.Height; y++) for (int x = 0; x < d.Width; x++) { var c = l.GetPixel(PaintChannel.Color, x, y); px.Add(c.R); px.Add(c.G); px.Add(c.B); px.Add(c.A); }
                s.Append(" pixels=" + Fnv(px).ToString("x16"));
            }
            else if (l.Kind == LayerKind.Fill) { var c = l.FillValues[PaintChannel.Color]; s.Append(" fill=" + c.R + "," + c.G + "," + c.B + "," + c.A); }
            else if (l.Kind == LayerKind.Adjustment)
            {
                var a = l.Adjustment;
                s.Append(" adjustment=" + a.Type + "/" + string.Join("/", new[] { a.InputBlack, a.InputWhite, a.Gamma, a.OutputBlack, a.OutputWhite, a.Hue, a.Saturation, a.Lightness }.Select(Bits)));
            }
            if (l.Kind != LayerKind.Group)
                s.Append(" channels=" + string.Join(",", Enum.GetValues(typeof(PaintChannel)).Cast<PaintChannel>().Where(l.IsChannelEnabled).Select(c => (int)c)));
            s.Append("\n");
        }
        s.Append("composite " + Fnv(d.Composite(PaintChannel.Color)).ToString("x16") + "\n");
        return s.ToString();
    }
    static string Codes(PsdReadResult r) => string.Join(",", r.Diagnostics.Select(x => x.Code).OrderBy(x => x, StringComparer.Ordinal));

    static int Main(string[] args)
    {
        if (args.Length < 2 || args[0] != "golden") { Console.Error.WriteLine("usage: golden <out dir>"); return 2; }
        string outDir = args[1]; Directory.CreateDirectory(outDir);
        var index = new StringBuilder();
        index.Append("# " + (Environment.GetEnvironmentVariable("GOLDEN_SOURCE") ?? "出どころ不明") + "\n");
        var exported = new List<KeyValuePair<string, Func<PaintDocument>>>
        {
            new KeyValuePair<string, Func<PaintDocument>>("plain", Plain),
            new KeyValuePair<string, Func<PaintDocument>>("masks", Masks),
            new KeyValuePair<string, Func<PaintDocument>>("fills", Fills),
            new KeyValuePair<string, Func<PaintDocument>>("adjustments", Adjustments),
            new KeyValuePair<string, Func<PaintDocument>>("groups", Groups),
            new KeyValuePair<string, Func<PaintDocument>>("locks", Locks),
            new KeyValuePair<string, Func<PaintDocument>>("channel_blends", ChannelBlends),
        };
        foreach (var entry in exported)
        {
            var doc = entry.Value();
            byte[] bytes = PsdCodec.Write(PsdBridge.Export(doc, PaintChannel.Color));
            File.WriteAllBytes(Path.Combine(outDir, entry.Key + ".psd"), bytes);
            var read = PsdCodec.Read(bytes);
            File.WriteAllText(Path.Combine(outDir, entry.Key + ".snap"), Snapshot(PsdBridge.Import(read), Codes(read)));
            index.Append("case " + entry.Key + " export\n");
        }
        var imported = new[] { new KeyValuePair<string, byte[]>("import_masks", ImportMasks()), new KeyValuePair<string, byte[]>("import_groups", ImportGroups()) };
        foreach (var entry in imported)
        {
            File.WriteAllBytes(Path.Combine(outDir, entry.Key + ".psd"), entry.Value);
            var read = PsdCodec.Read(entry.Value);
            if (read.Mode != PsdCompatibilityMode.EditableRaster) throw new Exception(entry.Key + ": " + read.Mode);
            File.WriteAllText(Path.Combine(outDir, entry.Key + ".snap"), Snapshot(PsdBridge.Import(read), Codes(read)));
            index.Append("case " + entry.Key + " import\n");
        }
        var s = Exact();
        var refused = new[]
        {
            new KeyValuePair<string, PaintDocument>("refused_inverted_mask", RefusedInvertedMask()),
            new KeyValuePair<string, PaintDocument>("refused_clipped_group", RefusedClippedGroup()),
            new KeyValuePair<string, PaintDocument>("refused_translucent_fill", RefusedTranslucentFill()),
            new KeyValuePair<string, PaintDocument>("refused_levels_between", RefusedAdjustment(AdjustmentSettings.Levels(0.3, 1.0, 1.0))),
            new KeyValuePair<string, PaintDocument>("refused_gamma_between", RefusedAdjustment(AdjustmentSettings.Levels(0.0, 1.0, 1.234))),
            new KeyValuePair<string, PaintDocument>("refused_hue_between", RefusedAdjustment(AdjustmentSettings.HueSaturation(10.5, 0.0))),
            new KeyValuePair<string, PaintDocument>("refused_saturation_between", RefusedAdjustment(AdjustmentSettings.HueSaturation(0.0, 0.333))),
            new KeyValuePair<string, PaintDocument>("refused_levels_range", RefusedAdjustment(AdjustmentSettings.Levels(0.0, 1 / 255.0))),
        };
        foreach (var entry in refused)
        {
            string message = null;
            try { PsdBridge.Export(entry.Value, PaintChannel.Color); } catch (InvalidOperationException e) { message = e.Message; }
            if (message == null) throw new Exception(entry.Key + ": C# が断らなかった");
            File.WriteAllText(Path.Combine(outDir, entry.Key + ".refused"), message + "\n");
            index.Append("case " + entry.Key + " refused\n");
        }
        File.WriteAllText(Path.Combine(outDir, "index.txt"), index.ToString());
        return 0;
    }
}
