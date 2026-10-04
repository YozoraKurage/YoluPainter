using System;
using System.Collections.Generic;
using System.IO;
using System.Linq;
using System.Text;
using Yozolab.YoluPainter.Core;
using Yozolab.YoluPainter.Core.Persistence;

/// M2 の層（グループ・マスク・塗りつぶし・調整・クリッピング・チャンネルごとの有効と合成・Normal の設定）の正本を C# の実際の書き手で作り、
/// 全チャンネルの合成と Normal のファイル出力を添える（Rust の to_core の意味を、往復のバイトだけでなく絵でも確かめるため）。
/// ほかに、Rust が書いた正本を Unity 0.2.0 の読み手に読ませた結果の記録が 2 つ: 版 22（ユーザーチャンネル）は読み手が断ること
/// （DocumentBinary・YlpFormat・ウィンドウの開く手順）、版 21（ユーザーチャンネルが無い文書）は読めて、書き直すと同じバイト列になり、
/// 合成が Rust と一致すること。
static class M2Fixture
{
    static int next = 2000;
    static Guid Id() => new Guid(next++, 0x1234, 0x5678, new byte[] { 0x80, 1, 2, 3, 4, 5, 6, 7 });
    static readonly PaintChannel[] All = (PaintChannel[])Enum.GetValues(typeof(PaintChannel));

    public static bool Run(string[] args)
    {
        if (args[0] == "--unity-reads") { UnityReads(args[1], args[2]); return true; }
        if (args[0] == "--rust-written") { RustWritten(args[1], args[2], args[3]); return true; }
        if (args[0] == "--locks") { Locks(args[1]); return true; }
        if (args[0] != "--m2") return false;
        if (DocumentBinary.CurrentVersion != 21) throw new Exception("正本21の書き手が必要です");
        var root = args[1]; Directory.CreateDirectory(root);
        Save(root, "m2-groups", Groups());
        Save(root, "m2-masks", Masks());
        Save(root, "m2-channels", Channels());
        Save(root, "m2-clipping", Clipping());
        Save(root, "m2-tiny", Tiny());
        Console.WriteLine("M2の正本5件と合成をC#で生成しました");
        return true;
    }

