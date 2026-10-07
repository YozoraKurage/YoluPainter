// YoluPainter-rs の書き出し（テンプレートとパディング）の正解のファイルを、Unity 版の C# の Core（ExportTemplates・TexturePadding）に
// 同じ入力を通して作る（Core と一緒に組んで Mono で走らせる）。
//   golden <cases.txt> <出力のフォルダ>   台本の事例を走らせ、index.txt と <事例>.bin を書く
//   bench [回数]                          4096² のテンプレートの Build とパディング（覆い・塗り広げ）の時間を測る
// 台本の読み方・乱数・出力の書き方は crates/yolu-core/tests/reference/export_golden.rs と揃えてある（片方を変えたら両方を変える）。
using System;
using System.Collections.Generic;
using System.Diagnostics;
using System.Globalization;
using System.IO;
using System.Linq;
using System.Text;
using Yozolab.YoluPainter.Core;

namespace YoluPainterRs.ExportGolden
{
    /// <summary>splitmix64。色の成分は 1/8 で 0、1/8 で 255、ほかは一様。アルファは 1/4 で 0、1/4 で 255、ほかは一様。</summary>
    sealed class SplitMix
    {
        ulong state;
        public SplitMix(ulong seed) { state = seed; }
        public ulong Next()
        {
            unchecked
            {
                state += 0x9E3779B97F4A7C15UL;
                ulong z = state;
                z = (z ^ (z >> 30)) * 0xBF58476D1CE4E5B9UL;
                z = (z ^ (z >> 27)) * 0x94D049BB133111EBUL;
                return z ^ (z >> 31);
            }
        }
        public double U01() { return (Next() >> 11) * (1.0 / 9007199254740992.0); }
        public byte Channel() { ulong r = Next(); switch (r & 7) { case 0: return 0; case 1: return 255; default: return (byte)(r >> 8); } }
        public byte Alpha() { ulong r = Next(); switch (r & 7) { case 0: case 1: return 0; case 2: case 3: return 255; default: return (byte)(r >> 8); } }
        public Rgba32 Rgba() { byte r = Channel(); byte g = Channel(); byte b = Channel(); byte a = Alpha(); return new Rgba32(r, g, b, a); }
    }

    static class Program
    {
        static int Main(string[] args)
        {
            try
            {
                if (args.Length == 3 && args[0] == "golden") { RunCases(args[1], args[2]); return 0; }
                if (args.Length >= 1 && args[0] == "bench") { Bench(args.Length > 1 ? int.Parse(args[1]) : 5); return 0; }
                Console.Error.WriteLine("使い方: golden <cases.txt> <出力> | bench [回数]");
                return 2;
            }
            catch (Exception e) { Console.Error.WriteLine(e); return 1; }
        }

        // ───────── 台本 ─────────

        sealed class CaseState
        {
            public string Name;
            public PaintDocument Doc;
            public byte[] Occlusion;
            public int PadWidth, PadHeight;
            public readonly List<(double, double, double, double, double, double)> Triangles = new List<(double, double, double, double, double, double)>();
            public byte[] PadImage;
            public bool[] Keep;
            public readonly List<string> Lines = new List<string>();
            public readonly MemoryStream Bin = new MemoryStream();
        }

        static void RunCases(string casesPath, string outDir)
        {
            Directory.CreateDirectory(outDir);
            foreach (var old in Directory.GetFiles(outDir, "*.bin")) File.Delete(old);
            var index = new List<string>();
            CaseState c = null;
            int lineNo = 0;
            foreach (var raw in File.ReadAllLines(casesPath))
            {
                lineNo++;
                string line = raw; int hash = line.IndexOf('#'); if (hash >= 0) line = line.Substring(0, hash);
                var t = line.Split(new[] { ' ', '\t' }, StringSplitOptions.RemoveEmptyEntries);
                if (t.Length == 0) continue;
                if (t[0] == "case") { Finish(c, index, outDir); c = new CaseState { Name = t[1] }; continue; }
                if (c == null) throw new FormatException(lineNo + ": case の前に命令がある");
                try { Command(c, t); }
                catch (Exception e) when (!(e is FormatException))
                {
                    c.Lines.Add("refused " + t[0]); // Rust は Err。理由の文は言語の違いで揃わないので書かない
                    _ = e;
                }
            }
            Finish(c, index, outDir);
            var header = new List<string> { "# 生成: tools/csharp-golden（手で書き換えない）。Unity 版の C# の Core（ExportTemplates・TexturePadding）の出力。" };
            var s = Environment.GetEnvironmentVariable("GOLDEN_SOURCE");
            if (!string.IsNullOrEmpty(s)) header.Add("# source: " + s);
            File.WriteAllText(Path.Combine(outDir, "index.txt"), string.Join("\n", header.Concat(index)) + "\n", new UTF8Encoding(false));
        }

