using System;
using System.Collections.Generic;
using System.IO;
using System.Linq;
using System.Text;
using Yozolab.YoluPainter.Core;
using Yozolab.YoluPainter.Core.Paths;
using Yozolab.YoluPainter.Core.MeshMaps;
using Yozolab.YoluPainter.Core.Persistence;
using Yozolab.YoluPainter.Core.Shelf;

/// 効果（フィルターのスタック・Generator・Anchor・塗りつぶしの画像と投影・グラデーション）を持つ正本を C# の実際の書き手で作り、
/// 全チャンネルの合成と、評価した層・マスクの出力（Rust の評価が C# の FilterEngine と同じバイトかを層ごとに見るため）、外から渡した
/// 入力（メッシュマップ・モデルのルート・画像）を添える。入力は人工の値で、ユーザーのモデルや画像は含まない。
static class EffectFixture
{
    static int next = 3000;
    static Guid Id() => new Guid(next++, 0x2234, 0x5678, new byte[] { 0x80, 1, 2, 3, 4, 5, 6, 7 });
    static readonly PaintChannel[] All = (PaintChannel[])Enum.GetValues(typeof(PaintChannel));
    const int W = 41, H = 27, TS = 8;

    public static bool Run(string[] args)
    {
        if (args[0] == "--rust-written-effects") { RustWrittenEffects(args[1], args[2], args[3], args[4]); return true; }
        if (args[0] != "--effects") return false;
        if (DocumentBinary.CurrentVersion != 21) throw new Exception("正本21の書き手が必要です");
        var root = args[1]; Directory.CreateDirectory(root);
        var inputs = new Inputs();
        var resources = new ProjectResources();
        var images = MakeImages(resources);
        Save(root, "effects-filters", Filters(), inputs, resources);
        Save(root, "effects-generators", Generators(inputs), inputs, resources);
        Save(root, "effects-anchors", Anchors(inputs), inputs, resources);
        Save(root, "effects-fills", Fills(resources, images, inputs), inputs, resources);
        Save(root, "effects-paths", Paths(), inputs, resources);
        WriteInputs(Path.Combine(root, "effects-inputs.bin"), inputs, images);
        for (int v = 9; v <= 20; v++) SaveLegacy(root, v, inputs, resources, images);
        Console.WriteLine("効果の正本4件と合成・層の出力・入力をC#で生成しました");
        return true;
    }

    /// Rust が編集 API で作って書いた版 21 の正本（効果入り）を Unity 版の読み手に読ませ、読めたこと・書き直したバイト列が元と同じこと・層の数を
    /// 記録し、同じ人工の入力（`--effects` と同じ ID の画像・マップ）での全チャンネルの合成と、層ごとの評価した出力を添える
    /// （Rust の `from_core` が効果を C# の書き手と同じ並びで書けていることを、C# の読み手の側から確かめる）。
    static void RustWrittenEffects(string input, string record, string composite, string layers)
    {
        var inputs = new Inputs(); var resources = new ProjectResources();
        MakeImages(resources); // `--effects` と同じ並びで作る（画像の ID が同じになる）
        var bytes = File.ReadAllBytes(input);
        PaintDocument doc = null;
        var lines = new List<string> { "DocumentBinary.CurrentVersion: " + DocumentBinary.CurrentVersion };
        lines.Add("DocumentBinary.ReadId: " + Try(() => DocumentBinary.ReadId(bytes)));
        lines.Add("DocumentBinary.Read: " + Try(() => doc = DocumentBinary.Read(bytes)));
        if (doc == null) throw new Exception("C# の読み手が Rust の書いた正本を読めません: " + lines.Last());
        lines.Add("DocumentBinary.Write(Read) == input: " + DocumentBinary.Write(doc).SequenceEqual(bytes));
        lines.Add("Layers: " + doc.Layers.Count());
        doc.GeneratorInputs = inputs; doc.ImageResources = resources;
        File.WriteAllText(record, string.Join("\n", lines) + "\n", new UTF8Encoding(false));
        using var s = new MemoryStream();
        foreach (var c in All) { var b = doc.Composite(c); s.Write(b, 0, b.Length); }
        var normal = NormalMaps.FileOutput(doc); s.Write(normal, 0, normal.Length);
        File.WriteAllBytes(composite, s.ToArray());
        File.WriteAllBytes(layers, LayerOutputs(doc));
    }
    static string Try(Func<object> f)
    {
        try { f(); return "OK"; }
        catch (Exception ex) { return ex.GetType().Name + ": " + ex.Message; }
    }

    // ───────── 人工の入力 ─────────

    static ushort Raw(int x, int y, int c, MeshMapKind k) => (ushort)(((long)x * 1193 + (long)y * 3571 + c * 13451 + (int)k * 7919) % 65536);

    sealed class Inputs : IGeneratorInputs, IGeneratorModelFrame
    {
        public readonly BakedMeshMap[] Maps = new BakedMeshMap[10];
        public long Revision => 1;
        /// 渡した値そのまま（C# のコンストラクターが 1 回正規化する。写しは正規化の前の値で書き、Rust も 1 回だけ正規化する）。
        public static readonly double[] RawFrame = { .13, -.27, .41, .17, -.31, .23, .89 };
        public GeneratorModelFrame ModelFrame { get; } = new GeneratorModelFrame(RawFrame[0], RawFrame[1], RawFrame[2], RawFrame[3], RawFrame[4], RawFrame[5], RawFrame[6]);
        public Inputs()
        {
            foreach (MeshMapKind k in Enum.GetValues(typeof(MeshMapKind)))
            {
                if (k == MeshMapKind.Thickness) continue; // 焼いていないマップ（使う Generator は入力のまま通す）
                int channels = BakedMeshMap.ChannelCount(k);
                var data = new ushort[W * H * channels]; var coverage = new byte[W * H];
                for (int y = 0; y < H; y++) for (int x = 0; x < W; x++)
                {
                    for (int c = 0; c < channels; c++) data[(y * W + x) * channels + c] = Value(k, x, y, c);
                    coverage[y * W + x] = (byte)((x + 3 * y + (int)k) % 17 == 0 ? 0 : 1 + x % 2);
                }
                var p = new MeshMapProvenance(k, 1, "synthetic", "synthetic", 0, W, H, -1, 0, 1, "test", "SnapshotWorld", "StaticSnapshot", "Self", new[] { -1.0, -2.0, -3.0 }, new[] { 2.0, 3.0, 1.0 });
                Maps[(int)k] = new BakedMeshMap(p, data, coverage);
            }
        }
        /// 位置は画素にそってなめらかに、法線はゆるやかに向きが変わる（投影が差を取れるように）。ほかは決まった並び。
        static ushort Value(MeshMapKind k, int x, int y, int c)
        {
            if (k == MeshMapKind.Position)
                return (ushort)(c == 0 ? 65535.0 * (x + .5) / W : c == 1 ? 65535.0 * (y + .5) / H : 65535.0 * (((x * 3 + y * 5) % 41) / 40.0));
            if (k == MeshMapKind.WorldNormal || k == MeshMapKind.BentNormal)
            {
                double nx = Math.Sin(x * .3) * .6, ny = Math.Cos(y * .2) * .5 + 1.1, nz = .5 + Math.Sin((x + y) * .1) * .4, len = Math.Sqrt(nx * nx + ny * ny + nz * nz);
                double v = c == 0 ? nx : c == 1 ? ny : nz;
                return (ushort)Math.Round((v / len * .5 + .5) * 65535);
            }
            return Raw(x, y, c, k);
        }
        public bool TryGetMap(MeshMapKind kind, out BakedMeshMap map, out string reason)
        {
            map = Maps[(int)kind]; reason = map == null ? kind + " has not been baked." : null;
            return map != null;
        }
        public string Key(MeshMapKind kind) => Maps[(int)kind].Provenance.ConditionKey;
    }

