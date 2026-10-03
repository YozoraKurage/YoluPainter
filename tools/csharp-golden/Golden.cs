// YoluPainter-rs の正解のファイルを、Unity 版の C# の Core に同じ入力を通して作る（Core と一緒に組んで Mono で走らせる）。
//   golden <cases.txt> <出力のフォルダ>   台本の事例を走らせ、index.txt と <事例>.<番号>.rgba を書く
//   bench                                 4096² の合成と半径 40 のストロークの時間を測る
// 乱数・台本の読み方・出来事の書き方は crates/yolu-core/tests/golden.rs と揃えてある（片方を変えたら両方を変える）。
using System;
using System.Collections.Generic;
using System.Diagnostics;
using System.Globalization;
using System.IO;
using System.Linq;
using System.Text;
using Yozolab.YoluPainter.Core;

namespace YoluPainterRs.Golden
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
        /// <summary>不透明度の見本: 1・0・0.5・1/255・最小の非正規化数・0.99999999、ほかは一様。</summary>
        public double Opacity()
        {
            switch (Next() % 8)
            {
                case 0: return 1.0;
                case 1: return 0.0;
                case 2: return 0.5;
                case 3: return 1 / 255.0;
                case 4: return double.Epsilon;
                case 5: return 0.99999999;
                default: return U01();
            }
        }
    }

    /// <summary>FNV-1a 64。</summary>
    sealed class Fnv
    {
        public ulong Value = 14695981039346656037UL;
        public void Add(byte b) { unchecked { Value ^= b; Value *= 1099511628211UL; } }
        public void Add(Rgba32 c) { Add(c.R); Add(c.G); Add(c.B); Add(c.A); }
        public void Add(double d) { ulong bits = (ulong)BitConverter.DoubleToInt64Bits(d); for (int i = 0; i < 8; i++) Add((byte)(bits >> (8 * i))); }
        public string Hex { get { return Value.ToString("x16"); } }
    }

    static class Program
    {
        static int Main(string[] args)
        {
            try
            {
                if (args.Length == 3 && args[0] == "golden") { RunCases(args[1], args[2]); return 0; }
                if (args.Length >= 1 && args[0] == "bench") { Bench(args.Length > 1 ? int.Parse(args[1]) : 7); return 0; }
                Console.Error.WriteLine("使い方: golden <cases.txt> <出力> | bench [回数]");
                return 2;
            }
            catch (Exception e) { Console.Error.WriteLine(e); return 1; }
        }

        // ───────── 台本 ─────────

        sealed class CaseState
        {
            public string Name; public PaintDocument Doc; public BrushStroke Stroke; public int Outputs;
            public readonly Fnv Params = new Fnv(); public readonly List<string> Events = new List<string>();
        }

        static void RunCases(string casesPath, string outDir)
        {
            Directory.CreateDirectory(outDir);
            foreach (var old in Directory.GetFiles(outDir, "*.rgba")) File.Delete(old);
            var index = new List<string>();
            CaseState c = null;
            int lineNo = 0;
            foreach (var raw in File.ReadAllLines(casesPath))
            {
                lineNo++;
                string line = raw; int hash = line.IndexOf('#'); if (hash >= 0) line = line.Substring(0, hash);
                var t = line.Split(new[] { ' ', '\t' }, StringSplitOptions.RemoveEmptyEntries);
                if (t.Length == 0) continue;
                if (t[0] == "case") { Finish(c, index); c = new CaseState { Name = t[1] }; continue; }
                if (c == null) throw new FormatException(lineNo + ": case の前に命令がある");
                try { Command(c, t, outDir); }
                catch (Exception e) when (!(e is FormatException))
                {
                    // 断られた命令（Rust は Err）。ストロークの途中の失敗は Core がストロークを取り消している
                    c.Events.Add("error " + t[0]);
                    if (c.Stroke != null && c.Stroke.IsFinished) c.Stroke = null;
                    _ = e;
                }
            }
            Finish(c, index);
            Sweeps(index);
            var header = new List<string> { "# 生成: tools/csharp-golden（手で書き換えない）。Unity 版の C# の Core の出力。" };
            header.AddRange(SourceLines());
            File.WriteAllText(Path.Combine(outDir, "index.txt"), string.Join("\n", header.Concat(index)) + "\n", new UTF8Encoding(false));
        }

        static IEnumerable<string> SourceLines()
        {
            var s = Environment.GetEnvironmentVariable("GOLDEN_SOURCE");
            if (!string.IsNullOrEmpty(s)) yield return "# source: " + s;
        }

        static void Finish(CaseState c, List<string> index)
        {
            if (c == null) return;
            if (c.Stroke != null) throw new FormatException(c.Name + ": 確定も取消もしていないストロークがある");
            index.Add("case " + c.Name + " params=" + c.Params.Hex);
            index.AddRange(c.Events);
        }

        static double Num(CaseState c, string s) { double v = double.Parse(s, NumberStyles.Float, CultureInfo.InvariantCulture); c.Params.Add(v); return v; }
        static int Int(string s) { return int.Parse(s, NumberStyles.Integer, CultureInfo.InvariantCulture); }
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
        static Guid LayerAt(CaseState c, string s) { return c.Doc.Layers[Int(s)].Id; }

        static void Command(CaseState c, string[] t, string outDir)
        {
            var doc = c.Doc;
            switch (t[0])
            {
                case "canvas": c.Doc = new PaintDocument(Int(t[1]), Int(t[2]), Int(t[3])); return;
                case "layer":
                {
                    var layer = doc.AddLayer(t[1]);
                    var mode = Mode(t[2]); double opacity = Num(c, t[3]);
                    if (mode != LayerBlendMode.Normal) doc.SetLayerBlendMode(layer.Id, mode);
                    if (opacity != 1) doc.SetLayerOpacity(layer.Id, opacity);
                    string fill = null;
                    for (int i = 4; i < t.Length; i++)
                    {
                        if (t[i] == "hidden") doc.SetLayerVisibility(layer.Id, false);
                        else if (t[i] == "clip") doc.SetLayerClipping(layer.Id, true);
                        else fill = t[i];
                    }
                    if (fill == null) throw new FormatException("layer に中身が無い");
                    Fill(doc, layer, fill);
                    doc.ClearHistory();
                    return;
                }
                case "add": doc.AddLayer(t[1]); return;
                case "stroke":
                {
                    var s = new BrushSettings();
                    for (int i = 2; i < t.Length; i++)
                    {
                        int eq = t[i].IndexOf('='); string k = t[i].Substring(0, eq), v = t[i].Substring(eq + 1);
                        switch (k)
                        {
                            case "radius": s.Radius = Num(c, v); break;
                            case "hardness": s.Hardness = Num(c, v); break;
                            case "spacing": s.Spacing = Num(c, v); break;
                            case "opacity": s.Opacity = Num(c, v); break;
                            case "flow": s.Flow = Num(c, v); break;
                            case "color": s.Color = Color(v); break;
                            case "psize": s.PressureSize = v == "1"; break;
                            case "popacity": s.PressureOpacity = v == "1"; break;
                            case "pflow": s.PressureFlow = v == "1"; break;
                            case "erase": s.Erase = v == "1"; break;
                            default: throw new FormatException("stroke のキー: " + k);
                        }
                    }
                    if (c.Stroke != null) throw new FormatException("ストロークが重なっている");
                    c.Stroke = doc.BeginStroke(LayerAt(c, t[1]), PaintChannel.Color, s);
                    return;
                }
                case "point": { double x = Num(c, t[1]), y = Num(c, t[2]), p = Num(c, t[3]); c.Stroke.Add(new BrushSample(x, y, p)); return; }
                case "walk":
                {
                    var rng = new SplitMix((ulong)Int(t[1])); int count = Int(t[2]);
                    double x = Num(c, t[3]), y = Num(c, t[4]), step = Num(c, t[5]);
                    for (int i = 0; i < count; i++)
                    {
                        double p = 0.1 + 0.9 * rng.U01();
                        c.Stroke.Add(new BrushSample(x, y, p));
                        x += (rng.U01() * 2 - 1) * step;
                        y += (rng.U01() * 2 - 1) * step;
                    }
                    return;
                }
                case "pixel": { int x = Int(t[1]), y = Int(t[2]); double cov = Num(c, t[3]), p = Num(c, t[4]); c.Stroke.ApplyPixel(x, y, cov, p); return; }
                case "commit":
                {
                    var s = c.Stroke; c.Stroke = null;
                    bool changed = s.Commit();
                    c.Events.Add("commit stamps=" + s.StampCount + " samples=" + s.SampleCount + " changed=" + (changed ? 1 : 0));
                    return;
                }
                case "cancel": { var s = c.Stroke; c.Stroke = null; s.Cancel(); c.Events.Add("cancel"); return; }
                case "undo": c.Events.Add("undo " + (doc.Undo() ? 1 : 0)); return;
                case "redo": c.Events.Add("redo " + (doc.Redo() ? 1 : 0)); return;
                case "visible": doc.SetLayerVisibility(LayerAt(c, t[1]), t[2] == "1"); return;
                case "opacity": doc.SetLayerOpacity(LayerAt(c, t[1]), Num(c, t[2])); return;
                case "mode": doc.SetLayerBlendMode(LayerAt(c, t[1]), Mode(t[2])); return;
                case "clip": doc.SetLayerClipping(LayerAt(c, t[1]), t[2] == "1"); return;
                case "move": doc.MoveLayer(LayerAt(c, t[1]), Int(t[2])); return;
                case "remove": doc.RemoveLayer(LayerAt(c, t[1])); return;
                case "out":
                {
                    byte[] bytes; string what;
                    if (t[1] == "composite") { bytes = CpuCompositor.Composite(doc, PaintChannel.Color); what = "composite " + doc.Width + "x" + doc.Height; }
                    else if (t[1] == "layer") { bytes = LayerBytes(doc, Int(t[2])); what = "layer " + t[2] + " " + doc.Width + "x" + doc.Height; }
                    else if (t[1] == "region")
                    {
                        int x = Int(t[2]), y = Int(t[3]), w = Int(t[4]), h = Int(t[5]);
                        bytes = CpuCompositor.CompositeRegion(doc, PaintChannel.Color, x, y, w, h); what = "region " + x + " " + y + " " + w + "x" + h;
                    }
                    else throw new FormatException("out: " + t[1]);
                    int n = c.Outputs++;
                    File.WriteAllBytes(Path.Combine(outDir, c.Name + "." + n + ".rgba"), bytes);
                    c.Events.Add("out " + n + " " + what);
                    return;
                }
                default: throw new FormatException("命令: " + t[0]);
            }
        }

        /// <summary>層の中身。random は画布の全画素を下の行から、sparse はタイルごとに 無し・一様・画素 を選ぶ。</summary>
        static void Fill(PaintDocument doc, PaintLayer layer, string fill)
        {
            int w = doc.Width, h = doc.Height, ts = doc.TileSize;
            var surface = layer.GetChannel(PaintChannel.Color);
            int columns = (w + ts - 1) / ts, rows = (h + ts - 1) / ts;
            if (fill == "empty") return;
            if (fill.StartsWith("random:"))
            {
                var rng = new SplitMix(ulong.Parse(fill.Substring(7)));
                var canvas = new Rgba32[w * h];
                for (int i = 0; i < canvas.Length; i++) canvas[i] = rng.Rgba();
                for (int ty = 0; ty < rows; ty++) for (int tx = 0; tx < columns; tx++)
                {
                    var bytes = new byte[ts * ts * 4];
                    for (int y = 0; y < ts && ty * ts + y < h; y++) for (int x = 0; x < ts && tx * ts + x < w; x++)
                    { var p = canvas[(ty * ts + y) * w + tx * ts + x]; int o = (y * ts + x) * 4; bytes[o] = p.R; bytes[o + 1] = p.G; bytes[o + 2] = p.B; bytes[o + 3] = p.A; }
                    surface.ImportTile(new TileCoord(tx, ty), bytes);
                }
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
                for (int ty = 0; ty < rows; ty++) for (int tx = 0; tx < columns; tx++)
                {
                    var bytes = new byte[ts * ts * 4];
                    for (int y = 0; y < ts && ty * ts + y < h; y++) for (int x = 0; x < ts && tx * ts + x < w; x++)
                    { int o = (y * ts + x) * 4; bytes[o] = p.R; bytes[o + 1] = p.G; bytes[o + 2] = p.B; bytes[o + 3] = p.A; }
                    surface.ImportTile(new TileCoord(tx, ty), bytes);
                }
                return;
            }
            throw new FormatException("中身: " + fill);
        }

        static byte[] LayerBytes(PaintDocument doc, int index)
        {
            int w = doc.Width, h = doc.Height, ts = doc.TileSize;
            var surface = doc.Layers[index].Channels[PaintChannel.Color];
            var result = new byte[w * h * 4]; var tile = new byte[ts * ts * 4];
            for (int ty = 0; ty * ts < h; ty++) for (int tx = 0; tx * ts < w; tx++)
            {
                if (!surface.CopyTile(new TileCoord(tx, ty), tile)) continue;
                int cw = Math.Min(ts, w - tx * ts);
                for (int y = 0; y < ts && ty * ts + y < h; y++) Buffer.BlockCopy(tile, y * ts * 4, result, ((ty * ts + y) * w + tx * ts) * 4, cw * 4);
            }
            return result;
        }

        /// <summary>画素 1 つの式を 65536 組ずつ（モードごとの Blend・ClipOnto と Fade）。出力の指紋だけを残す。</summary>
        static void Sweeps(List<string> index)
        {
            index.Add("case sweeps params=" + new Fnv().Hex);
            foreach (LayerBlendMode mode in Enum.GetValues(typeof(LayerBlendMode)))
            {
                if (mode == LayerBlendMode.PassThrough) continue;
                var rng = new SplitMix(1000 + (ulong)mode);
                Fnv blend = new Fnv(), clip = new Fnv();
                for (int i = 0; i < 65536; i++)
                {
                    var d = rng.Rgba(); var s = rng.Rgba(); double op = rng.Opacity();
                    blend.Add(CpuCompositor.Blend(d, s, op, mode));
                    clip.Add(CpuCompositor.ClipOnto(d, s, op, mode));
                }
                index.Add("sweep blend_" + mode + " " + blend.Hex);
                index.Add("sweep clip_" + mode + " " + clip.Hex);
            }
            var fr = new SplitMix(2000); var fade = new Fnv();
            for (int i = 0; i < 65536; i++) { var d = fr.Rgba(); var s = fr.Rgba(); double op = fr.Opacity(); fade.Add(CpuCompositor.Fade(d, s, op)); }
            index.Add("sweep fade " + fade.Hex);
        }

        // ───────── 計測 ─────────

        static void FillRandom(PaintDocument doc, PaintLayer layer, ulong seed)
        {
            int ts = doc.TileSize; var rng = new SplitMix(seed); var surface = layer.GetChannel(PaintChannel.Color);
            for (int ty = 0; ty * ts < doc.Height; ty++) for (int tx = 0; tx * ts < doc.Width; tx++)
            {
                var bytes = new byte[ts * ts * 4];
                for (int i = 0; i < bytes.Length; i += 4) { var p = rng.Rgba(); bytes[i] = p.R; bytes[i + 1] = p.G; bytes[i + 2] = p.B; bytes[i + 3] = p.A; }
                surface.ImportTile(new TileCoord(tx, ty), bytes);
            }
            doc.ClearHistory();
        }

        static string Stats(List<double> ms)
        {
            ms.Sort();
            return string.Format(CultureInfo.InvariantCulture, "最小 {0:F2} ms / 中央 {1:F2} ms（{2} 回）", ms[0], ms[ms.Count / 2], ms.Count);
        }

        static void Bench(int runs)
        {
            Console.WriteLine("C#（Mono " + Environment.Version + "）/ 論理プロセッサ " + Environment.ProcessorCount + " / CoreParallelism " + CoreParallelism.Degree);
            {
                var doc = new PaintDocument(4096, 4096, 128); var l = doc.AddLayer("a"); FillRandom(doc, l, 1);
                for (int i = 0; i < 2; i++) CpuCompositor.Composite(doc, PaintChannel.Color);
                var ms = new List<double>();
                for (int i = 0; i < runs; i++) { var sw = Stopwatch.StartNew(); CpuCompositor.Composite(doc, PaintChannel.Color); ms.Add(sw.Elapsed.TotalMilliseconds); }
                Console.WriteLine("合成 4096² 1 層（全タイル乱数）: " + Stats(ms));
                var l2 = doc.AddLayer("b"); FillRandom(doc, l2, 2); doc.SetLayerBlendMode(l2.Id, LayerBlendMode.Multiply); doc.SetLayerOpacity(l2.Id, 0.7);
                for (int i = 0; i < 2; i++) CpuCompositor.Composite(doc, PaintChannel.Color);
                ms.Clear();
                for (int i = 0; i < runs; i++) { var sw = Stopwatch.StartNew(); CpuCompositor.Composite(doc, PaintChannel.Color); ms.Add(sw.Elapsed.TotalMilliseconds); }
                Console.WriteLine("合成 4096² 2 層（Normal + Multiply 0.7）: " + Stats(ms));
            }
            foreach (var (radius, over) in new[] { (40.0, false), (40.0, true), (200.0, false) })
            {
                var ms = new List<double>(); long stamps = 0;
                for (int i = 0; i < runs + 2; i++)
                {
                    var doc = new PaintDocument(4096, 4096, 128); var l = doc.AddLayer("a");
                    if (over) FillRandom(doc, l, 3);
                    var brush = new BrushSettings { Radius = radius, Hardness = 0.8, Spacing = 0.15, Color = new Rgba32(200, 60, 30, 255) };
                    var sw = Stopwatch.StartNew();
                    var s = doc.BeginStroke(l.Id, PaintChannel.Color, brush);
                    for (int k = 0; k <= 100; k++) s.Add(new BrushSample(200 + 36 * k, 2048 + 600 * Math.Sin(k * 0.1), 0.5 + 0.5 * (k % 10) / 9.0));
                    s.Commit();
                    if (i >= 2) ms.Add(sw.Elapsed.TotalMilliseconds);
                    stamps = s.StampCount;
                }
                Console.WriteLine("ストローク 半径 " + radius + "・101 点・" + stamps + " ダブ（" + (over ? "乱数の画素の上" : "空の層") + "）: " + Stats(ms));
            }
        }
    }
}