        static void Finish(CaseState c, List<string> index, string outDir)
        {
            if (c == null) return;
            index.Add("case " + c.Name);
            index.AddRange(c.Lines);
            File.WriteAllBytes(Path.Combine(outDir, c.Name + ".bin"), c.Bin.ToArray());
        }

        static void Out(CaseState c, string label, byte[] bytes)
        {
            c.Lines.Add("out " + label + " " + bytes.Length);
            c.Bin.Write(bytes, 0, bytes.Length);
        }

        static int Int(string s) { return int.Parse(s, NumberStyles.Integer, CultureInfo.InvariantCulture); }
        static double Dbl(string s)
        {
            switch (s) { case "nan": return double.NaN; case "inf": return double.PositiveInfinity; case "-inf": return double.NegativeInfinity; }
            return double.Parse(s, NumberStyles.Float, CultureInfo.InvariantCulture);
        }
        static Rgba32 Color(string s)
        {
            var p = s.Split(',');
            if (p.Length != 4) throw new FormatException("色は R,G,B,A: " + s);
            return new Rgba32(byte.Parse(p[0]), byte.Parse(p[1]), byte.Parse(p[2]), byte.Parse(p[3]));
        }
        static LayerBlendMode Mode(string s)
        {
            LayerBlendMode m;
            if (!Enum.TryParse(s, false, out m) || !Enum.IsDefined(typeof(LayerBlendMode), m)) throw new FormatException("モード: " + s);
            return m;
        }
        static PaintChannel Chan(string s)
        {
            PaintChannel ch;
            if (!Enum.TryParse(s, false, out ch) || !Enum.IsDefined(typeof(PaintChannel), ch)) throw new FormatException("チャンネル: " + s);
            return ch;
        }
        /// <summary>レイヤーの番号（今の並び、下から 0）か、@名前。</summary>
        static Guid LayerAt(CaseState c, string s)
        {
            if (s.StartsWith("@"))
            {
                string name = s.Substring(1);
                for (int i = 0; i < c.Doc.Layers.Count; i++) if (c.Doc.Layers[i].Name == name) return c.Doc.Layers[i].Id;
                throw new FormatException("層の名前: " + s);
            }
            return c.Doc.Layers[Int(s)].Id;
        }
        static AdjustmentSettings Adjust(string s)
        {
            var parts = s.Split(':');
            switch (parts[0])
            {
                case "invert": return AdjustmentSettings.Invert();
                case "levels": { var v = parts[1].Split(','); return AdjustmentSettings.Levels(Dbl(v[0]), Dbl(v[1]), Dbl(v[2]), Dbl(v[3]), Dbl(v[4])); }
                case "hsl": { var v = parts[1].Split(','); return AdjustmentSettings.HueSaturation(Dbl(v[0]), Dbl(v[1]), Dbl(v[2])); }
                default: throw new FormatException("調整: " + s);
            }
        }
        static void ModeOpacity(PaintDocument doc, Guid id, string mode, string opacity, LayerBlendMode initial)
        {
            var m = Mode(mode); double o = Dbl(opacity);
            if (m != initial) doc.SetLayerBlendMode(id, m);
            if (o != 1) doc.SetLayerOpacity(id, o);
        }
        static void Flag(PaintDocument doc, Guid id, string flag)
        {
            if (flag == "hidden") doc.SetLayerVisibility(id, false);
            else if (flag == "clip") doc.SetLayerClipping(id, true);
            else throw new FormatException("印: " + flag);
        }