    static Dictionary<string, (Guid id, int w, int h, ResourceColorSpace space, byte[] pixels)> MakeImages(ProjectResources resources)
    {
        var images = new Dictionary<string, (Guid, int, int, ResourceColorSpace, byte[])>();
        void Add(string name, int w, int h, ResourceColorSpace space, Func<int, int, byte[]> pixel)
        {
            var bytes = new byte[w * h * 4];
            for (int y = 0; y < h; y++) for (int x = 0; x < w; x++) { var p = pixel(x, y); Buffer.BlockCopy(p, 0, bytes, (y * w + x) * 4, 4); }
            var id = Id(); resources.Add(name, ImageContent.FromPixels(bytes, w, h), ResourceOrigin.None, space, out _, id);
            images[name] = (id, w, h, space, bytes);
        }
        Add("stripes", 8, 6, ResourceColorSpace.Srgb, (x, y) => new[] { (byte)(x * 31 + 20), (byte)(y * 40 + 10), (byte)((x + y) % 2 == 0 ? 220 : 60), (byte)((x + 2 * y) % 5 == 0 ? 90 : 255) });
        Add("linear", 5, 7, ResourceColorSpace.Linear, (x, y) => new[] { (byte)(x * 50), (byte)(y * 35), (byte)(x * y * 9), (byte)255 });
        Add("shape", 6, 6, ResourceColorSpace.Srgb, (x, y) => new byte[] { 255, 255, 255, (byte)((x - 2.5) * (x - 2.5) + (y - 2.5) * (y - 2.5) < 7 ? 255 : 0) });
        Add("grey", 11, 9, ResourceColorSpace.Unspecified, (x, y) => new[] { (byte)(x * 23), (byte)(y * 27 + 5), (byte)(200 - x * 9), (byte)(x == y ? 128 : 255) });
        return images.ToDictionary(e => e.Key, e => e.Value);
    }

    // ───────── 層の道具 ─────────

    static void Paint(PaintDocument doc, SparseTileSurface surface, int seed, bool mask = false, int skip = -1)
    {
        int ts = doc.TileSize, cols = (doc.Width + ts - 1) / ts, rows = (doc.Height + ts - 1) / ts;
        for (int ty = 0; ty < rows; ty++) for (int tx = 0; tx < cols; tx++)
        {
            if ((tx + 2 * ty + seed) % 5 == skip) continue;
            var tile = new byte[ts * ts * 4];
            for (int y = 0; y < ts; y++) for (int x = 0; x < ts; x++)
            {
                int px = tx * ts + x, py = ty * ts + y, i = (y * ts + x) * 4;
                if (px >= doc.Width || py >= doc.Height) continue;
                if (!mask)
                {
                    tile[i] = (byte)(px * 37 + py * 19 + seed * 57 + 11);
                    tile[i + 1] = (byte)(px * 17 + py * 67 + seed * 39 + 41);
                    tile[i + 2] = (byte)(px * 71 + py * 11 + seed * 17 + 73);
                }
                tile[i + 3] = (px + py + seed) % 6 == 0 ? (byte)0 : (byte)(px * 13 + py * 29 + seed * 83);
            }
            surface.ImportTile(new TileCoord(tx, ty), tile);
        }
    }
    static PaintLayer Raster(PaintDocument doc, string name, int seed, params PaintChannel[] channels)
    {
        var layer = doc.AddLayer(name, Id());
        foreach (var c in channels.Length == 0 ? new[] { PaintChannel.Color } : channels) Paint(doc, layer.GetChannel(c), seed + (int)c * 3, skip: seed % 5);
        return layer;
    }
    static RasterMask Mask(PaintDocument doc, PaintLayer layer, int seed)
    {
        var mask = doc.AddLayerMask(layer.Id);
        if (seed >= 0) Paint(doc, mask.Surface, seed, mask: true, skip: (seed + 1) % 5);
        return mask;
    }
    static FilterEffect Add(PaintDocument doc, PaintLayer layer, FilterSettings f, PaintChannel[] channels = null, double strength = 1, bool enabled = true)
        => doc.AddFilter(layer.Id, FilterTarget.Content, f, channels ?? new[] { PaintChannel.Color }, -1, Id(), enabled, strength);
    static FilterEffect AddMask(PaintDocument doc, PaintLayer layer, FilterSettings f, double strength = 1)
        => doc.AddFilter(layer.Id, FilterTarget.Mask, f, null, -1, Id(), true, strength);
    static FilterSettings Gen(GeneratorSettings g) => FilterSettings.FromGenerator(g);
    static GradientRamp Ramp() => new GradientRamp(new[]
    {
        new GradientStop(.1, new Rgba32(231, 19, 47, 0), .17), new GradientStop(.57, new Rgba32(11, 207, 59, 255), .81), new GradientStop(.94, new Rgba32(29, 43, 249, 64))
    }, new[]
    {
        new GradientOpacityStop(0, .9, .24), new GradientOpacityStop(.63, 0, .73), new GradientOpacityStop(1, .7)
    }, new[]
    {
        new GradientCurvePoint(0, .1), new GradientCurvePoint(.23, .9), new GradientCurvePoint(.61, .2), new GradientCurvePoint(1, 1)
    });

    // ───────── フィルター ─────────

