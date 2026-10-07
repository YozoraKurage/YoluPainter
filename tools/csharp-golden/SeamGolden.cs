// 効果（フィルター・Generator・Anchor・塗りつぶしの画像・パス）とレイヤーのロック・レイヤーの操作のつなぎ目を、実 C# の PaintDocument に通す。
// Rust 側 crates/yolu-core/tests/reference/seam_golden.rs と同じ台本・同じ人工の文書・同じ書き出しの並び。事例ごとに SHA-256 を 1 行（名前 ハッシュ）。
//   golden             全事例の「名前 ハッシュ」を標準出力へ
//   dump <名前> <出力>   1 事例の生のバイト列（Rust 側の SEAM_DUMP_DIR と cmp で比べて、食い違いの場所を探す）
// 人工の画素・画像・パスだけ（ユーザーのデータは使わない）。メッシュマップは渡さない（Generator の Anchor は使える。形のグラデーションなど
// マップを要る段は、入力のまま通す＝効いていない効果になる）。
using System;
using System.IO;
using System.Linq;
using System.Text;
using System.Collections.Generic;
using System.Security.Cryptography;
using Yozolab.YoluPainter.Core;
using Yozolab.YoluPainter.Core.Paths;
using Yozolab.YoluPainter.Core.Shelf;
static class SeamGolden
{
    const int W = 40, H = 28, TS = 8;
    static readonly PaintChannel[] All = (PaintChannel[])Enum.GetValues(typeof(PaintChannel));

    // ───────── 人工の文書 ─────────

    static PaintLayer Lay(PaintDocument d, Guid id) => d.Layers.First(l => l.Id == id);
    static void Paint(PaintDocument d, Guid layer, PaintChannel c, int seed)
    {
        var s = Lay(d, layer).GetChannel(c);
        for (int y = 0; y < H; y++) for (int x = 0; x < W; x++)
        {
            if ((x * 7 + y * 3 + seed) % 5 == 0) continue;
            s.SetPixel(x, y, new Rgba32((byte)(x * 13 + y * 5 + seed * 3), (byte)(x * 3 + y * 17 + seed), (byte)(x + y * 31 + seed * 7), (byte)((x * 29 + y * 11 + seed * 5) % 256)));
        }
    }
    static void PaintSurface(SparseTileSurface s, int seed)
    {
        for (int y = 0; y < H; y++) for (int x = 0; x < W; x++)
        {
            if ((x * 7 + y * 3 + seed) % 5 == 0) continue;
            s.SetPixel(x, y, new Rgba32((byte)(x * 13 + y * 5 + seed * 3), (byte)(x * 3 + y * 17 + seed), (byte)(x + y * 31 + seed * 7), (byte)((x * 29 + y * 11 + seed * 5) % 256)));
        }
    }
    static readonly Guid ImageId = new Guid(0x1000, 0x2234, 0x5678, new byte[] { 0x80, 1, 2, 3, 4, 5, 6, 7 });
    static ProjectResources Images()
    {
        var resources = new ProjectResources();
        const int w = 8, h = 6;
        var bytes = new byte[w * h * 4];
        for (int y = 0; y < h; y++) for (int x = 0; x < w; x++)
        {
            int o = (y * w + x) * 4;
            bytes[o] = (byte)(x * 31 + 20); bytes[o + 1] = (byte)(y * 40 + 10); bytes[o + 2] = (byte)((x + y) % 2 == 0 ? 220 : 60); bytes[o + 3] = (byte)((x + 2 * y) % 5 == 0 ? 90 : 255);
        }
        resources.Add("stripes", ImageContent.FromPixels(bytes, w, h), ResourceOrigin.None, ResourceColorSpace.Srgb, out _, ImageId);
        return resources;
    }
    static PaintDocument NewDoc()
    {
        var d = new PaintDocument(W, H, TS, 256L << 20);
        d.ImageResources = Images();
        return d;
    }
    static PathBrush PathBrushOf() => new PathBrush { RadiusWorld = 3, Hardness = .7, Spacing = .17, Opacity = .83, Flow = .42, Color = new Rgba32(201, 37, 89, 219), PressureSize = true, PressureOpacity = false, PressureFlow = false, Erase = false };
    static CanvasPath PathPoints(double shift) => new CanvasPath(Guid.NewGuid(), PaintChannel.Color, PathBrushOf(),
        new[] { new CanvasPoint(4.5 + shift, 7.25, .3), new CanvasPoint(29.5, 21.5 - shift, .9), new CanvasPoint(37.25, 5.75, .55) });
    static GeneratorSettings AnchorGenerator() => GeneratorSettings.Default(GeneratorType.Anchor).WithBlend(GeneratorBlend.Replace);