        static void Command(CaseState c, string[] t)
        {
            var doc = c.Doc;
            switch (t[0])
            {
                // ── 文書 ──
                case "canvas": c.Doc = new PaintDocument(Int(t[1]), Int(t[2]), Int(t[3])); return;
                case "layer": // layer 名前 モード 不透明度 中身 [印…]（Color のレイヤー）
                {
                    var layer = doc.AddLayer(t[1]);
                    ModeOpacity(doc, layer.Id, t[2], t[3], LayerBlendMode.Normal);
                    for (int i = 5; i < t.Length; i++) Flag(doc, layer.Id, t[i]);
                    Fill(doc, layer.GetChannel(PaintChannel.Color), t[4]);
                    doc.ClearHistory();
                    return;
                }
                case "paint": // paint レイヤー チャンネル 中身（チャンネルを有効にして埋める）
                {
                    var layer = doc.GetLayer(LayerAt(c, t[1]));
                    Fill(doc, layer.GetChannel(Chan(t[2])), t[3]);
                    doc.ClearHistory();
                    return;
                }
                case "chenable": doc.SetChannelEnabled(LayerAt(c, t[1]), Chan(t[2]), t[3] == "1"); doc.ClearHistory(); return;
                case "group":
                {
                    var g = doc.AddGroup(t[1]);
                    ModeOpacity(doc, g.Id, t[2], t[3], LayerBlendMode.PassThrough);
                    for (int i = 4; i < t.Length; i++) Flag(doc, g.Id, t[i]);
                    doc.ClearHistory();
                    return;
                }
                case "fill": // fill 名前 モード 不透明度 チャンネル=R,G,B,A… [印]
                {
                    var values = new Dictionary<PaintChannel, Rgba32>(); var flags = new List<string>();
                    for (int i = 4; i < t.Length; i++)
                    {
                        int eq = t[i].IndexOf('=');
                        if (eq < 0) flags.Add(t[i]); else values[Chan(t[i].Substring(0, eq))] = Color(t[i].Substring(eq + 1));
                    }
                    var f = doc.AddFillLayer(t[1], values);
                    ModeOpacity(doc, f.Id, t[2], t[3], LayerBlendMode.Normal);
                    foreach (var flag in flags) Flag(doc, f.Id, flag);
                    doc.ClearHistory();
                    return;
                }
                case "adjust": // adjust 名前 モード 不透明度 調整 [only=チャンネル,…] [印]
                {
                    AdjustmentSettings settings = null; List<PaintChannel> only = null; var flags = new List<string>();
                    for (int i = 4; i < t.Length; i++)
                    {
                        if (t[i] == "hidden" || t[i] == "clip") flags.Add(t[i]);
                        else if (t[i].StartsWith("only=")) { only = new List<PaintChannel>(); foreach (var n in t[i].Substring(5).Split(',')) only.Add(Chan(n)); }
                        else settings = Adjust(t[i]);
                    }
                    var a = doc.AddAdjustmentLayer(t[1], settings, only);
                    ModeOpacity(doc, a.Id, t[2], t[3], LayerBlendMode.Normal);
                    foreach (var flag in flags) Flag(doc, a.Id, flag);
                    doc.ClearHistory();
                    return;
                }
                case "normal": // normal 作る 強さ 端 向き
                {
                    var edges = t[3] == "wrap" ? HeightEdgeMode.Wrap : t[3] == "clamp" ? HeightEdgeMode.Clamp : throw new FormatException("端: " + t[3]);
                    var dir = t[4] == "dx" ? NormalYDirection.DirectX : t[4] == "gl" ? NormalYDirection.OpenGL : throw new FormatException("向き: " + t[4]);
                    doc.SetNormalSettings(new NormalSettings(t[1] == "1", Dbl(t[2]), edges, dir));
                    return;
                }
                case "occlusion": // occlusion none | random:種 | flat:値（1 テクセル 1 バイトの焼いた AO）
                {
                    if (t[1] == "none") { c.Occlusion = null; return; }
                    int n = doc.Width * doc.Height; var values = new byte[n];
                    if (t[1].StartsWith("random:")) { var rng = new SplitMix(ulong.Parse(t[1].Substring(7))); for (int i = 0; i < n; i++) values[i] = (byte)(rng.Next() >> 8); }
                    else if (t[1].StartsWith("flat:")) { byte v = byte.Parse(t[1].Substring(5)); for (int i = 0; i < n; i++) values[i] = v; }
                    else throw new FormatException("occlusion: " + t[1]);
                    c.Occlusion = values;
                    return;
                }
                case "export": // export テンプレート（画像ごとに should と、読むものが無くても作った画像）
                {
                    var template = ExportTemplate.BuiltIn.Single(x => x.Id == t[1]);
                    foreach (var image in template.Images)
                    {
                        c.Lines.Add("should " + template.Id + " " + image.Suffix + " " + (ExportTemplates.ShouldWrite(doc, image, c.Occlusion != null) ? 1 : 0));
                        Out(c, template.Id + " " + image.Suffix, ExportTemplates.Build(doc, image, c.Occlusion));
                    }
                    return;
                }
                // ── パディング ──
                case "size": c.PadWidth = Int(t[1]); c.PadHeight = Int(t[2]); return;
                case "tri": c.Triangles.Add((Dbl(t[1]), Dbl(t[2]), Dbl(t[3]), Dbl(t[4]), Dbl(t[5]), Dbl(t[6]))); return;
                case "cleartris": c.Triangles.Clear(); return;
                case "tris": // tris 種 個数 大きさ: 乱数の三角形（画像の少し外まで。4 つに 1 つは頂点を整数に丸める）
                {
                    var rng = new SplitMix(ulong.Parse(t[1])); int count = Int(t[2]); double size = Dbl(t[3]);
                    for (int i = 0; i < count; i++)
                    {
                        double x0 = (rng.U01() * 1.2 - 0.1) * c.PadWidth, y0 = (rng.U01() * 1.2 - 0.1) * c.PadHeight;
                        double x1 = x0 + (rng.U01() * 2 - 1) * size, y1 = y0 + (rng.U01() * 2 - 1) * size;
                        double x2 = x0 + (rng.U01() * 2 - 1) * size, y2 = y0 + (rng.U01() * 2 - 1) * size;
                        if (rng.Next() % 4 == 0)
                        {
                            x0 = Math.Round(x0); y0 = Math.Round(y0); x1 = Math.Round(x1); y1 = Math.Round(y1); x2 = Math.Round(x2); y2 = Math.Round(y2);
                        }
                        c.Triangles.Add((x0, y0, x1, y1, x2, y2));
                    }
                    return;
                }
                case "coverage":
                {
                    var covered = TexturePadding.Coverage(c.PadWidth, c.PadHeight, c.Triangles);
                    c.Keep = covered;
                    var bytes = new byte[covered.Length]; for (int i = 0; i < bytes.Length; i++) bytes[i] = (byte)(covered[i] ? 1 : 0);
                    Out(c, "coverage", bytes);
                    return;
                }
                case "keep": // keep none | all | random:種:パーセント
                {
                    int n = c.PadWidth * c.PadHeight; var keep = new bool[n];
                    if (t[1] == "all") for (int i = 0; i < n; i++) keep[i] = true;
                    else if (t[1].StartsWith("random:"))
                    {
                        var q = t[1].Split(':'); var rng = new SplitMix(ulong.Parse(q[1])); double percent = Dbl(q[2]);
                        for (int i = 0; i < n; i++) keep[i] = rng.U01() * 100 < percent;
                    }
                    else if (t[1] != "none") throw new FormatException("keep: " + t[1]);
                    c.Keep = keep;
                    return;
                }
                case "image": // image random:種 | opaque:種 | clear:種 | solid:R,G,B,A（パディングを掛ける画像。画素ごとに下の行から）
                {
                    int n = c.PadWidth * c.PadHeight; var image = new byte[n * 4];
                    if (t[1].StartsWith("solid:")) { var p = Color(t[1].Substring(6)); for (int i = 0; i < n; i++) { image[i * 4] = p.R; image[i * 4 + 1] = p.G; image[i * 4 + 2] = p.B; image[i * 4 + 3] = p.A; } }
                    else
                    {
                        var q = t[1].Split(':'); var rng = new SplitMix(ulong.Parse(q[1]));
                        for (int i = 0; i < n; i++)
                        {
                            var p = rng.Rgba();
                            byte a = q[0] == "opaque" ? (byte)255 : q[0] == "clear" ? (byte)0 : q[0] == "random" ? p.A : throw new FormatException("image: " + t[1]);
                            image[i * 4] = p.R; image[i * 4 + 1] = p.G; image[i * 4 + 2] = p.B; image[i * 4 + 3] = a;
                        }
                    }
                    c.PadImage = image;
                    return;
                }
                case "dilate": // dilate 段数（-1 は全部） [budget=バイト]
                {
                    long budget = long.MaxValue;
                    for (int i = 2; i < t.Length; i++) { if (t[i].StartsWith("budget=")) budget = long.Parse(t[i].Substring(7)); else throw new FormatException("dilate: " + t[i]); }
                    Out(c, "dilate " + t[1] + (budget == long.MaxValue ? "" : " budget=" + budget), TexturePadding.Dilate(c.PadImage, c.PadWidth, c.PadHeight, c.Keep, Int(t[1]), budget));
                    return;
                }
                default: throw new FormatException("命令: " + t[0]);
            }
        }