    static PaintDocument Filters()
    {
        var doc = new PaintDocument(W, H, TS, 1024 * 1024, Id());
        doc.SetNormalSettings(new NormalSettings(true, -2.5, HeightEdgeMode.Wrap, NormalYDirection.DirectX));
        var a = Raster(doc, "ぼかし・シャープ・ノイズ・レベル・正規化", 101, PaintChannel.Color, PaintChannel.Roughness, PaintChannel.Normal);
        Add(doc, a, FilterSettings.GaussianBlur(3), new[] { PaintChannel.Color, PaintChannel.Roughness, PaintChannel.Normal }, .8);
        Add(doc, a, FilterSettings.Sharpen(2, .75, 17));
        Add(doc, a, FilterSettings.Noise(.3, -123, true), new[] { PaintChannel.Color, PaintChannel.Roughness });
        Add(doc, a, FilterSettings.Levels(.1, .9, 1.7, .2, .8), new[] { PaintChannel.Color, PaintChannel.Roughness });
        Add(doc, a, FilterSettings.Invert(), enabled: false);
        Add(doc, a, FilterSettings.Normalize(), new[] { PaintChannel.Color, PaintChannel.Roughness }, .6);
        Mask(doc, a, 102);
        AddMask(doc, a, FilterSettings.Noise(.2, 57, true));
        AddMask(doc, a, FilterSettings.Levels(.05, .95, 1.2, .1, 1));
        AddMask(doc, a, FilterSettings.GaussianBlur(2));
        AddMask(doc, a, FilterSettings.Invert(), .5);
        doc.SetLayerOpacity(a.Id, .85); doc.SetLayerMaskDensity(a.Id, .8);
        var b = Raster(doc, "大きなぼかし（タイルより広い）", 103, PaintChannel.Color, PaintChannel.Height);
        Add(doc, b, FilterSettings.GaussianBlur(12), new[] { PaintChannel.Color, PaintChannel.Height });
        Add(doc, b, FilterSettings.Normalize(), new[] { PaintChannel.Height });
        doc.SetLayerBlendMode(b.Id, LayerBlendMode.Overlay); doc.SetLayerOpacity(b.Id, .7);
        var c = Raster(doc, "クリップ", 104); doc.SetLayerClipping(c.Id, true);
        Add(doc, c, FilterSettings.Sharpen(3, 1.2, 5));
        Add(doc, c, FilterSettings.Noise(.4, 9, false));
        Mask(doc, c, 105); AddMask(doc, c, FilterSettings.Invert());
        var f = doc.AddFillLayer("塗り・フィルター", new Dictionary<PaintChannel, Rgba32> { { PaintChannel.Color, new Rgba32(200, 40, 90, 220) }, { PaintChannel.Height, new Rgba32(128, 128, 128, 255) } }, Id());
        doc.AddFilter(f.Id, FilterTarget.Content, FilterSettings.Noise(.4, 9, false), new[] { PaintChannel.Color }, -1, Id());
        doc.AddFilter(f.Id, FilterTarget.Content, FilterSettings.Noise(.4, 9, true), new[] { PaintChannel.Height }, -1, Id());
        doc.AddFilter(f.Id, FilterTarget.Content, FilterSettings.GaussianBlur(4), new[] { PaintChannel.Color }, -1, Id());
        Mask(doc, f, -1); AddMask(doc, f, FilterSettings.Noise(.5, 21, true)); doc.SetLayerMaskDensity(f.Id, .6);
        var d = Raster(doc, "グループの中", 106);
        var g = doc.GroupLayers(new[] { d.Id }, "グループのマスク", Id());
        Mask(doc, g, 107); AddMask(doc, g, FilterSettings.GaussianBlur(4)); doc.SetLayerMaskInverted(g.Id, true);
        var n = Raster(doc, "法線のぼかし", 108, PaintChannel.Normal);
        Add(doc, n, FilterSettings.GaussianBlur(5), new[] { PaintChannel.Normal }, .9);
        doc.SetLayerOpacity(n.Id, .8);
        return doc;
    }

    // ───────── Generator ─────────