    sealed class Rig
    {
        public PaintDocument D; public Guid Base, Mid, Top, Fill, PlainFill, PathLayer, Blur, MaskBlur, Reader, Anchor;
    }
    /// 効果をひと通り持つ文書（Rust 側 seam_golden.rs の rig と同じ）。
    static Rig MakeRig()
    {
        var d = NewDoc(); var r = new Rig { D = d };
        r.Base = d.AddLayer("土台").Id; Paint(d, r.Base, PaintChannel.Color, 1); Paint(d, r.Base, PaintChannel.Height, 2);
        r.Mid = d.AddLayer("中").Id; Paint(d, r.Mid, PaintChannel.Height, 3);
        r.Top = d.AddLayer("上").Id; Paint(d, r.Top, PaintChannel.Color, 4); Paint(d, r.Top, PaintChannel.Height, 5);
        r.Fill = d.AddFillLayer("塗り", new Dictionary<PaintChannel, Rgba32> { { PaintChannel.Color, new Rgba32(60, 160, 220, 200) }, { PaintChannel.Height, new Rgba32(128, 128, 128, 100) } }).Id;
        r.Blur = d.AddFilter(r.Base, FilterTarget.Content, FilterSettings.GaussianBlur(3), new[] { PaintChannel.Color }).Id;
        r.Anchor = d.AddAnchor(r.Base, AnchorPlacement.Layer, "土台").Id;
        r.Reader = d.AddFilter(r.Mid, FilterTarget.Content, FilterSettings.FromGenerator(AnchorGenerator()), new[] { PaintChannel.Height }).Id;
        d.SetGeneratorAnchor(r.Mid, r.Reader, r.Anchor, PaintChannel.Height, AnchorRead.Value);
        d.AddLayerMask(r.Top);
        r.MaskBlur = d.AddFilter(r.Top, FilterTarget.Mask, FilterSettings.GaussianBlur(2)).Id;
        d.SetFillImage(r.Fill, PaintChannel.Color, ImageId);
        r.PlainFill = d.AddFillLayer("素の塗り", new Dictionary<PaintChannel, Rgba32> { { PaintChannel.Color, new Rgba32(5, 6, 7, 255) } }).Id;
        r.PathLayer = d.AddLayer("パス").Id;
        d.SetCanvasPath(r.PathLayer, PathPoints(0));
        d.ClearHistory();
        return r;
    }

    // ───────── 書き出し ─────────

    sealed class Ids
    {
        readonly Dictionary<Guid, int> filters = new Dictionary<Guid, int>(), anchors = new Dictionary<Guid, int>();
        public int Filter(Guid g) { if (g == Guid.Empty) return -1; if (!filters.TryGetValue(g, out var n)) filters[g] = n = filters.Count; return n; }
        public int Anchor(Guid g) { if (g == Guid.Empty) return -1; if (!anchors.TryGetValue(g, out var n)) anchors[g] = n = anchors.Count; return n; }
    }
    static void Effects(BinaryWriter b, IReadOnlyList<FilterEffect> list, Ids ids)
    {
        b.Write(list.Count);
        foreach (var e in list)
        {
            var s = e.Settings;
            b.Write((int)s.Type); b.Write(s.Type == FilterType.GaussianBlur || s.Type == FilterType.Sharpen ? s.Radius : 0);
            b.Write(e.Enabled); b.Write(e.Strength);
            int mask = 0; foreach (var c in e.Channels) mask |= 1 << (int)c; b.Write(mask);
            b.Write(ids.Filter(e.Id));
            if (s.IsGenerator)
            {
                b.Write((int)s.Generator.Type);
                if (s.Generator.Type == GeneratorType.Anchor) { b.Write(ids.Anchor(s.Generator.AnchorId)); b.Write((int)s.Generator.AnchorChannel); b.Write((int)s.Generator.AnchorRead); }
            }
        }
    }
    static void AnchorOf(BinaryWriter b, AnchorPoint a, Ids ids)
    {
        b.Write(a != null);
        if (a != null) { b.Write(ids.Anchor(a.Id)); b.Write(a.Name); b.Write((int)a.Placement); }
    }
    static void Snapshot(BinaryWriter b, PaintDocument d, Ids ids)
    {
        var layers = d.Layers.ToList();
        b.Write(d.Width); b.Write(d.Height); b.Write(layers.Count);
        foreach (var l in layers)
        {
            b.Write((int)l.Kind); b.Write(l.ParentId == Guid.Empty ? -1 : layers.FindIndex(p => p.Id == l.ParentId));
            b.Write(l.Visible); b.Write(l.Opacity); b.Write((int)l.BlendMode); b.Write(l.Clipping); b.Write((int)l.Locks);
            foreach (var c in All) b.Write(l.IsChannelEnabled(c));
            b.Write(l.Path != null);
            if (l.Path != null)
            {
                b.Write((int)l.Path.Channel); b.Write(l.Path.PointCount);
                var canvas = l.Path as CanvasPath; b.Write(canvas != null);
                if (canvas != null)
                {
                    b.Write(canvas.Brush.RadiusWorld);
                    foreach (var p in canvas.Points) { b.Write(p.X); b.Write(p.Y); b.Write(p.Pressure); }
                }
            }
            Effects(b, l.Filters, ids);
            AnchorOf(b, l.Anchor, ids);
            b.Write(l.Mask != null);
            if (l.Mask != null)
            {
                b.Write(l.Mask.Enabled); b.Write(l.Mask.Inverted); b.Write(l.Mask.Density);
                Effects(b, l.Mask.Filters, ids); AnchorOf(b, l.Mask.Anchor, ids);
            }
        }
        foreach (var c in All) b.Write(d.Composite(c));
    }
    static void Report(BinaryWriter b, LayerMergeReport r)
    {
        b.Write((int)r.Method); b.Write((int)r.Notes); b.Write(r.ComparedPixels); b.Write(r.ChangedPixels); b.Write(r.MaxDifference); b.Write(r.MaxVisibleDifference);
        foreach (var c in All) b.Write(r.ChangedByChannel.TryGetValue(c, out var n) ? n : 0L);
    }
    /// 結果の型: 0 成功、1 ロックで断られた（レイヤー・持ち主・ロック）、2 そのほか（理由の名前）。
    static readonly bool Trace = Environment.GetEnvironmentVariable("SEAM_TRACE") == "1";
    static void Note(string what) { if (Trace) Console.Error.WriteLine("  " + what); }
    static bool Outcome(BinaryWriter o, PaintDocument d, Action act)
    {
        try { act(); o.Write((byte)0); Note("ok"); return true; }
        catch (LayerLockedException e)
        {
            o.Write((byte)1); var all = d.Layers.ToList(); Note("locked " + e.Lock + " holder=" + all.FindIndex(l => l.Id == e.LockedBy));
            o.Write(all.FindIndex(l => l.Id == e.LayerId)); o.Write(all.FindIndex(l => l.Id == e.LockedBy)); o.Write((int)e.Lock);
        }
        catch (LayerOpException e) { o.Write((byte)2); o.Write("op:" + e.Reason); Note("op:" + e.Reason); }
        catch (InvalidOperationException e) { o.Write((byte)2); o.Write("invalid-op"); Note("invalid-op " + e.Message); }
        catch (ArgumentException e) { o.Write((byte)2); o.Write("argument"); Note("argument " + e.Message); }
        catch (Exception e) { o.Write((byte)2); o.Write("other:" + e.GetType().Name); Note("other " + e); }
        return false;
    }
    static byte[] Finish(MemoryStream m, BinaryWriter b) { b.Flush(); return m.ToArray(); }
    static void AfterOp(BinaryWriter w, PaintDocument d, Ids ids, bool ok)
    {
        w.Write(d.HistoryBytes); Snapshot(w, d, ids);
        if (!ok) return;
        d.Undo(); w.Write(d.HistoryBytes); Snapshot(w, d, ids);
        d.Redo(); w.Write(d.HistoryBytes); Snapshot(w, d, ids);
    }