        /// <summary>レイヤーの中身。empty | random:種（キャンバスの全画素を下の行から）| sparse:種（タイルごとに 無し・一様・画素）| solid:R,G,B,A。</summary>
        static void Fill(PaintDocument doc, SparseTileSurface surface, string fill)
        {
            int w = doc.Width, h = doc.Height, ts = doc.TileSize;
            int columns = (w + ts - 1) / ts, rows = (h + ts - 1) / ts;
            if (fill == "empty") return;
            if (fill.StartsWith("random:"))
            {
                var rng = new SplitMix(ulong.Parse(fill.Substring(7)));
                var canvas = new Rgba32[w * h];
                for (int i = 0; i < canvas.Length; i++) canvas[i] = rng.Rgba();
                PutTiles(surface, w, h, ts, columns, rows, (x, y) => canvas[y * w + x]);
                return;
            }
            if (fill.StartsWith("sparse:"))
            {
                var rng = new SplitMix(ulong.Parse(fill.Substring(7)));
                for (int ty = 0; ty < rows; ty++) for (int tx = 0; tx < columns; tx++)
                {
                    ulong kind = rng.Next() % 4;
                    if (kind == 0) continue;
                    var bytes = new byte[ts * ts * 4];
                    Rgba32 uniform = kind == 1 ? rng.Rgba() : default(Rgba32);
                    for (int y = 0; y < ts && ty * ts + y < h; y++) for (int x = 0; x < ts && tx * ts + x < w; x++)
                    { var p = kind == 1 ? uniform : rng.Rgba(); int o = (y * ts + x) * 4; bytes[o] = p.R; bytes[o + 1] = p.G; bytes[o + 2] = p.B; bytes[o + 3] = p.A; }
                    surface.ImportTile(new TileCoord(tx, ty), bytes);
                }
                return;
            }
            if (fill.StartsWith("solid:"))
            {
                var p = Color(fill.Substring(6));
                PutTiles(surface, w, h, ts, columns, rows, (x, y) => p);
                return;
            }
            throw new FormatException("中身: " + fill);
        }