    static PaintDocument Generators(Inputs inputs)
    {
        var doc = new PaintDocument(W, H, TS, 1024 * 1024, Id());
        var baseLayer = Raster(doc, "Anchor の土台", 201, PaintChannel.Height, PaintChannel.Color);
        var anchor = doc.AddAnchor(baseLayer.Id, AnchorPlacement.Layer, "土台", Id());
        int v = 0;
        foreach (GeneratorType t in Enum.GetValues(typeof(GeneratorType)))
        {
            var layer = Raster(doc, "Generator " + t, 210 + v, PaintChannel.Height, PaintChannel.Color);
            var g = GeneratorSettings.Default(t).WithLevels(.07, .89, .37).WithInvert(v % 2 == 1)
                .WithNoise(v % 3 == 0 ? 0 : .73, .071, v % 2 == 0 ? int.MinValue : 19381, v % 3 == 1 ? GeneratorNoiseSpace.Uv : GeneratorNoiseSpace.Model).WithBlend((GeneratorBlend)((v + 1) % 7));
            if (t == GeneratorType.Dirt) g = g.WithBalance(.3);
            if (t == GeneratorType.PositionGradient) g = g.WithAxis(2);
            if (t == GeneratorType.Direction) g = g.WithDirection(.31, -.71, .19).WithBentNormal(true);
            if (t == GeneratorType.ShapeGradient) g = g.WithVolume(new ShapeVolume(GeneratorShape.Sphere, .2, -.1, .4, 17, -31, 43, 2.3, 3.1, 4.7, .63));
            if (t == GeneratorType.IdColor) g = g.WithIdColors(new[] { IdColor(inputs, 1, 1), IdColor(inputs, 3, 4), 0xabc123 }).WithIdTolerance(43);
            if (t == GeneratorType.EdgeWear) g = g.WithPin(MeshMapKind.Curvature, inputs.Key(MeshMapKind.Curvature)); // 今のマップに合うピン
            var stage = doc.AddFilter(layer.Id, FilterTarget.Content, Gen(g), new[] { PaintChannel.Height }, -1, Id(), true, v % 2 == 0 ? 1 : .6);
            if (t == GeneratorType.Anchor) doc.SetGeneratorAnchor(layer.Id, stage.Id, anchor.Id, PaintChannel.Height, AnchorRead.Value);
            doc.SetLayerOpacity(layer.Id, .5); doc.SetLayerBlendMode(layer.Id, (LayerBlendMode)(v % 4));
            v++;
        }
        // ランプ付き（色に値を色として出す）・マスクの Generator・ほかの段との組み合わせ
        var colour = Raster(doc, "ランプ（色）", 230, PaintChannel.Color);
        doc.AddFilter(colour.Id, FilterTarget.Content, Gen(GeneratorSettings.Default(GeneratorType.ShapeGradient).WithRamp(Ramp()).WithBlend(GeneratorBlend.Screen)), new[] { PaintChannel.Color }, -1, Id());
        doc.AddFilter(colour.Id, FilterTarget.Content, FilterSettings.GaussianBlur(2), new[] { PaintChannel.Color }, -1, Id());
        doc.AddFilter(colour.Id, FilterTarget.Content, Gen(GeneratorSettings.Default(GeneratorType.PositionGradient).WithAxis(0).WithBlend(GeneratorBlend.Multiply)), new[] { PaintChannel.Color }, -1, Id(), true, .5);
        var masked = Raster(doc, "マスクの Generator", 231);
        Mask(doc, masked, 232);
        doc.AddFilter(masked.Id, FilterTarget.Mask, Gen(GeneratorSettings.Default(GeneratorType.EdgeWear).WithBlend(GeneratorBlend.Max)), null, -1, Id());
        doc.AddFilter(masked.Id, FilterTarget.Mask, Gen(GeneratorSettings.Default(GeneratorType.ShapeGradient).WithRamp(Ramp()).WithBlend(GeneratorBlend.Multiply)), null, -1, Id(), true, .7);
        doc.AddFilter(masked.Id, FilterTarget.Mask, FilterSettings.GaussianBlur(3), null, -1, Id());
        var scalar = Raster(doc, "ランプ（スカラー）", 233, PaintChannel.Roughness, PaintChannel.Height);
        doc.AddFilter(scalar.Id, FilterTarget.Content, Gen(GeneratorSettings.Default(GeneratorType.ShapeGradient).WithRamp(Ramp()).WithBlend(GeneratorBlend.Replace)), new[] { PaintChannel.Roughness, PaintChannel.Height }, -1, Id());
        // 使えない Generator（入力のまま通す）: ピンが今のマップと違う・焼いていない Thickness のマップ
        var inactive = Raster(doc, "使えない Generator", 234);
        doc.AddFilter(inactive.Id, FilterTarget.Content, Gen(GeneratorSettings.Default(GeneratorType.EdgeWear).WithPin(MeshMapKind.Curvature, new string('b', 64))), new[] { PaintChannel.Color }, -1, Id());
        doc.AddFilter(inactive.Id, FilterTarget.Content, Gen(GeneratorSettings.Default(GeneratorType.Thickness)), new[] { PaintChannel.Color }, -1, Id());
        doc.AddFilter(inactive.Id, FilterTarget.Content, FilterSettings.Invert(), new[] { PaintChannel.Color }, -1, Id());
        return doc;
    }
    static int IdColor(Inputs inputs, int x, int y)
    {
        var d = new[] { Raw(x, y, 0, MeshMapKind.Id), Raw(x, y, 1, MeshMapKind.Id), Raw(x, y, 2, MeshMapKind.Id) };
        return IdMapColors.Rgb(d, 0);
    }

    // ───────── Anchor ─────────

    static PaintDocument Anchors(Inputs inputs)
    {
        var doc = new PaintDocument(W, H, TS, 1024 * 1024, Id());
        var baseLayer = Raster(doc, "土台", 301, PaintChannel.Height, PaintChannel.Color);
        Add(doc, baseLayer, FilterSettings.GaussianBlur(2), new[] { PaintChannel.Height, PaintChannel.Color });
        Mask(doc, baseLayer, 302);
        var a1 = doc.AddAnchor(baseLayer.Id, AnchorPlacement.Layer, "土台の層", Id());
        var m1 = doc.AddAnchor(baseLayer.Id, AnchorPlacement.Mask, "土台のマスク", Id());
        var mid = Raster(doc, "中", 303, PaintChannel.Height);
        doc.SetLayerOpacity(mid.Id, .6); doc.SetLayerBlendMode(mid.Id, LayerBlendMode.Multiply);
        var a2 = doc.AddAnchor(mid.Id, AnchorPlacement.Layer, "中の層", Id());
        // 読む層: 土台の Height を値として、Color を被覆として、マスクの Anchor を反転して
        var r1 = doc.AddFillLayer("読む 1", new Dictionary<PaintChannel, Rgba32> { { PaintChannel.Height, new Rgba32(128, 128, 128, 255) }, { PaintChannel.Color, new Rgba32(60, 160, 220, 255) } }, Id());
        var s1 = doc.AddFilter(r1.Id, FilterTarget.Content, Gen(GeneratorSettings.Default(GeneratorType.Anchor).WithLevels(.1, .9, .3).WithBlend(GeneratorBlend.Multiply)), new[] { PaintChannel.Height }, -1, Id());
        doc.SetGeneratorAnchor(r1.Id, s1.Id, a1.Id, PaintChannel.Height, AnchorRead.Value);
        var s2 = doc.AddFilter(r1.Id, FilterTarget.Content, Gen(GeneratorSettings.Default(GeneratorType.Anchor).WithBlend(GeneratorBlend.Replace)), new[] { PaintChannel.Color }, -1, Id(), true, .8);
        doc.SetGeneratorAnchor(r1.Id, s2.Id, a1.Id, PaintChannel.Color, AnchorRead.Coverage);
        doc.AddFilter(r1.Id, FilterTarget.Content, FilterSettings.GaussianBlur(3), new[] { PaintChannel.Color });
        Mask(doc, r1, -1);
        var s3 = doc.AddFilter(r1.Id, FilterTarget.Mask, Gen(GeneratorSettings.Default(GeneratorType.Anchor).WithInvert(true).WithBlend(GeneratorBlend.Replace)), null, -1, Id());
        doc.SetGeneratorAnchor(r1.Id, s3.Id, m1.Id, PaintChannel.Height, AnchorRead.Value);
        // 連鎖: 中の Anchor を読む層（段の後にぼかし）。その層にも Anchor
        var r2 = Raster(doc, "読む 2（連鎖）", 304, PaintChannel.Height, PaintChannel.Color);
        var s4 = doc.AddFilter(r2.Id, FilterTarget.Content, Gen(GeneratorSettings.Default(GeneratorType.Anchor).WithNoise(.5, .1, 5, GeneratorNoiseSpace.Uv).WithBlend(GeneratorBlend.Subtract)), new[] { PaintChannel.Height }, -1, Id());
        doc.SetGeneratorAnchor(r2.Id, s4.Id, a2.Id, PaintChannel.Height, AnchorRead.Value);
        doc.AddFilter(r2.Id, FilterTarget.Content, FilterSettings.GaussianBlur(3), new[] { PaintChannel.Height });
        var a3 = doc.AddAnchor(r2.Id, AnchorPlacement.Layer, "連鎖の層", Id());
        var r3 = doc.AddFillLayer("読む 3（連鎖の先）", new Dictionary<PaintChannel, Rgba32> { { PaintChannel.Height, new Rgba32(200, 200, 200, 255) } }, Id());
        var s5 = doc.AddFilter(r3.Id, FilterTarget.Content, Gen(GeneratorSettings.Default(GeneratorType.Anchor).WithBlend(GeneratorBlend.Min)), new[] { PaintChannel.Height }, -1, Id());
        doc.SetGeneratorAnchor(r3.Id, s5.Id, a3.Id, PaintChannel.Height, AnchorRead.Value);
        doc.SetLayerClipping(r3.Id, true);
        // 分離のグループの中の Anchor（グループの中は透明から）を、グループより上の層が読む
        var inside = Raster(doc, "グループの中", 305, PaintChannel.Height);
        var group = doc.GroupLayers(new[] { inside.Id }, "分離", Id()); doc.SetLayerBlendMode(group.Id, LayerBlendMode.Multiply);
        var a4 = doc.AddAnchor(inside.Id, AnchorPlacement.Layer, "グループの中", Id());
        var r4 = Raster(doc, "読む 4", 306, PaintChannel.Height);
        var s6 = doc.AddFilter(r4.Id, FilterTarget.Content, Gen(GeneratorSettings.Default(GeneratorType.Anchor).WithBlend(GeneratorBlend.Add)), new[] { PaintChannel.Height }, -1, Id(), true, .7);
        doc.SetGeneratorAnchor(r4.Id, s6.Id, a4.Id, PaintChannel.Height, AnchorRead.Value);
        // 参照が使えない読み手（保存したまま読み、入力のまま通す）: 消えた Anchor・自分より上の Anchor・まだ選んでいない
        doc.AddFilter(r4.Id, FilterTarget.Content, Gen(GeneratorSettings.Default(GeneratorType.Anchor).WithAnchor(Id(), PaintChannel.Height, AnchorRead.Value)), new[] { PaintChannel.Height }, -1, Id());
        doc.AddFilter(baseLayer.Id, FilterTarget.Content, Gen(GeneratorSettings.Default(GeneratorType.Anchor).WithAnchor(a2.Id, PaintChannel.Height, AnchorRead.Coverage)), new[] { PaintChannel.Height }, -1, Id());
        doc.AddFilter(r4.Id, FilterTarget.Content, Gen(GeneratorSettings.Default(GeneratorType.Anchor)), new[] { PaintChannel.Color }, -1, Id());
        return doc;
    }