    // ───────── 効果の設定の入口 × ロック ─────────

    sealed class Entry { public string Name; public Func<Rig, Guid> Target; public Action<Rig> Run; }
    static Entry E(string name, Func<Rig, Guid> target, Action<Rig> run) => new Entry { Name = name, Target = target, Run = run };
    static GeneratorSettings Gradient() => GeneratorSettings.Default(GeneratorType.ShapeGradient).WithRamp(GradientRamp.Default).WithBlend(GeneratorBlend.Replace);
    static List<Entry> Entries() => new List<Entry>
    {
        E("add_filter", r => r.Base, r => r.D.AddFilter(r.Base, FilterTarget.Content, FilterSettings.Invert(), new[] { PaintChannel.Height })),
        E("remove_filter", r => r.Base, r => r.D.RemoveFilter(r.Base, r.Blur)),
        E("set_filter_enabled", r => r.Base, r => r.D.SetFilterEnabled(r.Base, r.Blur, false)),
        E("set_filter_strength", r => r.Base, r => r.D.SetFilterStrength(r.Base, r.Blur, .5, false)),
        E("set_filter_settings", r => r.Base, r => r.D.SetFilterSettings(r.Base, r.Blur, FilterSettings.GaussianBlur(7), false)),
        E("set_filter_channels", r => r.Base, r => r.D.SetFilterChannels(r.Base, r.Blur, new[] { PaintChannel.Color, PaintChannel.Height })),
        E("move_filter", r => r.Base, r => { r.D.AddFilter(r.Base, FilterTarget.Content, FilterSettings.Invert(), new[] { PaintChannel.Height }); r.D.MoveFilter(r.Base, r.Blur, 1); }),
        E("mask_add_filter", r => r.Top, r => r.D.AddFilter(r.Top, FilterTarget.Mask, FilterSettings.Invert())),
        E("mask_set_filter_strength", r => r.Top, r => r.D.SetFilterStrength(r.Top, r.MaskBlur, .4, false)),
        E("add_anchor", r => r.Mid, r => r.D.AddAnchor(r.Mid, AnchorPlacement.Layer)),
        // 名前は渡す（名前を省くときの既定は、C# が「レイヤー名 (mask)」、Rust は日本語の画面に合わせて「レイヤー名（マスク）」）
        E("add_anchor_mask", r => r.Top, r => r.D.AddAnchor(r.Top, AnchorPlacement.Mask, "上のマスク")),
        E("remove_anchor", r => r.Base, r => r.D.RemoveAnchor(r.Anchor)),
        E("rename_anchor", r => r.Base, r => r.D.RenameAnchor(r.Anchor, "別の名前")),
        E("set_generator_anchor", r => r.Mid, r => r.D.SetGeneratorAnchor(r.Mid, r.Reader, Guid.Empty, PaintChannel.Height, AnchorRead.Value)),
        E("set_fill_image_off", r => r.Fill, r => r.D.SetFillImage(r.Fill, PaintChannel.Color, null)),
        E("set_fill_gradient", r => r.Fill, r => r.D.SetFillGradient(r.Fill, PaintChannel.Height, Gradient(), false)),
        E("set_fill_projection_image", r => r.Fill, r => r.D.SetFillProjection(r.Fill, FillProjection.Default.WithMode(FillProjectionMode.Planar).WithTiles(2, 2), false)),
        E("set_fill_projection_decal", r => r.Fill, r =>
        {
            r.D.SetFillImage(r.Fill, PaintChannel.Color, null);
            r.D.SetFillProjection(r.Fill, FillProjection.Default.WithMode(FillProjectionMode.Decal), false);
        }),
        E("set_fill_projection_plain", r => r.PlainFill, r => r.D.SetFillProjection(r.PlainFill, FillProjection.Default.WithMode(FillProjectionMode.Planar).WithTiles(3, 3), false)),
        E("set_fill_value_alpha", r => r.Fill, r => r.D.SetFillValue(r.Fill, PaintChannel.Color, new Rgba32(1, 2, 3, 77), false)),
        E("set_fill_value_colour", r => r.Fill, r => r.D.SetFillValue(r.Fill, PaintChannel.Color, new Rgba32(1, 2, 3, 200), false)),
        E("set_fill_value_clear", r => r.Fill, r => r.D.SetFillValue(r.Fill, PaintChannel.Color, null, false)),
        E("set_canvas_path", r => r.PathLayer, r => r.D.SetCanvasPath(r.PathLayer, PathPoints(2))),
        E("set_path", r => r.PathLayer, r => { var p = PathPoints(3); r.D.SetPath(r.PathLayer, p, CanvasPathRenderer.RenderChannels(r.D, p)); }),
        E("rasterize", r => r.PathLayer, r => r.D.Rasterize(r.PathLayer)),
        E("set_channel_enabled_off", r => r.Base, r => r.D.SetChannelEnabled(r.Base, PaintChannel.Height, false)),
        E("set_channel_enabled_noop", r => r.Base, r => r.D.SetChannelEnabled(r.Base, PaintChannel.Height, true)),
    };
    static byte[] FxLock(Entry e, LayerLocks lockFlag, bool grouped)
    {
        var r = MakeRig(); var d = r.D; var ids = new Ids();
        var target = e.Target(r);
        var holder = grouped ? d.GroupLayers(new[] { target }, "親").Id : target;
        d.SetLayerLocks(holder, lockFlag); d.ClearHistory();
        var m = new MemoryStream(); var w = new BinaryWriter(m);
        Snapshot(w, d, ids);
        bool ok = Outcome(w, d, () => e.Run(r));
        AfterOp(w, d, ids, ok);
        return Finish(m, w);
    }