    /// 画布の外の余白を 0 にした、seed ごとの模様のタイルを全部（skip の剰余のタイルは置かない）。マスクは隠す量だけ（RGB 0）。
    static void Paint(PaintDocument doc, SparseTileSurface surface, int seed, bool mask = false, int skip = -1)
    {
        int ts = doc.TileSize, cols = (doc.Width + ts - 1) / ts, rows = (doc.Height + ts - 1) / ts;
        for (int ty = 0; ty < rows; ty++) for (int tx = 0; tx < cols; tx++)
        {
            if ((tx + 2 * ty + seed) % 5 == skip) continue;
            var tile = new byte[ts * ts * 4];
            bool uniform = (tx + ty + seed) % 7 == 0 && (tx + 1) * ts <= doc.Width && (ty + 1) * ts <= doc.Height;
            for (int y = 0; y < ts; y++) for (int x = 0; x < ts; x++)
            {
                int px = tx * ts + x, py = ty * ts + y, i = (y * ts + x) * 4;
                if (px >= doc.Width || py >= doc.Height) continue;
                if (uniform) { if (!mask) { tile[i] = (byte)(seed * 41); tile[i + 1] = (byte)(seed * 23 + 7); tile[i + 2] = (byte)(seed * 59 + 3); } tile[i + 3] = (byte)(seed * 37 + 100); continue; }
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

    static PaintDocument Groups()
    {
        var doc = new PaintDocument(19, 13, 8, 1024 * 1024, Id());
        Raster(doc, "下地", 1);
        var a = Raster(doc, "中 1", 2); doc.SetLayerOpacity(a.Id, .7); doc.SetLayerBlendMode(a.Id, LayerBlendMode.Overlay);
        var b = Raster(doc, "中 2（クリップ）", 3); doc.SetLayerClipping(b.Id, true); doc.SetLayerBlendMode(b.Id, LayerBlendMode.Screen);
        var c = Raster(doc, "奥", 4, PaintChannel.Color, PaintChannel.Height);
        var inv = doc.AddAdjustmentLayer("反転", AdjustmentSettings.Invert(), new[] { PaintChannel.Color, PaintChannel.Height }, Id());
        var inner = doc.GroupLayers(new[] { c.Id, inv.Id }, "入れ子（通過）", Id());
        var fill = doc.AddFillLayer("塗り", new Dictionary<PaintChannel, Rgba32> { { PaintChannel.Color, new Rgba32(200, 40, 90, 128) } }, Id());
        doc.SetLayerClipping(fill.Id, true);
        var g1 = doc.GroupLayers(new[] { a.Id, b.Id, inner.Id, fill.Id }, "分離（乗算）", Id());
        doc.SetLayerBlendMode(g1.Id, LayerBlendMode.Multiply); doc.SetLayerOpacity(g1.Id, .8);
        var hidden = Raster(doc, "隠れた中身", 6);
        var g3 = doc.GroupLayers(new[] { hidden.Id }, "非表示のグループ", Id()); doc.SetLayerVisibility(g3.Id, false);
        var g4 = doc.AddGroup("空のグループ", Id()); doc.SetLayerBlendMode(g4.Id, LayerBlendMode.Normal);
        var top = Raster(doc, "上", 7); doc.SetLayerBlendMode(top.Id, LayerBlendMode.Difference); doc.SetLayerOpacity(top.Id, .55);
        var deep = Raster(doc, "深い層", 8);
        var lv3 = doc.GroupLayers(new[] { deep.Id }, "3 段目", Id()); doc.SetLayerBlendMode(lv3.Id, LayerBlendMode.Luminosity);
        var lv2 = doc.GroupLayers(new[] { lv3.Id }, "2 段目", Id()); doc.SetLayerOpacity(lv2.Id, .9);
        var lv1 = doc.GroupLayers(new[] { lv2.Id }, "1 段目", Id()); doc.SetLayerBlendMode(lv1.Id, LayerBlendMode.LinearBurn);
        // 一番上の段の層を、作った後でグループの中へ動かす（並びの組み替え）
        var moved = Raster(doc, "動かした層", 9);
        doc.MoveLayerTo(moved.Id, inner.Id, 1);
        return doc;
    }

    static PaintDocument Masks()
    {
        var doc = new PaintDocument(37, 21, 16, 1024 * 1024, Id());
        var a = Raster(doc, "マスク付き", 11); Mask(doc, a, 12); doc.SetLayerMaskDensity(a.Id, .65);
        var b = Raster(doc, "反転・無効のマスク", 13); Mask(doc, b, -1); doc.SetLayerMaskInverted(b.Id, true); doc.SetLayerMaskEnabled(b.Id, false);
        var fill = doc.AddFillLayer("塗り・マスク", new Dictionary<PaintChannel, Rgba32> { { PaintChannel.Color, new Rgba32(30, 200, 100, 255) }, { PaintChannel.Emission, new Rgba32(255, 128, 0, 255) } }, Id());
        Mask(doc, fill, 14); doc.SetLayerMaskInverted(fill.Id, true); doc.SetLayerMaskDensity(fill.Id, .3);
        var levels = doc.AddAdjustmentLayer("レベル・マスク", AdjustmentSettings.Levels(.1, .85, 1.6, .05, .95), new[] { PaintChannel.Color }, Id());
        Mask(doc, levels, 15);
        var inside = Raster(doc, "グループの中", 16);
        var g = doc.GroupLayers(new[] { inside.Id }, "マスクのグループ", Id()); doc.SetLayerBlendMode(g.Id, LayerBlendMode.Normal);
        Mask(doc, g, 17); doc.SetLayerMaskDensity(g.Id, .5);
        var inside2 = Raster(doc, "通過の中", 18);
        var pass = doc.GroupLayers(new[] { inside2.Id }, "通過のグループ（マスク）", Id());
        Mask(doc, pass, 19); doc.SetLayerMaskInverted(pass.Id, true);
        var clipped = Raster(doc, "マスク付きのクリップ", 20); doc.SetLayerClipping(clipped.Id, true); Mask(doc, clipped, 21);
        return doc;
    }

    static PaintDocument Channels()
    {
        var doc = new PaintDocument(17, 11, 8, 1024 * 1024, Id());
        doc.SetNormalSettings(new NormalSettings(true, -3.5, HeightEdgeMode.Wrap, NormalYDirection.DirectX));
        var all = Raster(doc, "全チャンネル", 31, All);
        doc.SetChannelEnabled(all.Id, PaintChannel.Metallic, false);
        doc.SetChannelBlend(all.Id, PaintChannel.Color, new ChannelBlend(LayerBlendMode.Multiply, .4));
        doc.SetChannelBlend(all.Id, PaintChannel.Height, new ChannelBlend(LayerBlendMode.Overlay, null));
        doc.SetChannelBlend(all.Id, PaintChannel.Normal, new ChannelBlend(null, .25));
        var empty = Raster(doc, "空の面", 32);
        empty.GetChannel(PaintChannel.Roughness); empty.GetChannel(PaintChannel.Emission);
        doc.SetChannelEnabled(empty.Id, PaintChannel.Emission, false);
        doc.SetChannelBlend(empty.Id, PaintChannel.Roughness, new ChannelBlend(LayerBlendMode.LinearDodge, .6));
        var normal = Raster(doc, "法線 Overlay", 33, PaintChannel.Normal); doc.SetLayerBlendMode(normal.Id, LayerBlendMode.Overlay);
        doc.SetChannelEnabled(normal.Id, PaintChannel.Color, false);
        var fill = doc.AddFillLayer("塗り 3 つ", new Dictionary<PaintChannel, Rgba32> { { PaintChannel.Color, new Rgba32(10, 20, 30, 255) }, { PaintChannel.Roughness, new Rgba32(200, 200, 200, 255) }, { PaintChannel.Normal, new Rgba32(128, 180, 230, 255) } }, Id());
        doc.SetChannelEnabled(fill.Id, PaintChannel.Roughness, false);
        doc.SetChannelBlend(fill.Id, PaintChannel.Normal, new ChannelBlend(LayerBlendMode.Overlay, .5));
        doc.SetFillValue(fill.Id, PaintChannel.Metallic, new Rgba32(90, 90, 90, 255));
        doc.SetFillValue(fill.Id, PaintChannel.Metallic, null); // 値を消しても有効の印は残る（C# の書き手は書かない）
        doc.AddAdjustmentLayer("レベル", AdjustmentSettings.Levels(.05, .95, .8, 0, 1), new[] { PaintChannel.Color, PaintChannel.Height, PaintChannel.Roughness }, Id());
        var hue = doc.AddAdjustmentLayer("色相", AdjustmentSettings.HueSaturation(-40, .3, -.2), new[] { PaintChannel.Color, PaintChannel.Emission }, Id());
        doc.SetLayerOpacity(hue.Id, .75);
        doc.AddAdjustmentLayer("反転（チャンネルなし）", AdjustmentSettings.Invert(), new PaintChannel[0], Id());
        var height = Raster(doc, "高さ", 34, PaintChannel.Color, PaintChannel.Height);
        var top = Raster(doc, "上の色", 35);
        var g = doc.GroupLayers(new[] { height.Id, top.Id }, "グループのチャンネル合成", Id());
        doc.SetChannelBlend(g.Id, PaintChannel.Color, new ChannelBlend(LayerBlendMode.Screen, null));
        doc.SetChannelBlend(g.Id, PaintChannel.Height, new ChannelBlend(LayerBlendMode.Normal, .5));
        doc.SetChannelEnabled(g.Id, PaintChannel.Roughness, false); // グループの有効の印は合成に効かず、C# の書き手も書かない
        return doc;
    }

    static PaintDocument Clipping()
    {
        var doc = new PaintDocument(16, 9, 8, 1024 * 1024, Id());
        var a = Raster(doc, "下地 A", 41); doc.SetLayerClipping(a.Id, true); // 一番下の印（効かない）
        var c1 = Raster(doc, "クリップ 1", 42); doc.SetLayerClipping(c1.Id, true); doc.SetLayerBlendMode(c1.Id, LayerBlendMode.Multiply);
        var c2 = Raster(doc, "クリップ 2", 43); doc.SetLayerClipping(c2.Id, true); doc.SetLayerBlendMode(c2.Id, LayerBlendMode.Screen); doc.SetLayerOpacity(c2.Id, .6);
        var b = Raster(doc, "下地 B（非表示）", 44); doc.SetLayerVisibility(b.Id, false);
        var c3 = Raster(doc, "クリップ 3（下地が非表示）", 45); doc.SetLayerClipping(c3.Id, true);
        var bottom = Raster(doc, "グループの一番下", 46); doc.SetLayerClipping(bottom.Id, true);
        var inner = Raster(doc, "グループの中のクリップ", 47); doc.SetLayerClipping(inner.Id, true); doc.SetLayerBlendMode(inner.Id, LayerBlendMode.HardLight);
        var g = doc.GroupLayers(new[] { bottom.Id, inner.Id }, "クリップされるグループ", Id()); doc.SetLayerClipping(g.Id, true);
        var adj = doc.AddAdjustmentLayer("クリップした反転", AdjustmentSettings.Invert(), new[] { PaintChannel.Color }, Id()); doc.SetLayerClipping(adj.Id, true);
        var fill = doc.AddFillLayer("クリップした塗り", new Dictionary<PaintChannel, Rgba32> { { PaintChannel.Color, new Rgba32(250, 220, 10, 200) } }, Id());
        doc.SetLayerClipping(fill.Id, true); doc.SetLayerBlendMode(fill.Id, LayerBlendMode.SoftLight);
        return doc;
    }

    /// 層のロック（版 12 の属性の印のビット 1 と、直後の int）を、C# の実際の書き手で。個別 4 種・重ね・すべて・グループ・塗りつぶし・調整・
    /// クリッピング・チャンネルごとの合成との同居（属性の印のビット 1 と 2 の並び）・ロックの無い層。ロックは合成を変えない。
    static PaintDocument LockedLayers()
    {
        var doc = new PaintDocument(19, 13, 8, 1024 * 1024, Id());
        var none = Raster(doc, "ロック無し", 51);
        var transparency = Raster(doc, "透明部分", 52); doc.SetLayerLocks(transparency.Id, LayerLocks.Transparency);
        var pixels = Raster(doc, "画素", 53); doc.SetLayerLocks(pixels.Id, LayerLocks.Pixels);
        var position = Raster(doc, "位置", 54); doc.SetLayerLocks(position.Id, LayerLocks.Position);
        var all = Raster(doc, "すべて", 55); doc.SetLayerLocks(all.Id, LayerLocks.All);
        var mixed = Raster(doc, "透明部分と位置（クリップ）", 56, PaintChannel.Color, PaintChannel.Height);
        doc.SetLayerClipping(mixed.Id, true); doc.SetLayerLocks(mixed.Id, LayerLocks.Transparency | LayerLocks.Position);
        doc.SetChannelBlend(mixed.Id, PaintChannel.Color, new ChannelBlend(LayerBlendMode.Multiply, .5)); // 属性の印のビット 1 と 2 が両方立つ
        var every = Raster(doc, "個別 3 つ + すべて", 57); doc.SetLayerLocks(every.Id, LayerLocks.Transparency | LayerLocks.Pixels | LayerLocks.Position | LayerLocks.All);
        var fill = doc.AddFillLayer("ロックした塗り", new Dictionary<PaintChannel, Rgba32> { { PaintChannel.Color, new Rgba32(30, 120, 220, 200) } }, Id());
        doc.SetLayerLocks(fill.Id, LayerLocks.Pixels);
        var adj = doc.AddAdjustmentLayer("ロックした調整", AdjustmentSettings.Levels(.1, .9, 1.2, 0, 1), new[] { PaintChannel.Color }, Id());
        doc.SetLayerLocks(adj.Id, LayerLocks.Position);
        var inside = Raster(doc, "ロックしたグループの中", 58);
        var group = doc.GroupLayers(new[] { inside.Id }, "ロックしたグループ", Id());
        doc.SetLayerLocks(group.Id, LayerLocks.Pixels | LayerLocks.Position);
        var inner = Raster(doc, "ロックの無い層（ロックの層の隣）", 59);
        return doc;
    }

    static void Locks(string root)
    {
        if (DocumentBinary.CurrentVersion != 21) throw new Exception("正本21の書き手が必要です");
        Directory.CreateDirectory(root);
        Save(root, "locks-v21", LockedLayers());
        Console.WriteLine("層のロックの正本をC#で生成しました");
    }

    static PaintDocument Tiny()
    {
        var doc = new PaintDocument(1, 1, 8, 1024 * 1024, Id());
        doc.SetNormalSettings(new NormalSettings(false, -0.0, HeightEdgeMode.Clamp, NormalYDirection.OpenGL));
        var layer = doc.AddLayer("1 画素", Id());
        var tile = new byte[256]; tile[0] = 9; tile[1] = 8; tile[2] = 7; tile[3] = 0; // 透明の画素の RGB
        layer.GetChannel(PaintChannel.Color).ImportTile(new TileCoord(0, 0), tile);
        doc.SetLayerOpacity(layer.Id, 0);
        var values = new Dictionary<PaintChannel, Rgba32>();
        foreach (var c in All) values[c] = c == PaintChannel.Metallic ? Rgba32.Transparent : new Rgba32((byte)(c + 1), (byte)(2 * (int)c), 250, (byte)(100 + (int)c));
        var fill = doc.AddFillLayer("全チャンネルの塗り", values, Id());
        doc.GroupLayers(new[] { layer.Id, fill.Id }, "グループ", Id());
        return doc;
    }

    static void Save(string root, string name, PaintDocument doc)
    {
        var bytes = DocumentBinary.Write(doc); var roundtrip = DocumentBinary.Write(DocumentBinary.Read(bytes));
        if (!bytes.SequenceEqual(roundtrip)) throw new Exception("C# 再保存不一致: " + name);
        File.WriteAllBytes(Path.Combine(root, name + ".utpaint"), bytes);
        File.WriteAllBytes(Path.Combine(root, name + ".composite"), Composites(doc));
    }

    /// 全チャンネルの合成（番号の順、下の行から）に続けて Normal のファイル出力。
    static byte[] Composites(PaintDocument doc)
    {
        using var s = new MemoryStream();
        foreach (var c in All) { var b = doc.Composite(c); s.Write(b, 0, b.Length); }
        var normal = NormalMaps.FileOutput(doc); s.Write(normal, 0, normal.Length);
        return s.ToArray();
    }

    /// Rust が書いた版 21 の正本（ユーザーチャンネルが無い文書）を Unity 0.2.0 の読み手に読ませ、書き直したバイト列が元と同じか、層の数、
    /// 全チャンネルの合成を記録する（Rust の from_core が C# の書き手と同じ並びで書けていることを、C# の読み手側から確かめる）。
    static void RustWritten(string input, string record, string composite)
    {
        var bytes = File.ReadAllBytes(input);
        PaintDocument doc = null;
        var lines = new List<string> { "DocumentBinary.CurrentVersion: " + DocumentBinary.CurrentVersion };
        lines.Add("DocumentBinary.ReadId: " + Try(() => DocumentBinary.ReadId(bytes)));
        lines.Add("DocumentBinary.Read: " + Try(() => doc = DocumentBinary.Read(bytes)));
        if (doc == null) throw new Exception("C# の読み手が Rust の書いた正本を読めません: " + lines.Last());
        lines.Add("DocumentBinary.Write(Read) == input: " + DocumentBinary.Write(doc).SequenceEqual(bytes));
        lines.Add("Layers: " + doc.Layers.Count());
        // ロックがあるときだけ、層ごとの自分のロックと効くロック（親のグループ・すべてを含む）の数も記録する（ロックの無い文書の記録は変わらない）
        if (doc.Layers.Any(l => l.Locks != LayerLocks.None))
            lines.Add("Locks (own/effective): " + string.Join(", ", doc.Layers.Select(l => (int)l.Locks + "/" + (int)doc.EffectiveLocks(l.Id))));
        File.WriteAllText(record, string.Join("\n", lines) + "\n", new UTF8Encoding(false));
        File.WriteAllBytes(composite, Composites(doc));
    }

    /// Unity 0.2.0 の読み手に正本を読ませた結果を、ウィンドウが出す文のまま記録する（決まった文だけ。パスや ID は入れない）。
    static void UnityReads(string input, string output)
    {
        var bytes = File.ReadAllBytes(input);
        var lines = new List<string> { "DocumentBinary.CurrentVersion: " + DocumentBinary.CurrentVersion, "YlpFormat.Current: " + YlpFormat.Current };
        lines.Add("DocumentBinary.Read: " + Try(() => DocumentBinary.Read(bytes)));
        lines.Add("DocumentBinary.ReadId: " + Try(() => DocumentBinary.ReadId(bytes)));
        // 形式 7 の .ylp の 1 つのセットとして（Rust の Project::create と同じ並び）: 外側と project.json は読め、開く手順のセットの読みで断る
        var set = Id(); var name = "ユーザーチャンネルのセット";
        var files = new Dictionary<string, byte[]> {
            { YlpFormat.SetEntry(set, "document.utpaint"), bytes },
            { "project.json", YlpFormat.WriteProject(new YlpProjectInfo(new[] { new YlpTextureSetInfo(set, name, YlpMaterialRef.UnassignedSlots) }, set)) },
        };
        var writer = new YlpWriterInfo("YoluPainter-rs", "0.0.1", "standalone"); YlpFormat.Stamp(files, writer, writer);
        YlpOpened opened = null;
        lines.Add("YlpFormat.Open: " + Try(() => opened = YlpFormat.Open(YlpArchive.Read(YlpArchive.Write(files)))));
        if (opened != null)
        {
            // TexturePaintWindow.ReadTextureSets が失敗に付ける文を、ここで同じ形に組み立てる（ウィンドウを動かした結果ではない）
            string window;
            try { DocumentBinary.Read(opened.SetFiles(set)["document.utpaint"]); window = "OK"; }
            catch (InvalidDataException ex) { window = "InvalidDataException: Texture set \"" + name + "\": " + ex.Message; }
            lines.Add("TexturePaintWindow.ReadTextureSets（文を組み立てたもの。ウィンドウは未実行）: " + window);
        }
        File.WriteAllText(output, string.Join("\n", lines) + "\n", new UTF8Encoding(false));
    }
    static string Try(Func<object> f)
    {
        try { f(); return "OK"; }
        catch (Exception ex) { return ex.GetType().Name + ": " + ex.Message; }
    }
}