    // ───────── 塗りつぶしの画像・投影・グラデーション ─────────

    static PaintDocument Fills(ProjectResources resources, Dictionary<string, (Guid id, int w, int h, ResourceColorSpace space, byte[] pixels)> images, Inputs inputs)
    {
        var doc = new PaintDocument(W, H, TS, 1024 * 1024, Id());
        doc.GeneratorInputs = inputs; doc.ImageResources = resources;
        var bottom = Raster(doc, "下地", 401, PaintChannel.Color, PaintChannel.Height);
        FillProjection[] projections =
        {
            FillProjection.Default.WithTiles(1.5, 2).WithOffset(.1, -.2).WithRotation(23),
            FillProjection.Default.WithMode(FillProjectionMode.Triplanar).WithTiles(2, 2).WithBlendWidth(.4).WithPlacement(new ShapeVolume(GeneratorShape.Box, .1, .2, -.3, 10, 20, 30, 1.5, 2, 2.5, 0)),
            FillProjection.Default.WithMode(FillProjectionMode.Planar).WithWrap(FillWrap.Clamp).WithPlacement(new ShapeVolume(GeneratorShape.Box, 0, 0, 0, 15, -25, 5, 3, 2, 1, 0)),
            FillProjection.Default.WithMode(FillProjectionMode.Spherical).WithTiles(2, 1).WithPlacement(new ShapeVolume(GeneratorShape.Box, .3, 0, .1, 0, 40, 0, 2, 2, 2, 0)),
            FillProjection.Default.WithMode(FillProjectionMode.Cylindrical).WithWrap(FillWrap.None).WithTiles(1, 3).WithPlacement(new ShapeVolume(GeneratorShape.Box, 0, .5, 0, 0, 0, 20, 1, 3, 1, 0)),
        };
        int k = 0;
        string[] names = { "stripes", "linear", "grey", "stripes", "linear" };
        foreach (var p in projections)
        {
            var fill = doc.AddFillLayer("画像 " + p.Mode, new Dictionary<PaintChannel, Rgba32> { { PaintChannel.Color, new Rgba32(10, 200, 30, 255) }, { PaintChannel.Roughness, new Rgba32(90, 90, 90, 255) }, { PaintChannel.Normal, new Rgba32(128, 128, 255, 255) } }, Id());
            doc.SetFillImage(fill.Id, PaintChannel.Color, images[names[k]].id);
            doc.SetFillImage(fill.Id, PaintChannel.Roughness, images["grey"].id); // スカラーは輝度
            doc.SetFillProjection(fill.Id, p);
            doc.SetLayerOpacity(fill.Id, .5); doc.SetLayerBlendMode(fill.Id, (LayerBlendMode)(k % 3));
            k++;
        }
        // デカール: 画像と形の画像（別のチャンネル）、値だけ、グラデーションと重ねる
        var decalProjection = FillProjection.DecalAt(new ShapeVolume(GeneratorShape.Box, .4, .3, 0, 0, 25, 0, 2.5, 3, 2, 0)).WithCulling(.6, 100, .5);
        var decal = doc.AddFillLayer("デカール", new Dictionary<PaintChannel, Rgba32> { { PaintChannel.Color, new Rgba32(220, 40, 40, 255) }, { PaintChannel.Roughness, new Rgba32(200, 200, 200, 255) }, { PaintChannel.Height, new Rgba32(255, 255, 255, 255) } }, Id());
        doc.SetFillImage(decal.Id, PaintChannel.Color, images["stripes"].id);
        doc.SetFillImage(decal.Id, PaintChannel.Height, images["shape"].id);
        doc.SetFillProjection(decal.Id, decalProjection);
        doc.SetFillGradient(decal.Id, PaintChannel.Roughness, GeneratorSettings.Default(GeneratorType.ShapeGradient).WithRamp(Ramp()).WithBlend(GeneratorBlend.Replace).WithVolume(new ShapeVolume(GeneratorShape.Plane, 0, 0, 0, 0, 0, 0, 1, 2, 1, 0)));
        // グラデーション（形・ランプ）: 色とスカラー、画像に替えて戻す
        var gradient = doc.AddFillLayer("グラデーション", new Dictionary<PaintChannel, Rgba32> { { PaintChannel.Color, new Rgba32(30, 30, 30, 255) }, { PaintChannel.Metallic, new Rgba32(10, 10, 10, 255) }, { PaintChannel.Height, new Rgba32(40, 40, 40, 255) } }, Id());
        doc.SetFillGradient(gradient.Id, PaintChannel.Color, GeneratorSettings.Default(GeneratorType.ShapeGradient).WithRamp(Ramp()).WithBlend(GeneratorBlend.Replace).WithVolume(new ShapeVolume(GeneratorShape.Sphere, .1, 0, 0, 0, 0, 0, 3, 3, 3, .7)));
        doc.SetFillGradient(gradient.Id, PaintChannel.Metallic, GeneratorSettings.Default(GeneratorType.ShapeGradient).WithRamp(GradientRamp.Default).WithBlend(GeneratorBlend.Replace).WithNoise(.5, .1, 3, GeneratorNoiseSpace.Uv).WithVolume(new ShapeVolume(GeneratorShape.Box, 0, 0, 0, 30, 0, 0, 2, 2, 2, .4)));
        doc.SetFillGradient(gradient.Id, PaintChannel.Height, GeneratorSettings.Default(GeneratorType.ShapeGradient).WithRamp(Ramp()).WithBlend(GeneratorBlend.Replace));
        doc.SetLayerOpacity(gradient.Id, .6);
        // 画像とフィルター・マスク・存在しない画像
        var filtered = doc.AddFillLayer("画像・フィルター", new Dictionary<PaintChannel, Rgba32> { { PaintChannel.Color, new Rgba32(250, 250, 10, 255) } }, Id());
        doc.SetFillImage(filtered.Id, PaintChannel.Color, images["stripes"].id);
        doc.SetFillProjection(filtered.Id, FillProjection.Default.WithTiles(3, 3));
        doc.AddFilter(filtered.Id, FilterTarget.Content, FilterSettings.GaussianBlur(3), new[] { PaintChannel.Color }, -1, Id());
        doc.AddFilter(filtered.Id, FilterTarget.Content, FilterSettings.Levels(.1, .9, 1.4, 0, 1), new[] { PaintChannel.Color }, -1, Id());
        Mask(doc, filtered, 402); doc.SetLayerMaskDensity(filtered.Id, .7);
        var missing = doc.AddFillLayer("画像が無い", new Dictionary<PaintChannel, Rgba32> { { PaintChannel.Color, new Rgba32(130, 20, 200, 180) } }, Id());
        doc.SetFillImagesForLoad(missing, new[] { new KeyValuePair<PaintChannel, Guid>(PaintChannel.Color, Id()) }, FillProjection.Default.WithMode(FillProjectionMode.Planar));
        return doc;
    }