    // ───────── 手の書き込み × ロック × パスレイヤー ─────────

    static BrushSettings Hard() => new BrushSettings { Radius = 3, Hardness = 1 };
    static ChannelPaint[] Two() => new[] { new ChannelPaint(PaintChannel.Color, new Rgba32(10, 200, 30, 255)), new ChannelPaint(PaintChannel.Emission, new Rgba32(90, 90, 90, 255)) };
    static readonly string[] HandNames = { "begin_stroke", "begin_stroke_erase", "begin_material_stroke", "begin_triangle_fill", "fill", "fill_erase", "fill_material", "gradient_material" };
    static void HandRun(int n, PaintDocument d, Guid id)
    {
        switch (n)
        {
            case 0: using (d.BeginStroke(id, PaintChannel.Color, Hard())) { } break;
            case 1: using (d.BeginStroke(id, PaintChannel.Color, new BrushSettings { Radius = 3, Hardness = 1, Erase = true })) { } break;
            case 2: using (d.BeginMaterialStroke(id, Two(), Hard())) { } break;
            case 3: d.BeginTriangleFill(id, PaintChannel.Color, new Rgba32(1, 2, 3, 255), 1, false).Stroke.Cancel(); break;
            case 4: d.Fill(id, PaintChannel.Color, new Rgba32(1, 2, 3, 255), 1, null, false); break;
            case 5: d.Fill(id, PaintChannel.Color, new Rgba32(1, 2, 3, 255), 1, null, true); break;
            case 6: d.FillMaterial(id, Two(), 1, null, false); break;
            default: d.GradientMaterial(id, Two(), new GradientSettings { X1 = 8, Y1 = 6 }, null, false); break;
        }
    }
    static byte[] HandLock(int n, LayerLocks lockFlag, bool grouped, bool onPathLayer)
    {
        var r = MakeRig(); var d = r.D; var ids = new Ids();
        var id = onPathLayer ? r.PathLayer : r.Base;
        var holder = grouped ? d.GroupLayers(new[] { id }, "親").Id : id;
        d.SetLayerLocks(holder, lockFlag); d.ClearHistory();
        var m = new MemoryStream(); var w = new BinaryWriter(m);
        Snapshot(w, d, ids);
        Outcome(w, d, () => HandRun(n, d, id));
        w.Write(d.HistoryBytes); w.Write(d.HasActiveStroke); Snapshot(w, d, ids);
        return Finish(m, w);
    }
    static byte[] HandTransform(int n, LayerLocks lockFlag, bool grouped)
    {
        var r = MakeRig(); var d = r.D; var ids = new Ids();
        var holder = grouped ? d.GroupLayers(new[] { r.PathLayer }, "親").Id : r.PathLayer;
        d.SetLayerLocks(holder, lockFlag); d.ClearHistory();
        var m = new MemoryStream(); var w = new BinaryWriter(m);
        Snapshot(w, d, ids);
        bool ok = Outcome(w, d, () =>
        {
            if (n == 0) d.Transform(r.PathLayer, Affine2D.Translation(2, 1), null, Resampling.Nearest, true);
            else d.TransformLayers(new[] { holder }, Affine2D.Translation(2, 1), Resampling.Nearest);
        });
        AfterOp(w, d, ids, ok);
        return Finish(m, w);
    }
    static byte[] GroupPath(LayerLocks lockFlag)
    {
        var r = MakeRig(); var d = r.D; var ids = new Ids();
        var inner = d.AddLayer("中身").Id; var group = d.GroupLayers(new[] { inner }, "グループ").Id;
        d.SetLayerLocks(group, lockFlag); d.ClearHistory();
        var m = new MemoryStream(); var w = new BinaryWriter(m);
        Snapshot(w, d, ids);
        var path = PathPoints(0);
        bool ok = Outcome(w, d, () => d.AddPathLayer("新", path, CanvasPathRenderer.RenderChannels(d, path), inner));
        AfterOp(w, d, ids, ok);
        return Finish(m, w);
    }

    // ───────── レイヤーの操作と効果 ─────────