        static void PutTiles(SparseTileSurface surface, int w, int h, int ts, int columns, int rows, Func<int, int, Rgba32> at)
        {
            for (int ty = 0; ty < rows; ty++) for (int tx = 0; tx < columns; tx++)
            {
                var bytes = new byte[ts * ts * 4];
                for (int y = 0; y < ts && ty * ts + y < h; y++) for (int x = 0; x < ts && tx * ts + x < w; x++)
                { var p = at(tx * ts + x, ty * ts + y); int o = (y * ts + x) * 4; bytes[o] = p.R; bytes[o + 1] = p.G; bytes[o + 2] = p.B; bytes[o + 3] = p.A; }
                surface.ImportTile(new TileCoord(tx, ty), bytes);
            }
        }

        // ───────── 計測 ─────────

        static string Stats(List<double> ms)
        {
            ms.Sort();
            return string.Format(CultureInfo.InvariantCulture, "最小 {0:F2} ms / 中央 {1:F2} ms（{2} 回）", ms[0], ms[ms.Count / 2], ms.Count);
        }

        static string Time(int runs, Action action)
        {
            action();
            var ms = new List<double>();
            for (int i = 0; i < runs; i++) { var sw = Stopwatch.StartNew(); action(); ms.Add(sw.Elapsed.TotalMilliseconds); }
            return Stats(ms);
        }