    // ───────── パス ─────────

    static PathBrush Brush(double radius, bool pressureSize, bool erase = false) => new PathBrush
    {
        RadiusWorld = radius, Hardness = .7, Spacing = .17, Opacity = .83, Flow = .42, Color = new Rgba32(201, 37, 89, 219),
        PressureSize = pressureSize, PressureOpacity = !pressureSize, PressureFlow = false, Erase = erase,
    };
    static SparseTileSurface Rendered(PaintDocument doc, int seed)
    {
        var surface = new SparseTileSurface(doc.Width, doc.Height, doc.TileSize);
        Paint(doc, surface, seed, skip: seed % 5);
        return surface;
    }

    /// 2D のパス（C# が描いた画素）・3D のパス（描いた画素を渡す）。層のパスと画素がそろって保存され、合成は画素のまま。
    static PaintDocument Paths()
    {
        var doc = new PaintDocument(W, H, TS, 1024 * 1024, Id());
        var under = Raster(doc, "下", 501, PaintChannel.Color, PaintChannel.Roughness);
        var points = new[] { new CanvasPoint(4.5, 7.25, .3), new CanvasPoint(29.5, 21.5, .9), new CanvasPoint(37.25, 5.75, .55) };
        var a = doc.AddLayer("2D のパス", Id());
        doc.SetCanvasPath(a.Id, new CanvasPath(Id(), PaintChannel.Color, Brush(3.25, true), points));
        var b = doc.AddLayer("2D のパス（組）", Id());
        doc.SetCanvasPath(b.Id, new CanvasPath(Id(), PaintChannel.Color, Brush(2.5, false), points.Take(2),
            new[] { new ChannelPaint(PaintChannel.Color, new Rgba32(31, 87, 231, 255)), new ChannelPaint(PaintChannel.Roughness, new Rgba32(140, 140, 140, 255)) }));
        doc.SetLayerOpacity(b.Id, .8);
        var c = doc.AddLayer("3D のパス", Id());
        var surfacePoints = new[] { new PathPoint(0, .2, .3, .7), new PathPoint(3, .5, .25, 1), new PathPoint(1, 0, 1, .2) };
        doc.SetPath(c.Id, new SurfacePath(Id(), PaintChannel.Color, "synthetic-model", Brush(.08, true), surfacePoints), Rendered(doc, 502));
        var d = doc.AddLayer("3D のパス（組）", Id());
        doc.SetPath(d.Id, new SurfacePath(Id(), PaintChannel.Height, "synthetic-model-2", Brush(.5, false, true), surfacePoints.Take(2),
                new[] { new ChannelPaint(PaintChannel.Height, new Rgba32(200, 200, 200, 255)), new ChannelPaint(PaintChannel.Color, new Rgba32(10, 220, 40, 255)) }),
            new Dictionary<PaintChannel, SparseTileSurface> { { PaintChannel.Height, Rendered(doc, 503) }, { PaintChannel.Color, Rendered(doc, 504) } });
        var group = doc.GroupLayers(new[] { d.Id }, "グループ", Id());
        var mask = Mask(doc, a, 505);
        AddMask(doc, a, FilterSettings.GaussianBlur(2));
        return doc;
    }

    // ───────── 旧い版の正本 ─────────

