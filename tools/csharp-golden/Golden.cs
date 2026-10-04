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
using Yozolab.YoluPainter.Core.Shelf;

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
            public string Name; public PaintDocument Doc; public BrushStroke Stroke; public int Outputs; public BrushStencil Stencil;
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
            BrushSweeps(index);
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
        /// <summary>層の番号（今の並び、下から 0）か、@名前（最初に見つかった同じ名前の層）。</summary>
        static int LayerIndex(CaseState c, string s)
        {
            if (s.StartsWith("@"))
            {
                string name = s.Substring(1);
                for (int i = 0; i < c.Doc.Layers.Count; i++) if (c.Doc.Layers[i].Name == name) return i;
                throw new FormatException("層の名前: " + s);
            }
            return Int(s);
        }
        static Guid LayerAt(CaseState c, string s) { return c.Doc.Layers[LayerIndex(c, s)].Id; }
        static PaintChannel Chan(string s)
        {
            PaintChannel ch;
            if (!Enum.TryParse(s, false, out ch) || !Enum.IsDefined(typeof(PaintChannel), ch)) throw new FormatException("チャンネル: " + s);
            return ch;
        }
        /// <summary>調整: invert | levels:入力の黒,入力の白,ガンマ,出力の黒,出力の白 | hsl:色相,彩度,明度。</summary>
        static AdjustmentSettings Adjust(CaseState c, string s)
        {
            var parts = s.Split(':');
            switch (parts[0])
            {
                case "invert": return AdjustmentSettings.Invert();
                case "levels": { var v = parts[1].Split(','); return AdjustmentSettings.Levels(Num(c, v[0]), Num(c, v[1]), Num(c, v[2]), Num(c, v[3]), Num(c, v[4])); }
                case "hsl": { var v = parts[1].Split(','); return AdjustmentSettings.HueSaturation(Num(c, v[0]), Num(c, v[1]), Num(c, v[2])); }
                default: throw new FormatException("調整: " + s);
            }
        }
        /// <summary>層の行の後ろの印（hidden・clip）を当てる。</summary>
        static void Flags(PaintDocument doc, Guid id, string flag)
        {
            if (flag == "hidden") doc.SetLayerVisibility(id, false);
            else if (flag == "clip") doc.SetLayerClipping(id, true);
            else throw new FormatException("印: " + flag);
        }
        static void ModeOpacity(CaseState c, PaintDocument doc, Guid id, string mode, string opacity, LayerBlendMode initial)
        {
            var m = Mode(mode); double o = Num(c, opacity);
            if (m != initial) doc.SetLayerBlendMode(id, m);
            if (o != 1) doc.SetLayerOpacity(id, o);
        }
        static byte[] Pixels(PaintDocument doc, PaintChannel channel, bool reference)
        {
            if (!reference) return CpuCompositor.Composite(doc, channel);
            var bytes = new byte[doc.Width * doc.Height * 4];
            for (int y = 0; y < doc.Height; y++) for (int x = 0; x < doc.Width; x++)
            {
                var p = CpuCompositor.CompositePixel(doc, channel, x, y); int i = (y * doc.Width + x) * 4;
                bytes[i] = p.R; bytes[i + 1] = p.G; bytes[i + 2] = p.B; bytes[i + 3] = p.A;
            }
            return bytes;
        }
        static string ChannelWord(PaintChannel ch) { return ch == PaintChannel.Color ? "" : ch + " "; }
        static PaintChannel Channel(string s)
        {
            PaintChannel m;
            if (!Enum.TryParse(s, false, out m) || !Enum.IsDefined(typeof(PaintChannel), m)) throw new FormatException("チャンネル: " + s);
            return m;
        }
        static BrushTip Tip(string id) { var tip = BuiltInBrushes.Tip(id); if (tip == null) throw new FormatException("筆先: " + id); return tip; }
        static bool Flag(string v) { return v == "1"; }

        /// <summary>ブラシの M1 より後のキー（Rust の golden.rs の brush_key と同じ並び・同じ読み方）。dual の後の d で始まるキーは
        /// 2 つ目の筆先の値。小数は Num（params の指紋に入る）、整数は Int。</summary>
        static bool BrushKey(CaseState c, BrushSettings s, string k, string v)
        {
            switch (k)
            {
                case "seed": s.Seed = Int(v); return true;
                case "tip": s.Tip = Tip(v); return true;
                case "tips": s.Tips = v.Split(',').Select(Tip).ToArray(); return true;
                case "tipsel": s.TipSelection = v == "seq" ? TipSelection.Sequential : v == "random" ? TipSelection.Random : throw new FormatException("tipsel: " + v); return true;
                case "angle": s.Angle = Num(c, v); return true;
                case "roundness": s.Roundness = Num(c, v); return true;
                case "follow": s.FollowDirection = Flag(v); return true;
                case "sizejit": s.SizeJitter = Num(c, v); return true;
                case "anglejit": s.AngleJitter = Num(c, v); return true;
                case "roundjit": s.RoundnessJitter = Num(c, v); return true;
                case "opjit": s.OpacityJitter = Num(c, v); return true;
                case "flowjit": s.FlowJitter = Num(c, v); return true;
                case "scatter": s.Scatter = Num(c, v); return true;
                case "count": s.Count = Int(v); return true;
                case "texture": s.Texture = Tip(v); return true;
                case "tdepth": s.TextureDepth = Num(c, v); return true;
                case "tscale": s.TextureScale = Num(c, v); return true;
                case "stabilizer": s.Stabilizer = Num(c, v); return true;
                case "taperin": s.TaperIn = Num(c, v); return true;
                case "taperout": s.TaperOut = Num(c, v); return true;
                case "curve": s.CurveInterpolation = Flag(v); return true;
                case "secondary": s.SecondaryColor = Color(v); return true;
                case "fbj": s.ForegroundBackgroundJitter = Num(c, v); return true;
                case "huejit": s.HueJitter = Num(c, v); return true;
                case "satjit": s.SaturationJitter = Num(c, v); return true;
                case "brightjit": s.BrightnessJitter = Num(c, v); return true;
                case "purity": s.Purity = Num(c, v); return true;
                case "pertip": s.ColorPerTip = Flag(v); return true;
                case "fadesize": s.FadeSize = Int(v); return true;
                case "fadeop": s.FadeOpacity = Int(v); return true;
                case "fadeflow": s.FadeFlow = Int(v); return true;
                case "tiltsize": s.TiltSize = Flag(v); return true;
                case "tiltop": s.TiltOpacity = Flag(v); return true;
                case "tiltflow": s.TiltFlow = Flag(v); return true;
                case "tiltangle": s.TiltAngle = Flag(v); return true;
                case "effect":
                {
                    BrushEffect e;
                    if (!Enum.TryParse(v, false, out e) || !Enum.IsDefined(typeof(BrushEffect), e)) throw new FormatException("effect: " + v);
                    s.Effect = e; return true;
                }
                case "blur": s.BlurRadius = Int(v); return true;
                case "smudge": s.SmudgeStrength = Num(c, v); return true;
                case "clone": { var q = v.Split(','); s.CloneOffsetX = Num(c, q[0]); s.CloneOffsetY = Num(c, q[1]); return true; }
                case "stencil": if (Flag(v)) { if (c.Stencil == null) throw new FormatException("stencil= の前に stencil 命令"); s.Stencil = c.Stencil; } return true;
                case "dual": s.Dual = new DualBrush { Tip = v == "round" ? null : Tip(v) }; return true;
                case "dradius": Dual(s).Radius = Num(c, v); return true;
                case "dhard": Dual(s).Hardness = Num(c, v); return true;
                case "dspacing": Dual(s).Spacing = Num(c, v); return true;
                case "dangle": Dual(s).Angle = Num(c, v); return true;
                case "dround": Dual(s).Roundness = Num(c, v); return true;
                case "dscatter": Dual(s).Scatter = Num(c, v); return true;
                case "dcount": Dual(s).Count = Int(v); return true;
                case "dmode":
                {
                    DualBrushMode m;
                    if (!Enum.TryParse(v, false, out m) || !Enum.IsDefined(typeof(DualBrushMode), m)) throw new FormatException("dmode: " + v);
                    Dual(s).Mode = m; return true;
                }
                default: return false;
            }
        }
        /// <summary>ステンシルの画像（straight RGBA8、下の行から）: grey:種:W:H（R = G = B と α が乱数）、color:種:W:H、varied:W:H
        /// （C# の StencilTests の Varied）、half:W:H（左が白・右が黒）。</summary>
        static byte[] StencilPixels(string spec, out int w, out int h)
        {
            var q = spec.Split(':');
            bool seeded = q[0] == "grey" || q[0] == "color";
            w = Int(q[seeded ? 2 : 1]); h = Int(q[seeded ? 3 : 2]);
            var rgba = new byte[w * h * 4];
            var rng = seeded ? new SplitMix(ulong.Parse(q[1])) : null;
            for (int y = 0; y < h; y++) for (int x = 0; x < w; x++)
            {
                Rgba32 p;
                switch (q[0])
                {
                    case "grey": { byte v = rng.Channel(); p = new Rgba32(v, v, v, rng.Alpha()); break; }
                    case "color": p = rng.Rgba(); break;
                    case "varied": p = new Rgba32((byte)(x * 13 % 256), (byte)(y * 29 % 256), (byte)((x * y + 7) % 256), 255); break;
                    case "half": p = x < w / 2 ? new Rgba32(255, 255, 255, 255) : new Rgba32(0, 0, 0, 255); break;
                    default: throw new FormatException("ステンシルの画像: " + spec);
                }
                int o = (y * w + x) * 4; rgba[o] = p.R; rgba[o + 1] = p.G; rgba[o + 2] = p.B; rgba[o + 3] = p.A;
            }
            return rgba;
        }

        static DualBrush Dual(BrushSettings s) { if (s.Dual == null) throw new FormatException("dual= の前に d のキー"); return s.Dual; }

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
                    Fill(doc, layer.GetChannel(PaintChannel.Color), fill);
                    doc.ClearHistory();
                    return;
                }
                case "paint":
                {
                    var layer = doc.GetLayer(LayerAt(c, t[1]));
                    Fill(doc, layer.GetChannel(Chan(t[2])), t[3]);
                    doc.ClearHistory();
                    return;
                }
                case "group":
                {
                    var g = doc.AddGroup(t[1]);
                    ModeOpacity(c, doc, g.Id, t[2], t[3], LayerBlendMode.PassThrough);
                    for (int i = 4; i < t.Length; i++) Flags(doc, g.Id, t[i]);
                    doc.ClearHistory();
                    return;
                }
                case "fill":
                {
                    var values = new Dictionary<PaintChannel, Rgba32>(); var flags = new List<string>();
                    for (int i = 4; i < t.Length; i++)
                    {
                        int eq = t[i].IndexOf('=');
                        if (eq < 0) flags.Add(t[i]); else values[Chan(t[i].Substring(0, eq))] = Color(t[i].Substring(eq + 1));
                    }
                    var f = doc.AddFillLayer(t[1], values);
                    ModeOpacity(c, doc, f.Id, t[2], t[3], LayerBlendMode.Normal);
                    foreach (var flag in flags) Flags(doc, f.Id, flag);
                    doc.ClearHistory();
                    return;
                }
                case "adjust":
                {
                    AdjustmentSettings settings = null; List<PaintChannel> only = null; var flags = new List<string>();
                    for (int i = 4; i < t.Length; i++)
                    {
                        if (t[i] == "hidden" || t[i] == "clip") flags.Add(t[i]);
                        else if (t[i].StartsWith("only=")) { only = new List<PaintChannel>(); foreach (var n in t[i].Substring(5).Split(',')) only.Add(Chan(n)); }
                        else settings = Adjust(c, t[i]);
                    }
                    var a = doc.AddAdjustmentLayer(t[1], settings, only);
                    ModeOpacity(c, doc, a.Id, t[2], t[3], LayerBlendMode.Normal);
                    foreach (var flag in flags) Flags(doc, a.Id, flag);
                    doc.ClearHistory();
                    return;
                }
                case "mask":
                {
                    var id = LayerAt(c, t[1]); var m = doc.AddLayerMask(id);
                    Fill(doc, m.Surface, t[2]);
                    for (int i = 3; i < t.Length; i++)
                    {
                        if (t[i] == "inverted") doc.SetLayerMaskInverted(id, true);
                        else if (t[i] == "off") doc.SetLayerMaskEnabled(id, false);
                        else if (t[i].StartsWith("density=")) doc.SetLayerMaskDensity(id, Num(c, t[i].Substring(8)));
                        else throw new FormatException("mask: " + t[i]);
                    }
                    doc.ClearHistory();
                    return;
                }
                case "add": doc.AddLayer(t[1]); return;
                case "groupof":
                {
                    var ids = new List<Guid>(); for (int i = 2; i < t.Length; i++) ids.Add(LayerAt(c, t[i]));
                    doc.GroupLayers(ids, t[1]); return;
                }
                case "ungroup": doc.Ungroup(LayerAt(c, t[1])); return;
                case "duplicate": doc.DuplicateLayer(LayerAt(c, t[1]), t.Length > 2 ? t[2] : null); return;
                case "into": doc.MoveLayerTo(LayerAt(c, t[1]), t[2] == "top" ? Guid.Empty : LayerAt(c, t[2]), Int(t[3])); return;
                case "addmask": doc.AddLayerMask(LayerAt(c, t[1])); return;
                case "removemask": doc.RemoveLayerMask(LayerAt(c, t[1])); return;
                case "maskprop":
                {
                    var id = LayerAt(c, t[1]);
                    doc.SetLayerMaskEnabled(id, t[2] == "1"); doc.SetLayerMaskInverted(id, t[3] == "1"); doc.SetLayerMaskDensity(id, Num(c, t[4]));
                    return;
                }
                case "chblend":
                {
                    LayerBlendMode? m = t[3] == "-" ? (LayerBlendMode?)null : Mode(t[3]);
                    double? o = t[4] == "-" ? (double?)null : Num(c, t[4]);
                    doc.SetChannelBlend(LayerAt(c, t[1]), Chan(t[2]), new ChannelBlend(m, o)); return;
                }
                case "chenable": doc.SetChannelEnabled(LayerAt(c, t[1]), Chan(t[2]), t[3] == "1"); return;
                case "fillvalue": doc.SetFillValue(LayerAt(c, t[1]), Chan(t[2]), t[3] == "none" ? (Rgba32?)null : Color(t[3])); return;
                case "setadjust": doc.SetAdjustment(LayerAt(c, t[1]), Adjust(c, t[2])); return;
                case "normal":
                {
                    bool derive = t[1] == "1"; double strength = Num(c, t[2]);
                    var edges = t[3] == "wrap" ? HeightEdgeMode.Wrap : t[3] == "clamp" ? HeightEdgeMode.Clamp : throw new FormatException("端: " + t[3]);
                    var dir = t[4] == "dx" ? NormalYDirection.DirectX : t[4] == "gl" ? NormalYDirection.OpenGL : throw new FormatException("向き: " + t[4]);
                    doc.SetNormalSettings(new NormalSettings(derive, strength, edges, dir)); return;
                }
                case "stroke":
                case "maskstroke":
                {
                    var s = new BrushSettings(); var channel = PaintChannel.Color;
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
                            case "channel": channel = Channel(v); break;
                            default: if (!BrushKey(c, s, k, v)) throw new FormatException("stroke のキー: " + k); break;
                        }
                    }
                    if (c.Stroke != null) throw new FormatException("ストロークが重なっている");
                    c.Stroke = t[0] == "maskstroke" ? doc.BeginMaskStroke(LayerAt(c, t[1]), s) : doc.BeginStroke(LayerAt(c, t[1]), channel, s);
                    return;
                }
                case "tpoint":
                {
                    double x = Num(c, t[1]), y = Num(c, t[2]), p = Num(c, t[3]), time = Num(c, t[4]), tx = Num(c, t[5]), ty = Num(c, t[6]);
                    c.Stroke.Add(new BrushSample(x, y, p, time, tx, ty)); return;
                }
                case "twalk":
                {
                    // 乱数の歩み（筆圧 0.1 + 0.9·u、傾き X・Y を ±1.5·(2u − 1)、次に x、y を ±歩幅·(2u − 1) 動かす。時刻 0）
                    var rng = new SplitMix((ulong)Int(t[1])); int count = Int(t[2]);
                    double x = Num(c, t[3]), y = Num(c, t[4]), step = Num(c, t[5]);
                    for (int i = 0; i < count; i++)
                    {
                        double p = 0.1 + 0.9 * rng.U01(), tx = (rng.U01() * 2 - 1) * 1.5, ty = (rng.U01() * 2 - 1) * 1.5;
                        c.Stroke.Add(new BrushSample(x, y, p, 0, tx, ty));
                        x += (rng.U01() * 2 - 1) * step;
                        y += (rng.U01() * 2 - 1) * step;
                    }
                    return;
                }
                case "dab":
                {
                    // 面のダブ: dab 中心X 中心Y 筆圧 X:Y:覆い …
                    double cx = Num(c, t[1]), cy = Num(c, t[2]), p = Num(c, t[3]);
                    var pixels = new List<BrushPixel>();
                    for (int i = 4; i < t.Length; i++) { var q = t[i].Split(':'); pixels.Add(new BrushPixel(Int(q[0]), Int(q[1]), Num(c, q[2]))); }
                    c.Stroke.ApplyDab(pixels, cx, cy, p); return;
                }
                case "dabdisc":
                {
                    // 面のダブ（円板）: dabdisc 中心X 中心Y 半径 筆圧。画素の中心が円の中なら、縁へ線形に落ちる覆い。下の行から
                    double cx = Num(c, t[1]), cy = Num(c, t[2]), r = Num(c, t[3]), p = Num(c, t[4]);
                    var pixels = new List<BrushPixel>();
                    for (int y = (int)Math.Floor(cy - r); y <= (int)Math.Ceiling(cy + r); y++)
                        for (int x = (int)Math.Floor(cx - r); x <= (int)Math.Ceiling(cx + r); x++)
                        {
                            double dx = x + 0.5 - cx, dy = y + 0.5 - cy, d = Math.Sqrt(dx * dx + dy * dy) / r;
                            if (d < 1) pixels.Add(new BrushPixel(x, y, 1 - d));
                        }
                    c.Stroke.ApplyDab(pixels, cx, cy, p); return;
                }
                case "resetdir": c.Stroke.ResetEffectDirection(); return;
                case "stencil":
                {
                    // stencil モード 繰り返し 反転 写し 画像 色空間 チャンネル（写しは none か xx,xy,x0,yx,yy,y0、チャンネルは - か名前,…）
                    StencilMode mode; StencilTiling tiling; ResourceColorSpace space;
                    if (!Enum.TryParse(t[1], false, out mode)) throw new FormatException("ステンシルのモード: " + t[1]);
                    if (!Enum.TryParse(t[2], false, out tiling)) throw new FormatException("ステンシルの繰り返し: " + t[2]);
                    bool invert = Flag(t[3]);
                    StencilMapping? mapping = null;
                    if (t[4] != "none") { var q = t[4].Split(','); mapping = new StencilMapping(Num(c, q[0]), Num(c, q[1]), Num(c, q[2]), Num(c, q[3]), Num(c, q[4]), Num(c, q[5])); }
                    var image = StencilPixels(t[5], out int iw, out int ih);
                    if (!Enum.TryParse(t[6], false, out space)) throw new FormatException("色空間: " + t[6]);
                    var channels = t[7] == "-" ? new PaintChannel[0] : t[7].Split(',').Select(Channel).ToArray();
                    c.Stencil = new BrushStencil(new StencilImage(ImageContent.FromPixels(image, iw, ih), space), mode, tiling, invert, mapping, channels);
                    return;
                }
                case "pixelat":
                {
                    int x = Int(t[1]), y = Int(t[2]); double cov = Num(c, t[3]), p = Num(c, t[4]), sx = Num(c, t[5]), sy = Num(c, t[6]), foot = Num(c, t[7]);
                    c.Stroke.ApplyPixel(x, y, cov, p, new StencilPoint(sx, sy, foot)); return;
                }
                case "dabdiscat":
                {
                    // 円板の面のダブを、画素ごとのステンシルの上の点（写し xx,xy,x0,yx,yy,y0 で画素の中心を写した点）で
                    double cx = Num(c, t[1]), cy = Num(c, t[2]), r = Num(c, t[3]), p = Num(c, t[4]);
                    var q = t[5].Split(','); var m = new StencilMapping(Num(c, q[0]), Num(c, q[1]), Num(c, q[2]), Num(c, q[3]), Num(c, q[4]), Num(c, q[5]));
                    var pixels = new List<BrushPixel>(); var points = new List<StencilPoint>();
                    for (int y = (int)Math.Floor(cy - r); y <= (int)Math.Ceiling(cy + r); y++)
                        for (int x = (int)Math.Floor(cx - r); x <= (int)Math.Ceiling(cx + r); x++)
                        {
                            double dx = x + 0.5 - cx, dy = y + 0.5 - cy, d = Math.Sqrt(dx * dx + dy * dy) / r;
                            if (d >= 1) continue;
                            pixels.Add(new BrushPixel(x, y, 1 - d));
                            m.Map(x, y, out double ix, out double iy); points.Add(new StencilPoint(ix, iy, m.Footprint));
                        }
                    c.Stroke.ApplyDab(pixels, cx, cy, p, points); return;
                }
                case "stats":
                    c.Events.Add("stats stamps=" + c.Stroke.StampCount + " samples=" + c.Stroke.SampleCount + " tiles=" + c.Stroke.ChangedTileCount + " rollback=" + c.Stroke.RollbackBytes);
                    return;
                case "budget": doc.ActiveStrokeBudgetBytes = Int(t[1]); return;
                case "enable": doc.SetChannelEnabled(LayerAt(c, t[1]), Channel(t[2]), true); doc.ClearHistory(); return;
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
                    byte[] bytes; string what; string size = doc.Width + "x" + doc.Height;
                    if (t[1] == "composite" || t[1] == "reference")
                    {
                        var ch = t.Length > 2 ? Chan(t[2]) : PaintChannel.Color;
                        bytes = Pixels(doc, ch, t[1] == "reference"); what = t[1] + " " + ChannelWord(ch) + size;
                    }
                    else if (t[1] == "layer")
                    {
                        var ch = t.Length > 3 ? Chan(t[3]) : PaintChannel.Color;
                        bytes = SurfaceBytes(doc, doc.Layers[LayerIndex(c, t[2])].Channels[ch]); what = "layer " + t[2] + " " + ChannelWord(ch) + size;
                    }
                    else if (t[1] == "mask") { bytes = SurfaceBytes(doc, doc.Layers[LayerIndex(c, t[2])].Mask.Surface); what = "mask " + t[2] + " " + size; }
                    else if (t[1] == "normal") { bytes = NormalMaps.Output(doc); what = "normal " + size; }
                    else if (t[1] == "normalfile") { bytes = NormalMaps.FileOutput(doc); what = "normalfile " + size; }
                    else if (t[1] == "derive") { bytes = NormalMaps.DeriveFromHeight(doc, PaintChannel.Height, doc.NormalSettings); what = "derive " + size; }
                    else if (t[1] == "channel") { bytes = SurfaceBytes(doc, doc.Layers[Int(t[2])].Channels[Channel(t[3])]); what = "channel " + t[2] + " " + t[3] + " " + size; }
                    else if (t[1] == "region")
                    {
                        int x = Int(t[2]), y = Int(t[3]), w = Int(t[4]), h = Int(t[5]);
                        var ch = t.Length > 6 ? Chan(t[6]) : PaintChannel.Color;
                        bytes = CpuCompositor.CompositeRegion(doc, ch, x, y, w, h); what = "region " + ChannelWord(ch) + x + " " + y + " " + w + "x" + h;
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
        static void Fill(PaintDocument doc, SparseTileSurface surface, string fill)
        {
            int w = doc.Width, h = doc.Height, ts = doc.TileSize;
            int columns = (w + ts - 1) / ts, rows = (h + ts - 1) / ts;
            if (fill == "empty") return;
            if (fill.StartsWith("m"))
            {
                // マスク: 同じ乱数の中身のアルファだけ（RGB は 0）
                Fill(doc, surface, fill.Substring(1));
                var tile = new byte[ts * ts * 4];
                for (int ty = 0; ty < rows; ty++) for (int tx = 0; tx < columns; tx++)
                {
                    if (!surface.CopyTile(new TileCoord(tx, ty), tile)) continue;
                    for (int i = 0; i < tile.Length; i += 4) { tile[i] = 0; tile[i + 1] = 0; tile[i + 2] = 0; }
                    surface.ImportTile(new TileCoord(tx, ty), tile);
                }
                return;
            }
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

        static byte[] LayerBytes(PaintDocument doc, int index, PaintChannel channel = PaintChannel.Color) { return SurfaceBytes(doc, doc.Layers[index].Channels[channel]); }

        static byte[] SurfaceBytes(PaintDocument doc, SparseTileSurface surface)
        {
            int w = doc.Width, h = doc.Height, ts = doc.TileSize;
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
            // Normal のチャンネルの式（単位ベクトルの重ね・クリッピング・フェード）
            foreach (LayerBlendMode mode in Enum.GetValues(typeof(LayerBlendMode)))
            {
                if (mode == LayerBlendMode.PassThrough) continue;
                var rng = new SplitMix(3000 + (ulong)mode);
                Fnv blend = new Fnv(), clip = new Fnv();
                for (int i = 0; i < 65536; i++)
                {
                    var d = rng.Rgba(); var s = rng.Rgba(); double op = rng.Opacity();
                    blend.Add(NormalMaps.Blend(d, s, op, mode));
                    clip.Add(NormalMaps.ClipOnto(d, s, op, mode));
                }
                index.Add("sweep nblend_" + mode + " " + blend.Hex);
                index.Add("sweep nclip_" + mode + " " + clip.Hex);
            }
            var nr = new SplitMix(4000); var nfade = new Fnv();
            for (int i = 0; i < 65536; i++) { var d = nr.Rgba(); var s = nr.Rgba(); double op = nr.Opacity(); nfade.Add(NormalMaps.Fade(d, s, op)); }
            index.Add("sweep nfade " + nfade.Hex);
            // 調整（設定 × モードごとに 4096 画素）
            var adjustments = new[]
            {
                AdjustmentSettings.Invert(), AdjustmentSettings.Levels(.1, .9, 1.7, .05, .95), AdjustmentSettings.Levels(0, 1, .37, .2, .8),
                AdjustmentSettings.HueSaturation(73, -.4, .2), AdjustmentSettings.HueSaturation(-150, .8, -.6),
            };
            for (int k = 0; k < adjustments.Length; k++)
                foreach (LayerBlendMode mode in Enum.GetValues(typeof(LayerBlendMode)))
                {
                    if (mode == LayerBlendMode.PassThrough) continue;
                    var rng = new SplitMix(5000 + 100 * (ulong)k + (ulong)mode); var adjusted = new Fnv();
                    for (int i = 0; i < 4096; i++) { var d = rng.Rgba(); double op = rng.Opacity(); adjusted.Add(adjustments[k].Composite(d, op, mode)); }
                    index.Add("sweep adjust" + k + "_" + mode + " " + adjusted.Hex);
                }
        }

        /// <summary>ブラシの式の掃引（出力の指紋だけ）: System.Random の列、組み込みの筆先の画素、筆先の双線形、色の変化、デュアルの合わせ方、
        /// ペンの傾き、曲線の点。</summary>
        static void BrushSweeps(List<string> index)
        {
            index.Add("case brush_sweeps params=" + new Fnv().Hex);
            {
                var f = new Fnv();
                foreach (int seed in new[] { 0, 1, -1, 7, int.MinValue, int.MaxValue, 0x2545F491, 0x5DEECE6, 123456789 })
                {
                    var r = new Random(seed);
                    for (int i = 0; i < 4096; i++) f.Add(r.NextDouble());
                    for (int i = 1; i < 300; i++) f.Add((double)r.Next(i));
                }
                index.Add("sweep random " + f.Hex);
            }
            foreach (var id in new[] { "grain", "noisy-disc", "charcoal", "bristles", "dots", "rim", "rounded-square" })
            {
                var tip = BuiltInBrushes.Tip(id); var f = new Fnv();
                f.Add((byte)tip.Width); f.Add((byte)(tip.Width >> 8)); f.Add((byte)tip.Height); f.Add((byte)(tip.Height >> 8));
                foreach (byte b in tip.CopyAlpha()) f.Add(b);
                index.Add("sweep tip_" + id + " " + f.Hex);
            }
            {
                var rng = new SplitMix(3000); var f = new Fnv(); var tip = BuiltInBrushes.Tip("charcoal"); var grain = BuiltInBrushes.Tip("grain");
                for (int i = 0; i < 65536; i++)
                {
                    double u = rng.U01() * 1.2 - 0.1, v = rng.U01() * 1.2 - 0.1;
                    f.Add(tip.Sample(u, v)); f.Add(grain.SampleTiled((rng.U01() - 0.5) * 1000, (rng.U01() - 0.5) * 1000));
                }
                index.Add("sweep tip_sample " + f.Hex);
            }
            {
                var rng = new SplitMix(3001); var f = new Fnv();
                for (int i = 0; i < 4096; i++)
                {
                    var s = new BrushSettings
                    {
                        Color = rng.Rgba(), SecondaryColor = rng.Rgba(),
                        ForegroundBackgroundJitter = (rng.Next() & 1) == 0 ? 0 : rng.U01(), HueJitter = (rng.Next() & 1) == 0 ? 0 : rng.U01(),
                        SaturationJitter = (rng.Next() & 1) == 0 ? 0 : rng.U01(), BrightnessJitter = (rng.Next() & 1) == 0 ? 0 : rng.U01(),
                        Purity = (rng.Next() % 3) == 0 ? 0 : rng.U01() * 2 - 1,
                    };
                    var r = new Random((int)(rng.Next() & 0x7fffffff));
                    for (int k = 0; k < 8; k++) f.Add(ColorDynamics.Next(s, r));
                }
                index.Add("sweep color_dynamics " + f.Hex);
            }
            {
                var rng = new SplitMix(3002); var f = new Fnv();
                foreach (DualBrushMode mode in Enum.GetValues(typeof(DualBrushMode)))
                    for (int i = 0; i < 16384; i++)
                    {
                        double a = (rng.Next() % 5) == 0 ? (rng.Next() % 3) * 0.5 : rng.U01(), b = (rng.Next() % 5) == 0 ? (rng.Next() % 3) * 0.5 : rng.U01();
                        f.Add(DualBrush.Combine(mode, a, b));
                    }
                index.Add("sweep dual_combine " + f.Hex);
            }
            {
                var rng = new SplitMix(3003); var f = new Fnv();
                for (int i = 0; i < 65536; i++)
                {
                    double tx = (rng.Next() % 7) == 0 ? 0 : (rng.U01() * 2 - 1) * PenTilt.MaxAngle, ty = (rng.Next() % 7) == 0 ? 0 : (rng.U01() * 2 - 1) * PenTilt.MaxAngle;
                    f.Add(PenTilt.Amount(tx, ty)); f.Add(PenTilt.Azimuth(tx, ty));
                }
                index.Add("sweep pen_tilt " + f.Hex);
            }
            {
                var rng = new SplitMix(3004); var f = new Fnv();
                for (int i = 0; i < 65536; i++)
                {
                    var q = new double[8]; for (int k = 0; k < 8; k++) q[k] = (rng.U01() - 0.5) * 200;
                    if ((rng.Next() % 9) == 0) { q[0] = q[2]; q[1] = q[3]; }
                    StrokeCurve.Point(q[0], q[1], q[2], q[3], q[4], q[5], q[6], q[7], rng.U01(), out double x, out double y);
                    f.Add(x); f.Add(y);
                }
                index.Add("sweep curve " + f.Hex);
            }
            {
                var rng = new SplitMix(3005); var f = new Fnv();
                for (int i = 0; i < 65536; i++)
                {
                    ColorDynamics.RgbToHsv(rng.U01(), rng.U01(), rng.U01(), out double h, out double sat, out double val);
                    f.Add(h); f.Add(sat); f.Add(val);
                    ColorDynamics.HsvToRgb(rng.U01() * 3 - 1, rng.U01(), rng.U01(), out double r, out double g, out double b);
                    f.Add(r); f.Add(g); f.Add(b);
                }
                index.Add("sweep hsv " + f.Hex);
            }
            {
                // sRGB の表と輝度
                var f = new Fnv();
                for (int i = 0; i < 256; i++) f.Add(FillImageColor.Convert(FillImageConversion.LinearToSrgb, (byte)i));
                var rng = new SplitMix(3006);
                for (int i = 0; i < 65536; i++) f.Add(FillImageColor.Luminance(rng.Channel(), rng.Channel(), rng.Channel()));
                index.Add("sweep srgb_luminance " + f.Hex);
            }
            foreach (var spec in new[] { "grey:41:37:23", "color:42:64:48", "color:43:1:7", "half:5:5" })
            {
                // ステンシルの画像の読み（双線形・三線形・繰り返し・画像の外）
                var pixels = StencilPixels(spec, out int w, out int h);
                var image = new StencilImage(ImageContent.FromPixels(pixels, w, h), ResourceColorSpace.Srgb);
                var rng = new SplitMix(3007); var f = new Fnv();
                var footprints = new[] { 0.0, 0.5, 1.0, 1.7, 3.0, 9.0, 100.0, 1e6 };
                for (int i = 0; i < 16384; i++)
                {
                    double x = (rng.U01() * 2 - 0.5) * w, y = (rng.U01() * 2 - 0.5) * h, foot = footprints[rng.Next() % 8];
                    var tiling = (StencilTiling)(rng.Next() % 4);
                    var t = image.Read(x, y, foot, tiling);
                    f.Add((byte)(t.Inside ? 1 : 0)); f.Add(t.Alpha); f.Add(t.LumaAlpha); f.Add(t.Color);
                }
                f.Add((double)image.MipBytes);
                index.Add("sweep stencil_" + spec.Replace(':', '_') + " " + f.Hex);
            }
        }

        // ───────── 計測 ─────────

        static void FillRandom(PaintDocument doc, PaintLayer layer, ulong seed) { FillRandom(doc, layer.GetChannel(PaintChannel.Color), seed, false); }
        /// <summary>全タイルを乱数で（alphaOnly ならマスク: アルファだけ）。</summary>
        static void FillRandom(PaintDocument doc, SparseTileSurface surface, ulong seed, bool alphaOnly)
        {
            int ts = doc.TileSize; var rng = new SplitMix(seed);
            for (int ty = 0; ty * ts < doc.Height; ty++) for (int tx = 0; tx * ts < doc.Width; tx++)
            {
                var bytes = new byte[ts * ts * 4];
                for (int i = 0; i < bytes.Length; i += 4) { var p = rng.Rgba(); if (!alphaOnly) { bytes[i] = p.R; bytes[i + 1] = p.G; bytes[i + 2] = p.B; } bytes[i + 3] = p.A; }
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
                // M2: Rust の bench と同じ文書
                doc.SourceBudgetBytes = 2L << 30;
                var l3 = doc.AddLayer("c"); FillRandom(doc, l3, 3); doc.SetLayerBlendMode(l3.Id, LayerBlendMode.Screen); doc.SetLayerOpacity(l3.Id, 0.8);
                FillRandom(doc, doc.AddLayerMask(l3.Id).Surface, 4, true); doc.ClearHistory();
                var g = doc.GroupLayers(new[] { l2.Id, l3.Id }, "G"); doc.SetLayerOpacity(g.Id, 0.6);
                var l4 = doc.AddLayer("d"); FillRandom(doc, l4, 5);
                var inner = doc.GroupLayers(new[] { l4.Id }, "Inner"); doc.SetLayerBlendMode(inner.Id, LayerBlendMode.Overlay);
                var lv = doc.AddAdjustmentLayer("lv", AdjustmentSettings.Levels(0.1, 0.9, 1.4, 0, 1)); doc.SetLayerOpacity(lv.Id, 0.8);
                var f = doc.AddFillLayer("f", new Dictionary<PaintChannel, Rgba32> { { PaintChannel.Color, new Rgba32(30, 90, 200, 255) } });
                doc.SetLayerBlendMode(f.Id, LayerBlendMode.Multiply); doc.SetLayerOpacity(f.Id, 0.25);
                for (int i = 0; i < 2; i++) CpuCompositor.Composite(doc, PaintChannel.Color);
                ms.Clear();
                for (int i = 0; i < runs; i++) { var sw = Stopwatch.StartNew(); CpuCompositor.Composite(doc, PaintChannel.Color); ms.Add(sw.Elapsed.TotalMilliseconds); }
                Console.WriteLine("合成 4096² グループの文書（通過 0.6 に Multiply・マスク付き Screen、分離の Overlay、レベル補正、塗りつぶし）: " + Stats(ms));
            }
            {
                var doc = new PaintDocument(4096, 4096, 128); doc.SourceBudgetBytes = 2L << 30;
                var a = doc.AddLayer("a"); FillRandom(doc, a.GetChannel(PaintChannel.Normal), 6, false);
                var b = doc.AddLayer("b"); FillRandom(doc, b.GetChannel(PaintChannel.Normal), 7, false);
                doc.SetLayerBlendMode(b.Id, LayerBlendMode.Overlay); doc.SetLayerOpacity(b.Id, 0.6);
                var h = doc.AddLayer("h"); FillRandom(doc, h.GetChannel(PaintChannel.Height), 8, false);
                doc.ClearHistory();
                var ms = new List<double>();
                for (int i = 0; i < runs + 2; i++) { var sw = Stopwatch.StartNew(); CpuCompositor.Composite(doc, PaintChannel.Normal); if (i >= 2) ms.Add(sw.Elapsed.TotalMilliseconds); }
                Console.WriteLine("合成 4096² Normal 2 層（Normal + Overlay 0.6、ベクトル）: " + Stats(ms));
                doc.SetNormalSettings(new NormalSettings(true, 4, HeightEdgeMode.Clamp, NormalYDirection.OpenGL));
                ms.Clear();
                for (int i = 0; i < runs + 2; i++) { var sw = Stopwatch.StartNew(); NormalMaps.Output(doc, 1L << 30); if (i >= 2) ms.Add(sw.Elapsed.TotalMilliseconds); }
                Console.WriteLine("Normal の出力 4096²（上の 2 層 + Height → Normal）: " + Stats(ms));
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
            // M2 のブラシ（crates/yolu-core/examples/bench.rs の dynamic_brush と同じ設定）
            string only = Environment.GetEnvironmentVariable("BENCH_ONLY");
            foreach (double radius in new[] { 40.0, 200.0 })
                foreach (var kind in new[] { "round", "jitter", "tip", "texture", "dual", "color", "all", "blur", "smudge" })
                {
                    if (!string.IsNullOrEmpty(only) && only != kind) continue;
                    bool over = kind == "blur" || kind == "smudge";
                    var ms = new List<double>(); long stamps = 0;
                    for (int i = 0; i < runs + 2; i++)
                    {
                        var doc = new PaintDocument(4096, 4096, 128); var l = doc.AddLayer("a");
                        if (over) FillRandom(doc, l, 3);
                        var brush = DynamicBrush(kind, radius);
                        var sw = Stopwatch.StartNew();
                        var s = doc.BeginStroke(l.Id, PaintChannel.Color, brush);
                        for (int k = 0; k <= 100; k++) s.Add(new BrushSample(200 + 36 * k, 2048 + 600 * Math.Sin(k * 0.1), 0.5 + 0.5 * (k % 10) / 9.0));
                        s.Commit();
                        if (i >= 2) ms.Add(sw.Elapsed.TotalMilliseconds);
                        stamps = s.StampCount;
                    }
                    Console.WriteLine("M2 " + kind + " 半径 " + radius + "・" + stamps + " ダブ" + (over ? "・乱数の画素の上" : "") + ": " + Stats(ms));
                }
        }

        static BrushSettings DynamicBrush(string kind, double radius)
        {
            var b = new BrushSettings { Radius = radius, Hardness = 0.8, Spacing = 0.15, Color = new Rgba32(200, 60, 30, 255) };
            void Jitter() { b.Seed = 7; b.SizeJitter = 0.3; b.AngleJitter = 0.5; b.RoundnessJitter = 0.3; b.Scatter = 0.3; b.OpacityJitter = 0.2; b.FlowJitter = 0.2; }
            void Tip() { b.Tip = BuiltInBrushes.Tip("charcoal"); b.FollowDirection = true; }
            void Texture() { b.Texture = BuiltInBrushes.Tip("grain"); b.TextureDepth = 0.6; b.TextureScale = 2; }
            void Dual() { b.Dual = new DualBrush { Radius = radius / 4, Spacing = 0.25, Scatter = 0.5, Count = 2, Hardness = 0.5 }; }
            void Colour() { b.Seed = 3; b.HueJitter = 0.3; b.BrightnessJitter = 0.2; }
            switch (kind)
            {
                case "jitter": Jitter(); break;
                case "tip": Tip(); break;
                case "texture": Texture(); break;
                case "dual": Dual(); break;
                case "color": Colour(); break;
                case "all": Jitter(); Tip(); Texture(); Dual(); Colour(); break;
                case "blur": b.Effect = BrushEffect.Blur; b.BlurRadius = 3; break;
                case "smudge": b.Effect = BrushEffect.Smudge; b.SmudgeStrength = 0.5; break;
            }
            return b;
        }
    }
}