        static void Bench(int runs)
        {
            Console.WriteLine("C#（Mono " + Environment.Version + "）/ 論理プロセッサ " + Environment.ProcessorCount + " / 回数 " + runs);
            const int size = 4096;
            // 全チャンネルに全タイル乱数のレイヤーを 1 つずつ（Color・Roughness・Metallic・Height・Emission）。Height → Normal は有効（Normal の出力も重い方で測る）
            var doc = new PaintDocument(size, size, 128); doc.SourceBudgetBytes = 4L << 30;
            var layer = doc.AddLayer("a");
            Fill(doc, layer.GetChannel(PaintChannel.Color), "random:1");
            int seed = 2;
            foreach (var channel in new[] { PaintChannel.Roughness, PaintChannel.Metallic, PaintChannel.Height, PaintChannel.Emission })
                Fill(doc, layer.GetChannel(channel), "random:" + seed++);
            doc.SetNormalSettings(new NormalSettings(true, 4, HeightEdgeMode.Clamp, NormalYDirection.OpenGL));
            doc.ClearHistory();
            Console.WriteLine("Build 4096²（全チャンネルが全タイル乱数の 1 レイヤー、Height → Normal 有効）:");
            foreach (var template in ExportTemplate.BuiltIn)
                foreach (var image in template.Images)
                {
                    if (image.Kind == ExportImageKind.Packed && image.Scalars.Contains(ExportScalar.Occlusion) && image.Scalars.All(s => s == ExportScalar.Occlusion || s == ExportScalar.One)) continue; // AO だけの画像は詰める作業が無い
                    var occlusion = image.Scalars.Contains(ExportScalar.Occlusion) ? new byte[size * size] : null;
                    Console.WriteLine("  " + template.Id + "/" + image.Suffix + ": " + Time(runs, () => ExportTemplates.Build(doc, image, occlusion)));
                }
            // パディング: 乱数の三角形（画像の中に大きさ 60 まで）で覆い、半径の違う塗り広げ
            var triangles = new List<(double, double, double, double, double, double)>();
            var rng = new SplitMix(7);
            for (int i = 0; i < 20000; i++)
            {
                double x0 = rng.U01() * size, y0 = rng.U01() * size;
                triangles.Add((x0, y0, x0 + (rng.U01() * 2 - 1) * 60, y0 + (rng.U01() * 2 - 1) * 60, x0 + (rng.U01() * 2 - 1) * 60, y0 + (rng.U01() * 2 - 1) * 60));
            }
            Console.WriteLine("覆い 4096²（乱数の三角形 20000 個、大きさ 60 まで）: " + Time(runs, () => TexturePadding.Coverage(size, size, triangles)));
            var keep = TexturePadding.Coverage(size, size, triangles);
            int kept = keep.Count(k => k);
            Console.WriteLine("  覆われたテクセル " + kept + "（" + (100.0 * kept / ((long)size * size)).ToString("F1", CultureInfo.InvariantCulture) + "%）");
            var pixels = new byte[size * size * 4]; var prng = new SplitMix(9);
            for (int i = 0; i < size * size; i++) { var p = prng.Rgba(); pixels[i * 4] = p.R; pixels[i * 4 + 1] = p.G; pixels[i * 4 + 2] = p.B; pixels[i * 4 + 3] = p.A; }
            foreach (int texels in new[] { 16, TexturePadding.Fill })
                Console.WriteLine("塗り広げ 4096² " + (texels == TexturePadding.Fill ? "全部（既定）" : texels + " テクセル") + ": " + Time(runs, () => TexturePadding.Dilate(pixels, size, size, keep, texels)));
            // 大きなアイランド（画像の 7 割を覆う 2 つの三角形）での覆い
            var island = new List<(double, double, double, double, double, double)> { (200, 200, 3800, 200, 3800, 3800), (200, 200, 3800, 3800, 200, 3800) };
            Console.WriteLine("覆い 4096²（大きなアイランド 2 三角形）: " + Time(runs, () => TexturePadding.Coverage(size, size, island)));
            var keepIsland = TexturePadding.Coverage(size, size, island);
            Console.WriteLine("塗り広げ 4096² 全部（大きなアイランドの外を埋める）: " + Time(runs, () => TexturePadding.Dilate(pixels, size, size, keepIsland, TexturePadding.Fill)));
        }
    }
}