    /// 版 v の正本を手で組む（C# の書き手は今の版しか書かない）。効果は版ごとに足されたものだけ: 9 フィルター（ぼかし・マスクの反転）、
    /// 11 Generator の段、13 形のグラデーション、15 ID の色、16 塗りつぶしの画像と投影、17 デカール、20 Anchor と Anchor の Generator。
    /// C# の読み手で読み直し、書き直した版 21 のバイト列と合成を添える（Rust の読みと移行が C# と同じことを見る）。
    static byte[] Legacy(int v, Dictionary<string, (Guid id, int w, int h, ResourceColorSpace space, byte[] pixels)> images, Inputs inputs)
    {
        using var s = new MemoryStream(); using var w = new BinaryWriter(s);
        void Text(string text) { var b = Encoding.UTF8.GetBytes(text); w.Write(b.Length); w.Write(b); }
        void Guid16(Guid id) => w.Write(id.ToByteArray());
        w.Write(Encoding.ASCII.GetBytes("DOTPAINT")); w.Write(v); Guid16(Id()); w.Write(W); w.Write(H); w.Write(TS);
        if (v >= 7) { w.Write(1); w.Write(false); w.Write(-0.0); w.Write(0); w.Write(1); }
        bool anchors = v >= 20, fillImage = v >= 16, decal = v >= 17;
        int layers = 1 + (v >= 3 ? 1 : 0) + (decal ? 1 : 0) + (anchors ? 1 : 0);
        w.Write(layers);
        void Tiles(bool mask)
        {
            int cols = (W + TS - 1) / TS, rows = (H + TS - 1) / TS; var list = new List<(int, int, byte[])>();
            for (int ty = 0; ty < rows; ty++) for (int tx = 0; tx < cols; tx++)
            {
                if ((tx + 2 * ty) % 4 == 3) continue;
                var tile = new byte[TS * TS * 4];
                for (int y = 0; y < TS; y++) for (int x = 0; x < TS; x++)
                {
                    int px = tx * TS + x, py = ty * TS + y, i = (y * TS + x) * 4; if (px >= W || py >= H) continue;
                    if (!mask) { tile[i] = (byte)(px * 37 + py * 19 + v * 57); tile[i + 1] = (byte)(px * 17 + py * 67 + 41); tile[i + 2] = (byte)(px * 71 + py * 11 + 73); }
                    tile[i + 3] = (px + py) % 6 == 0 ? (byte)0 : (byte)(px * 13 + py * 29 + v * 83);
                }
                list.Add((tx, ty, tile));
            }
            w.Write(list.Count); foreach (var (x, y, b) in list) { w.Write(x); w.Write(y); w.Write(b.Length); w.Write(b); }
        }
        void Generator(int type, int algorithm, Action extra)
        {
            w.Write(type); w.Write(algorithm); w.Write(.07); w.Write(.89); w.Write(.37); w.Write(false); w.Write(.4); w.Write(.1); w.Write(5); w.Write(1);
            w.Write(type == 0 ? 0 : 1); w.Write(.5); w.Write(type == 2 ? 2 : 1); w.Write(0.0); w.Write(1.0); w.Write(0.0); w.Write(false);
            w.Write(0); // ピンは無し
            extra();
        }
        void Filter(Guid id, int type, bool content, int[] channels, Action body, Action generator = null)
        {
            Guid16(id); w.Write(type); w.Write(1); w.Write(true); w.Write(type == 6 ? .75 : 1.0);
            if (content) { w.Write(channels.Length); foreach (var c in channels) w.Write(c); }
            body(); // radius, amount, threshold, seed, mono, 5 double
            generator?.Invoke();
        }
        void Defaults() { w.Write(0); w.Write(0.0); w.Write(0); w.Write(0); w.Write(false); w.Write(0.0); w.Write(1.0); w.Write(1.0); w.Write(0.0); w.Write(1.0); }
        Guid anchorId = Id(), layer0 = Id();
        // 層 0: ラスター（Color）。マスクとフィルター・Anchor
        Guid16(layer0); Text("旧い版の効果"); w.Write(true); w.Write(.8); w.Write(0);
        int attribute = (anchors ? 16 : 0);
        if (v >= 12) w.Write((byte)attribute); else if (v >= 5) w.Write((byte)0);
        if (v >= 3) w.Write(0); if (v >= 6) Guid16(Guid.Empty); if (v >= 3) w.Write(0);
        w.Write(1); w.Write(0); w.Write(true); Tiles(false);
        if (v >= 2) { w.Write(true); w.Write(true); w.Write(false); w.Write(.8); Tiles(true); }
        if (v >= 8) w.Write(false);
        if (v >= 9)
        {
            w.Write(true);
            var stages = new List<Action>();
            stages.Add(() => Filter(Id(), 0, true, new[] { 0 }, () => { w.Write(2); w.Write(0.0); w.Write(0); w.Write(0); w.Write(false); w.Write(0.0); w.Write(1.0); w.Write(1.0); w.Write(0.0); w.Write(1.0); }));
            if (v >= 11) stages.Add(() => Filter(Id(), 6, true, new[] { 0 }, Defaults, () => Generator(2, 1, () => { })));
            if (v >= 13) stages.Add(() => Filter(Id(), 6, true, new[] { 0 }, Defaults, () => Generator(5, 1, () => { w.Write(1); foreach (double d in new[] { .2, -.1, .4, 17, -31, 43, 2.3, 3.1, 4.7, .63 }) w.Write(d); })));
            if (v >= 15) stages.Add(() => Filter(Id(), 6, true, new[] { 0 }, Defaults, () => Generator(6, 1, () => { w.Write(43); w.Write(2); w.Write(IdColor(inputs, 1, 1)); w.Write(0xabc123); })));
            w.Write(stages.Count); foreach (var st in stages) st();
            w.Write(1); Filter(Id(), 4, false, null, Defaults); // マスクの反転
        }
        if (v >= 10) w.Write(false);
        if (anchors) { w.Write((byte)1); Guid16(anchorId); Text("旧い版の Anchor"); }
        // 層 1: 塗りつぶし（版 3 から）。版 16 から画像と投影
        if (v >= 3)
        {
            Guid16(Id()); Text("塗り"); w.Write(true); w.Write(1.0); w.Write(0);
            int a1 = fillImage ? 8 : 0;
            if (v >= 12) w.Write((byte)a1); else if (v >= 5) w.Write((byte)0);
            w.Write(1); if (v >= 6) Guid16(Guid.Empty);
            w.Write(1); w.Write(0); w.Write(true); w.Write(new byte[] { 200, 40, 90, 160 });
            if (fillImage)
            {
                w.Write(1); w.Write(0); Guid16(images["stripes"].id);
                w.Write(1); w.Write(0); w.Write(0); foreach (double d in new[] { 1.5, 2, .1, -.2, 23, .3, 0, 0, 0, 0, 0, 0, 1, 1, 1 }) w.Write(d);
            }
            w.Write(0); // チャンネルの面は無い
            if (v >= 2) w.Write(false); if (v >= 8) w.Write(false); if (v >= 9) w.Write(false); if (v >= 10) w.Write(false);
        }
        // 層 2: デカール（版 17）
        if (decal)
        {
            Guid16(Id()); Text("デカール"); w.Write(true); w.Write(1.0); w.Write(0);
            w.Write((byte)8); w.Write(1); Guid16(Guid.Empty);
            w.Write(1); w.Write(0); w.Write(true); w.Write(new byte[] { 220, 40, 40, 255 });
            w.Write(1); w.Write(0); Guid16(images["shape"].id);
            w.Write(1); w.Write(5); w.Write(2); foreach (double d in new[] { 1, 1, 0, 0, 0, .3, .4, .3, 0, 0, 25, 0, 2.5, 3, 2, .6, 100, .5 }) w.Write(d);
            w.Write(0); w.Write(false); w.Write(false); w.Write(false); w.Write(false);
        }
        // 層 3: Anchor を読む塗りつぶし（版 20）
        if (anchors)
        {
            Guid16(Id()); Text("Anchor を読む"); w.Write(true); w.Write(1.0); w.Write(0);
            w.Write((byte)0); w.Write(1); Guid16(Guid.Empty);
            w.Write(1); w.Write(0); w.Write(true); w.Write(new byte[] { 90, 200, 130, 255 });
            w.Write(0); w.Write(false); w.Write(false);
            w.Write(true); w.Write(1);
            Filter(Id(), 6, true, new[] { 0 }, Defaults, () => Generator(7, 1, () => { Guid16(anchorId); w.Write(0); w.Write(1); }));
            w.Write(false);
        }
        w.Flush(); return s.ToArray();
    }