    static Affine2D Rotation() => Affine2D.FromParts(20, 14, 1, 0, 17, 1.2, .8);
    static (Guid lower, Guid upper) Pair(PaintDocument d)
    {
        var lower = d.AddLayer("下").Id; Paint(d, lower, PaintChannel.Color, 11);
        var upper = d.AddLayer("上").Id;
        var s = Lay(d, upper).GetChannel(PaintChannel.Color);
        for (int y = 5; y < 8; y++) for (int x = 5; x < 8; x++) s.SetPixel(x, y, new Rgba32(240, 30, 60, 255));
        return (lower, upper);
    }
    static FilterEffect Blur(PaintDocument d, Guid layer, int radius) => d.AddFilter(layer, FilterTarget.Content, FilterSettings.GaussianBlur(radius), new[] { PaintChannel.Color });
    static Guid ReaderLayer(PaintDocument d, Guid anchor, string name = "読む層")
    {
        var layer = d.AddLayer(name).Id; Paint(d, layer, PaintChannel.Color, 21);
        var reader = d.AddFilter(layer, FilterTarget.Content, FilterSettings.FromGenerator(AnchorGenerator()), new[] { PaintChannel.Height });
        d.SetGeneratorAnchor(layer, reader.Id, anchor, PaintChannel.Height, AnchorRead.Value);
        return layer;
    }

    /// 文書を作って操作を 1 つ行い、結果・履歴・前後の文書（Undo と Redo のあとも）を書く。`run` が報告を返せば書く。
    static byte[] Op(Func<(PaintDocument d, Func<LayerMergeReport> act)> build)
    {
        var (d, act) = build();
        d.ClearHistory();
        var m = new MemoryStream(); var w = new BinaryWriter(m); var ids = new Ids();
        Snapshot(w, d, ids);
        LayerMergeReport report = null;
        bool ok = Outcome(w, d, () => report = act());
        if (report != null) Report(w, report);
        AfterOp(w, d, ids, ok);
        return Finish(m, w);
    }
    static byte[] OpNoReport(Func<(PaintDocument d, Action act)> build) => Op(() => { var (d, act) = build(); return (d, () => { act(); return null; }); });

    static byte[] ResizeCase(Func<PaintDocument> build, int w, int h, CanvasResampling mode)
    {
        var d = build(); var m = new MemoryStream(); var o = new BinaryWriter(m); var ids = new Ids();
        Snapshot(o, d, ids);
        ResampledDocument result = null;
        bool ok = Outcome(o, d, () => result = d.Resampled(w, h, mode));
        if (ok)
        {
            o.Write(result.Notes.Count); o.Write(result.SurfacePathLayers.Count);
            var layers = result.Document.Layers.ToList();
            foreach (var id in result.SurfacePathLayers) o.Write(layers.FindIndex(l => l.Id == id));
            Snapshot(o, result.Document, ids);
        }
        Snapshot(o, d, ids); // 元の文書は変わらない
        return Finish(m, o);
    }

    static PaintDocument WorldWithBlurs()
    {
        var r = MakeRig(); var d = r.D;
        Blur(d, r.Base, 200); Blur(d, r.Base, 1);
        d.AddFilter(r.Top, FilterTarget.Mask, FilterSettings.GaussianBlur(5));
        return d;
    }
    static PaintDocument WorldWithManyBlurs()
    {
        var r = MakeRig(); var d = r.D;
        for (int i = 0; i < 3; i++) Blur(d, r.Base, 150);
        return d;
    }
    static PaintDocument WithSurfacePath()
    {
        var r = MakeRig(); var d = r.D;
        var layer = d.AddLayer("面のパス").Id; d.SetChannelEnabled(layer, PaintChannel.Height, true);
        var brush = new PathBrush { RadiusWorld = .1, Hardness = .7, Spacing = .17, Opacity = .83, Flow = .42, Color = new Rgba32(201, 37, 89, 219), PressureSize = true, PressureOpacity = false, PressureFlow = false, Erase = false };
        var rendered = new SparseTileSurface(W, H, TS); PaintSurface(rendered, 77);
        d.SetPath(layer, new SurfacePath(Guid.NewGuid(), PaintChannel.Height, "synthetic-model", brush, new[] { new PathPoint(0, .2, .3, 1), new PathPoint(1, .4, .2, 1) }), rendered);
        return d;
    }