    static void SaveLegacy(string root, int v, Inputs inputs, ProjectResources resources, Dictionary<string, (Guid id, int w, int h, ResourceColorSpace space, byte[] pixels)> images)
    {
        var bytes = Legacy(v, images, inputs);
        var doc = DocumentBinary.Read(bytes); doc.GeneratorInputs = inputs; doc.ImageResources = resources;
        File.WriteAllBytes(Path.Combine(root, "effects-legacy-v" + v + ".utpaint"), bytes);
        File.WriteAllBytes(Path.Combine(root, "effects-legacy-v" + v + ".v21"), DocumentBinary.Write(doc));
        using var s = new MemoryStream();
        foreach (var c in All) { var b = doc.Composite(c); s.Write(b, 0, b.Length); }
        var normal = NormalMaps.FileOutput(doc); s.Write(normal, 0, normal.Length);
        File.WriteAllBytes(Path.Combine(root, "effects-legacy-v" + v + ".composite"), s.ToArray());
    }

    // ───────── 保存・記録 ─────────

    static void Save(string root, string name, PaintDocument doc, Inputs inputs, ProjectResources resources)
    {
        doc.GeneratorInputs = inputs; doc.ImageResources = resources;
        var bytes = DocumentBinary.Write(doc); var roundtrip = DocumentBinary.Write(DocumentBinary.Read(bytes));
        if (!bytes.SequenceEqual(roundtrip)) throw new Exception("C# 再保存不一致: " + name);
        File.WriteAllBytes(Path.Combine(root, name + ".utpaint"), bytes);
        using var s = new MemoryStream();
        foreach (var c in All) { var b = doc.Composite(c); s.Write(b, 0, b.Length); }
        var normal = NormalMaps.FileOutput(doc); s.Write(normal, 0, normal.Length);
        File.WriteAllBytes(Path.Combine(root, name + ".composite"), s.ToArray());
        File.WriteAllBytes(Path.Combine(root, name + ".layers"), LayerOutputs(doc));
    }

    /// 層ごとに、チャンネルごとの評価した出力（評価が要る層・チャンネルだけ。旗 1 バイトに続けて画素）と、マスクの評価した隠す量。
    /// 合成が読むのと同じタイルの経路（`GetOutputPixel`・`OutputHideAt`。何も出ない所は透明）で読む。
    static byte[] LayerOutputs(PaintDocument doc)
    {
        using var s = new MemoryStream();
        foreach (var layer in doc.Layers)
        {
            foreach (var c in All)
            {
                bool has = (layer.Kind == LayerKind.Raster || layer.Kind == LayerKind.Fill) && layer.HasEvaluatedOutput(c);
                s.WriteByte((byte)(has ? 1 : 0));
                if (has)
                    for (int y = 0; y < doc.Height; y++) for (int x = 0; x < doc.Width; x++) { var p = layer.GetOutputPixel(c, x, y); s.WriteByte(p.R); s.WriteByte(p.G); s.WriteByte(p.B); s.WriteByte(p.A); }
            }
            bool mask = layer.Mask != null && layer.Mask.HasActiveFilters;
            s.WriteByte((byte)(mask ? 1 : 0));
            if (mask)
                for (int y = 0; y < doc.Height; y++) for (int x = 0; x < doc.Width; x++) s.WriteByte(layer.Mask.OutputHideAt(x, y));
        }
        return s.ToArray();
    }

    /// 入力の写し: 数（int）、マップごとに種類・幅・高さ（int）・鍵（64 バイトの ASCII）・箱（double × 6）・状態（byte。0 は今のもの）・成分（ushort）・被覆（byte）、
    /// モデルのルート（double × 7）、画像の数（int）、画像ごとに ID（16 バイト）・幅・高さ（int）・色空間（byte）・画素。
    static void WriteInputs(string path, Inputs inputs, Dictionary<string, (Guid id, int w, int h, ResourceColorSpace space, byte[] pixels)> images)
    {
        using var s = new MemoryStream(); using var w = new BinaryWriter(s);
        var maps = inputs.Maps.Where(m => m != null).ToArray();
        w.Write(maps.Length);
        foreach (var m in maps)
        {
            w.Write((int)m.Kind); w.Write(m.Width); w.Write(m.Height); w.Write(Encoding.ASCII.GetBytes(m.Provenance.ConditionKey));
            for (int a = 0; a < 3; a++) w.Write(m.Provenance.BoundsMin(a));
            for (int a = 0; a < 3; a++) w.Write(m.Provenance.BoundsMax(a));
            w.Write((byte)0);
            for (int y = 0; y < m.Height; y++) for (int x = 0; x < m.Width; x++) for (int c = 0; c < m.Channels; c++) w.Write(m.RawValue(x, y, c));
            for (int y = 0; y < m.Height; y++) for (int x = 0; x < m.Width; x++) w.Write((byte)m.CoverageAt(x, y));
        }
        foreach (double d in Inputs.RawFrame) w.Write(d);
        w.Write(images.Count);
        foreach (var i in images.Values.OrderBy(i => i.id))
        { w.Write(i.id.ToByteArray()); w.Write(i.w); w.Write(i.h); w.Write((byte)i.space); w.Write(i.pixels); }
        w.Flush(); File.WriteAllBytes(path, s.ToArray());
    }
}