    static IEnumerable<(string name, Func<byte[]> run)> Cases()
    {
        var locks = new[] { LayerLocks.None, LayerLocks.Transparency, LayerLocks.Pixels, LayerLocks.Position, LayerLocks.All };
        var entries = Entries();
        for (int e = 0; e < entries.Count; e++)
            foreach (var l in locks) foreach (var g in new[] { false, true })
            {
                int ee = e; var ll = l; var gg = g;
                yield return ("fxlock-" + entries[ee].Name + "-" + (int)ll + "-" + (gg ? "group" : "layer"), () => FxLock(entries[ee], ll, gg));
            }
        for (int n = 0; n < HandNames.Length; n++)
            foreach (var l in locks) foreach (var g in new[] { false, true }) foreach (var onPath in new[] { true, false })
            {
                int nn = n; var ll = l; var gg = g; var pp = onPath;
                yield return ("hand-" + HandNames[nn] + "-" + (int)ll + "-" + (gg ? "group" : "layer") + (pp ? "-path" : "-raster"), () => HandLock(nn, ll, gg, pp));
            }
        for (int n = 0; n < 2; n++)
            foreach (var l in locks) foreach (var g in new[] { false, true })
            {
                int nn = n; var ll = l; var gg = g;
                yield return ("xform-" + (nn == 0 ? "layer" : "layers") + "-" + (int)ll + "-" + (gg ? "group" : "layer"), () => HandTransform(nn, ll, gg));
            }
        foreach (var l in locks) { var ll = l; yield return ("grouppath-" + (int)ll, () => GroupPath(ll)); }

        // 複製・削除・変形
        yield return ("dup-rig", () => OpNoReport(() => { var r = MakeRig(); return (r.D, () => r.D.DuplicateLayers(new[] { r.Base, r.Mid, r.Top, r.PathLayer })); }));
        yield return ("dup-group", () => OpNoReport(() => { var r = MakeRig(); var g = r.D.GroupLayers(new[] { r.Base, r.Mid }, "組").Id; return (r.D, () => r.D.DuplicateLayers(new[] { g })); }));
        yield return ("dup-reader-alone", () => OpNoReport(() => { var r = MakeRig(); return (r.D, () => r.D.DuplicateLayers(new[] { r.Mid })); }));
        yield return ("dup-single-reader", () => OpNoReport(() => { var r = MakeRig(); return (r.D, () => r.D.DuplicateLayer(r.Mid)); }));
        yield return ("dup-single-base", () => OpNoReport(() => { var r = MakeRig(); return (r.D, () => r.D.DuplicateLayer(r.Base)); }));
        yield return ("dup-single-path", () => OpNoReport(() => { var r = MakeRig(); return (r.D, () => r.D.DuplicateLayer(r.PathLayer)); }));
        yield return ("remove-base", () => OpNoReport(() => { var r = MakeRig(); return (r.D, () => r.D.RemoveLayers(new[] { r.Base })); }));
        yield return ("remove-group", () => OpNoReport(() => { var r = MakeRig(); var g = r.D.GroupLayers(new[] { r.Base, r.Mid }, "組").Id; return (r.D, () => r.D.RemoveLayers(new[] { g })); }));
        yield return ("transform-translate", () => OpNoReport(() => { var r = MakeRig(); return (r.D, () => r.D.TransformLayers(new[] { r.Base, r.Top }, Affine2D.Translation(3, -2), Resampling.Bilinear)); }));
        yield return ("transform-rotate", () => OpNoReport(() => { var r = MakeRig(); return (r.D, () => r.D.TransformLayers(new[] { r.Base, r.Top }, Rotation(), Resampling.Bilinear)); }));
        yield return ("transform-with-path", () => OpNoReport(() => { var r = MakeRig(); return (r.D, () => r.D.TransformLayers(new[] { r.Base, r.PathLayer }, Rotation(), Resampling.Bilinear)); }));
        yield return ("transform-path-group", () => OpNoReport(() => { var r = MakeRig(); var g = r.D.GroupLayers(new[] { r.PathLayer }, "組").Id; return (r.D, () => r.D.TransformLayers(new[] { g }, Rotation(), Resampling.Bilinear)); }));
        yield return ("transform-single-filtered", () => OpNoReport(() => { var r = MakeRig(); return (r.D, () => r.D.Transform(r.Base, Rotation(), null, Resampling.Bilinear, true)); }));
        yield return ("transform-single-path", () => OpNoReport(() => { var r = MakeRig(); return (r.D, () => r.D.Transform(r.PathLayer, Rotation(), null, Resampling.Bilinear, true)); }));

        // パスで描かれたチャンネルを無効にする: 組を持たないパスは断る。組を持つパスは基準のチャンネルも組のチャンネルも無効にできる
        yield return ("channel-off-path-plain", () => OpNoReport(() => { var r = MakeRig(); return (r.D, () => r.D.SetChannelEnabled(r.PathLayer, PaintChannel.Color, false)); }));
        yield return ("channel-off-path-material-base", () => OpNoReport(() => { var r = MakeRig(); r.D.SetCanvasPath(r.PathLayer, PathPoints(0).WithMaterial(Two())); return (r.D, () => r.D.SetChannelEnabled(r.PathLayer, PaintChannel.Color, false)); }));
        yield return ("channel-off-path-material-other", () => OpNoReport(() => { var r = MakeRig(); r.D.SetCanvasPath(r.PathLayer, PathPoints(0).WithMaterial(Two())); return (r.D, () => r.D.SetChannelEnabled(r.PathLayer, PaintChannel.Emission, false)); }));

        // 並べ替え・表示・不透明度・クリッピング・選択範囲の変形（効果を持つレイヤー・Anchor を読むレイヤーとの組み合わせ）
        yield return ("move-reader-below", () => OpNoReport(() => { var r = MakeRig(); return (r.D, () => r.D.MoveLayers(new[] { r.Mid }, Guid.Empty, 0)); }));
        yield return ("move-reader-into-group", () => OpNoReport(() => { var r = MakeRig(); var g = r.D.GroupLayers(new[] { r.Top }, "組").Id; return (r.D, () => r.D.MoveLayers(new[] { r.Mid }, g, 0)); }));
        yield return ("step-base-up", () => OpNoReport(() => { var r = MakeRig(); return (r.D, () => r.D.StepLayers(new[] { r.Base }, true)); }));
        yield return ("hide-base", () => OpNoReport(() => { var r = MakeRig(); return (r.D, () => r.D.SetLayerVisibility(r.Base, false)); }));
        yield return ("opacity-base", () => OpNoReport(() => { var r = MakeRig(); return (r.D, () => r.D.SetLayerOpacity(r.Base, .5)); }));
        yield return ("clip-reader", () => OpNoReport(() => { var r = MakeRig(); return (r.D, () => r.D.SetLayerClipping(r.Mid, true)); }));
        yield return ("region-filtered", () => OpNoReport(() => { var r = MakeRig(); return (r.D, () => r.D.Transform(r.Base, Rotation(), SelectionMask.Rectangle(r.D, 5, 1, 16, 8), Resampling.Bilinear)); }));
        yield return ("region-path", () => OpNoReport(() => { var r = MakeRig(); return (r.D, () => r.D.Transform(r.PathLayer, Rotation(), SelectionMask.Rectangle(r.D, 5, 1, 16, 8), Resampling.Bilinear)); }));

        // 結合
        yield return ("merge-down-clipped-blur", () => Op(() =>
        {
            var d = NewDoc(); var (lower, upper) = Pair(d); Blur(d, upper, 3); Blur(d, lower, 2);
            d.SetLayerClipping(upper, true);
            return (d, () => d.MergeDown(upper, 255));
        }));
        yield return ("merge-down-blur", () => Op(() =>
        {
            var d = NewDoc(); var (lower, upper) = Pair(d); Blur(d, upper, 6);
            return (d, () => d.MergeDown(upper, 2));
        }));
        yield return ("merge-down-isolated-mask", () => Op(() =>
        {
            var d = NewDoc(); var (lower, upper) = Pair(d);
            d.AddLayerMask(lower);
            var mask = Lay(d, lower).Mask;
            for (int y = 0; y < H; y++) for (int x = 0; x < W; x++) if ((x + y) % 3 == 0) mask.Surface.SetPixel(x, y, new Rgba32(0, 0, 0, 140));
            d.AddFilter(lower, FilterTarget.Mask, FilterSettings.GaussianBlur(2));
            d.AddFilter(upper, FilterTarget.Content, FilterSettings.Levels(.1, .9, 1.2, 0, 1), new[] { PaintChannel.Color });
            d.SetLayerOpacity(lower, .5);
            return (d, () => d.MergeDown(upper, 0));
        }));
        yield return ("merge-down-kept-mask", () => Op(() =>
        {
            var d = NewDoc(); var (lower, upper) = Pair(d);
            d.AddLayerMask(lower);
            var mask = Lay(d, lower).Mask;
            for (int y = 0; y < H; y++) for (int x = 28; x < W; x++) mask.Surface.SetPixel(x, y, new Rgba32(0, 0, 0, 200));
            d.AddFilter(lower, FilterTarget.Mask, FilterSettings.GaussianBlur(2));
            var below = d.AddLayer("一番下").Id; Lay(d, below).GetChannel(PaintChannel.Color).SetPixel(3, 3, new Rgba32(1, 2, 3, 255));
            d.MoveLayer(below, 0);
            return (d, () => d.MergeDown(upper, 2));
        }));
        yield return ("merge-down-anchors", () => Op(() =>
        {
            var d = NewDoc(); var (lower, upper) = Pair(d); Blur(d, upper, 2);
            d.AddAnchor(lower, AnchorPlacement.Layer, "下");
            var upperAnchor = d.AddAnchor(upper, AnchorPlacement.Layer, "上").Id;
            ReaderLayer(d, upperAnchor);
            return (d, () => d.MergeDown(upper, 2));
        }));
        yield return ("merge-down-lower-anchor-read", () => Op(() =>
        {
            var d = NewDoc(); var (lower, upper) = Pair(d); Blur(d, upper, 2);
            var lowerAnchor = d.AddAnchor(lower, AnchorPlacement.Layer, "下").Id;
            ReaderLayer(d, lowerAnchor);
            return (d, () => d.MergeDown(upper, 2));
        }));
        yield return ("merge-visible-generator", () => Op(() =>
        {
            var r = MakeRig(); var d = r.D;
            var layer = d.AddLayer("空の層").Id;
            var f = d.AddFilter(layer, FilterTarget.Content, FilterSettings.FromGenerator(AnchorGenerator()), new[] { PaintChannel.Height });
            d.SetGeneratorAnchor(layer, f.Id, r.Anchor, PaintChannel.Height, AnchorRead.Value);
            return (d, () => d.MergeVisible("merged", null, 2));
        }));
        yield return ("merge-visible-rig", () => Op(() => { var r = MakeRig(); return (r.D, () => r.D.MergeVisible("merged", null, 255)); }));
        // 土台の Anchor を読む段は外しておく（結合で土台の Anchor が無くなると読む段は入力のまま通すようになり、見た目が変わる。C# の結合の
        // 報告は、その変化を数えそこなう: 派生の Anchor のキャッシュが古い。Rust は数える。crates/yolu-core/tests/effects/seam_ops.rs で確かめる）
        yield return ("merge-down-path-onto-base", () => Op(() => { var r = MakeRig(); var d = r.D; d.RemoveFilter(r.Mid, r.Reader); d.MoveLayer(r.PathLayer, 1); return (d, () => d.MergeDown(r.PathLayer, 255)); }));
        yield return ("merge-group-path", () => Op(() =>
        {
            var r = MakeRig(); var d = r.D;
            var g = d.GroupLayers(new[] { r.Base, r.PathLayer }, "グループ").Id;
            d.AddAnchor(g, AnchorPlacement.Layer, "グループ");
            return (d, () => d.MergeGroup(g, 255));
        }));
        yield return ("merge-layers-path", () => Op(() => { var r = MakeRig(); r.D.RemoveFilter(r.Mid, r.Reader); return (r.D, () => r.D.MergeLayers(new[] { r.Base, r.PathLayer }, 255)); }));
        yield return ("merge-layers-mask-filter", () => Op(() => { var r = MakeRig(); return (r.D, () => r.D.MergeLayers(new[] { r.Mid, r.Top }, 255)); }));

        // 効いていない効果は焼かない
        yield return ("merge-inactive-no-anchor", () => Op(() =>
        {
            var d = NewDoc(); var (lower, upper) = Pair(d);
            d.AddFilter(upper, FilterTarget.Content, FilterSettings.FromGenerator(AnchorGenerator()), new[] { PaintChannel.Height });
            return (d, () => d.MergeDown(upper, 255));
        }));
        yield return ("merge-inactive-lower-content", () => Op(() =>
        {
            var d = NewDoc(); var (lower, upper) = Pair(d);
            d.AddFilter(lower, FilterTarget.Content, FilterSettings.FromGenerator(AnchorGenerator()), new[] { PaintChannel.Height });
            return (d, () => d.MergeDown(upper, 255));
        }));
        // 下のレイヤーの下に見えるレイヤーを置いて、分離の結合にしない（マスクが効果ごと結果に残る結合）。分離の結合は下のマスクのフィルターも焼くが、
        // C# はそのマスクの効いていない Generator を見ずに黙って落とす。Rust は意図して断る（crates/yolu-core/tests/effects/seam_ops.rs で確かめる）
        yield return ("merge-inactive-lower-mask", () => Op(() =>
        {
            var d = NewDoc(); var (lower, upper) = Pair(d);
            var below = d.AddLayer("下の下").Id; Paint(d, below, PaintChannel.Color, 12); d.MoveLayers(new[] { below }, Guid.Empty, 0);
            d.AddLayerMask(lower);
            d.AddFilter(lower, FilterTarget.Mask, FilterSettings.FromGenerator(AnchorGenerator()));
            return (d, () => d.MergeDown(upper, 255));
        }));
        yield return ("merge-inactive-needs-map", () => Op(() =>
        {
            var d = NewDoc(); var (lower, upper) = Pair(d);
            d.AddFilter(upper, FilterTarget.Content, FilterSettings.FromGenerator(Gradient()), new[] { PaintChannel.Color });
            return (d, () => d.MergeDown(upper, 255));
        }));
        yield return ("merge-inactive-disabled-passes", () => Op(() =>
        {
            var d = NewDoc(); var (lower, upper) = Pair(d);
            d.AddFilter(upper, FilterTarget.Content, FilterSettings.FromGenerator(AnchorGenerator()), new[] { PaintChannel.Height }, -1, null, false);
            return (d, () => d.MergeDown(upper, 255));
        }));
        yield return ("merge-inactive-visible", () => Op(() =>
        {
            var d = NewDoc(); var (lower, upper) = Pair(d);
            d.AddFilter(upper, FilterTarget.Content, FilterSettings.FromGenerator(AnchorGenerator()), new[] { PaintChannel.Height });
            return (d, () => d.MergeVisible("merged", null, 255));
        }));
        yield return ("merge-inactive-group", () => Op(() =>
        {
            var d = NewDoc(); var (lower, upper) = Pair(d);
            d.AddFilter(upper, FilterTarget.Content, FilterSettings.FromGenerator(AnchorGenerator()), new[] { PaintChannel.Height });
            var g = d.GroupLayers(new[] { lower, upper }, "組").Id;
            return (d, () => d.MergeGroup(g, 255));
        }));
        yield return ("merge-inactive-layers", () => Op(() =>
        {
            var d = NewDoc(); var (lower, upper) = Pair(d);
            d.AddFilter(upper, FilterTarget.Content, FilterSettings.FromGenerator(AnchorGenerator()), new[] { PaintChannel.Height });
            return (d, () => d.MergeLayers(new[] { lower, upper }, 255));
        }));
        // ロックとの順: 画像のロックは効いていない効果より先
        yield return ("merge-inactive-then-lock", () => Op(() =>
        {
            var d = NewDoc(); var (lower, upper) = Pair(d);
            d.AddFilter(upper, FilterTarget.Content, FilterSettings.FromGenerator(AnchorGenerator()), new[] { PaintChannel.Height });
            d.SetLayerLocks(lower, LayerLocks.Pixels);
            return (d, () => d.MergeDown(upper, 255));
        }));

        // 画像の大きさ
        yield return ("resize-blurs-up", () => ResizeCase(WorldWithBlurs, 80, 56, CanvasResampling.Bilinear));
        yield return ("resize-blurs-down", () => ResizeCase(WorldWithBlurs, 10, 7, CanvasResampling.Area));
        yield return ("resize-blurs-nearest", () => ResizeCase(WorldWithBlurs, 61, 43, CanvasResampling.Nearest));
        yield return ("resize-too-far", () => ResizeCase(WorldWithManyBlurs, 60, 42, CanvasResampling.Nearest));
        yield return ("resize-too-far-down", () => ResizeCase(WorldWithManyBlurs, 30, 21, CanvasResampling.Nearest));
        yield return ("resize-rig-up", () => ResizeCase(() => MakeRig().D, 80, 56, CanvasResampling.Bilinear));
        yield return ("resize-rig-odd", () => ResizeCase(() => MakeRig().D, 53, 31, CanvasResampling.Area));
        yield return ("resize-rig-down", () => ResizeCase(() => MakeRig().D, 20, 14, CanvasResampling.Bilinear));
        yield return ("resize-surface-path", () => ResizeCase(WithSurfacePath, 80, 56, CanvasResampling.Bilinear));
        yield return ("resize-locked", () => ResizeCase(() => { var r = MakeRig(); r.D.SetLayerLocks(r.Base, LayerLocks.All); r.D.SetLayerLocks(r.PathLayer, LayerLocks.Pixels); return r.D; }, 60, 42, CanvasResampling.Bilinear));
    }

    static string Hex(byte[] bytes) { var sb = new StringBuilder(); foreach (var b in bytes) sb.Append(b.ToString("x2")); return sb.ToString(); }
    static int Main(string[] args)
    {
        try
        {
            var cases = Cases().ToList();
            if (args.Length == 3 && args[0] == "dump")
            {
                var c = cases.First(x => x.name == args[1]);
                File.WriteAllBytes(args[2], c.run());
                return 0;
            }
            Console.WriteLine("# " + Environment.GetEnvironmentVariable("GOLDEN_SOURCE"));
            using (var sha = SHA256.Create())
                foreach (var c in cases) { if (Trace) Console.Error.WriteLine(c.name); Console.WriteLine(c.name + " " + Hex(sha.ComputeHash(c.run()))); }
            return 0;
        }
        catch (Exception e) { Console.Error.WriteLine(e); return 1; }
    }
}
