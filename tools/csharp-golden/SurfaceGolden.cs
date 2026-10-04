// YoluPainter-rs の面の計算（yolu-core の geometry）の正解のファイルを、Unity 版の SurfaceGeometry（Editor/Preview の原文）に同じ入力を
// 通して作る。UnityEngine の Vector3・Mathf・Bounds・Ray は本物の UnityEngine.CoreModule.dll のもの（Unity の外の Mono でも動く C# の部分）を
// 使い、ネイティブの Bounds.SqrDistance だけを、その C++ の式を写した NativeBounds.SqrDistance に原文の呼び出しごと置き換えて組む（run.sh）。
//   golden <surface/cases.txt> <出力のフォルダ>   台本の事例を走らせ、index.txt を書く
//   bench [回数]                                  7 万三角形の球で、組み立て・レイ 1 本・ダブの時間を測る（Mono）
// 形の作り方・乱数・台本の読み方・出力の書き方は crates/yolu-core/tests/surface_golden.rs と揃えてある（片方を変えたら両方を変える）。
using System;
using System.Collections;
using System.Collections.Generic;
using System.Diagnostics;
using System.Globalization;
using System.IO;
using System.Linq;
using System.Reflection;
using System.Text;
using UnityEngine;
using Yozolab.YoluPainter.Editor.Preview;

namespace Yozolab.YoluPainter.Editor.Preview
{
    /// <summary>Unity のネイティブの Bounds.SqrDistance（CalculateSqrDistance: 中心からの差を半分の大きさの外へ出た分だけ 2 乗して足す）。</summary>
    static class NativeBounds
    {
        public static float SqrDistance(Bounds b, Vector3 p)
        {
            Vector3 closest = p - b.center; Vector3 e = b.extents; float sum = 0;
            for (int i = 0; i < 3; i++)
            {
                float c = closest[i], x = e[i];
                if (c < -x) { float d = c + x; sum += d * d; }
                else if (c > x) { float d = c - x; sum += d * d; }
            }
            return sum;
        }
    }
}

namespace YoluPainterRs.SurfaceGolden
{
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
        /// <summary>−1〜1 の単精度（(float)(u × 2 − 1)）。</summary>
        public float S() { return (float)(U01() * 2 - 1); }
        public float F() { return (float)U01(); }
    }

    sealed class Fnv
    {
        public ulong Value = 14695981039346656037UL;
        public void Byte(byte b) { unchecked { Value ^= b; Value *= 1099511628211UL; } }
        public void Int(int v) { for (int i = 0; i < 4; i++) Byte((byte)(v >> (8 * i))); }
        public void Long(long v) { for (int i = 0; i < 8; i++) Byte((byte)(v >> (8 * i))); }
        public void Float(float f) { Int(BitConverter.ToInt32(BitConverter.GetBytes(f), 0)); }
        public void V3(Vector3 v) { Float(v.x); Float(v.y); Float(v.z); }
        public void V2(Vector2 v) { Float(v.x); Float(v.y); }
        public string Hex { get { return Value.ToString("x16"); } }
    }

    static class Program
    {
        static int Main(string[] args)
        {
            try
            {
                if (args.Length == 3 && args[0] == "golden") { Run(args[1], args[2]); return 0; }
                if (args.Length >= 1 && args[0] == "bench") { Bench(args.Length > 1 ? int.Parse(args[1]) : 5); return 0; }
                Console.Error.WriteLine("使い方: golden <surface/cases.txt> <出力> | bench [回数]");
                return 2;
            }
            catch (Exception e) { Console.Error.WriteLine(e); return 1; }
        }

        static string H(float f) => BitConverter.ToInt32(BitConverter.GetBytes(f), 0).ToString("x8");
        static string H3(Vector3 v) => H(v.x) + "," + H(v.y) + "," + H(v.z);
        static string H2(Vector2 v) => H(v.x) + "," + H(v.y);

        // ───────── 形 ─────────

        static void AddCube(List<SurfaceTriangle> list, int renderer, int slot, int material)
        {
            // Unity 版の LoadDemoSnapshot と同じ面・UV・添字
            var v = new List<Vector3>(); var uv = new List<Vector2>(); var idx = new List<int>();
            void Face(Vector3 a, Vector3 b, Vector3 c, Vector3 d)
            {
                int start = v.Count, face = start / 4;
                v.Add(a); v.Add(b); v.Add(c); v.Add(d);
                float u = (face % 3) / 3f + 0.02f, vv = (face / 3) * 0.5f + 0.03f;
                float width = 1f / 3f - 0.04f, height = 0.44f;
                uv.Add(new Vector2(u, vv)); uv.Add(new Vector2(u, vv + height));
                uv.Add(new Vector2(u + width, vv)); uv.Add(new Vector2(u + width, vv + height));
                idx.Add(start); idx.Add(start + 1); idx.Add(start + 2);
                idx.Add(start + 2); idx.Add(start + 1); idx.Add(start + 3);
            }
            Face(new Vector3(-.5f, -.5f, -.5f), new Vector3(-.5f, .5f, -.5f), new Vector3(.5f, -.5f, -.5f), new Vector3(.5f, .5f, -.5f));
            Face(new Vector3(.5f, -.5f, .5f), new Vector3(.5f, .5f, .5f), new Vector3(-.5f, -.5f, .5f), new Vector3(-.5f, .5f, .5f));
            Face(new Vector3(-.5f, -.5f, .5f), new Vector3(-.5f, .5f, .5f), new Vector3(-.5f, -.5f, -.5f), new Vector3(-.5f, .5f, -.5f));
            Face(new Vector3(.5f, -.5f, -.5f), new Vector3(.5f, .5f, -.5f), new Vector3(.5f, -.5f, .5f), new Vector3(.5f, .5f, .5f));
            Face(new Vector3(-.5f, .5f, -.5f), new Vector3(-.5f, .5f, .5f), new Vector3(.5f, .5f, -.5f), new Vector3(.5f, .5f, .5f));
            Face(new Vector3(-.5f, -.5f, .5f), new Vector3(-.5f, -.5f, -.5f), new Vector3(.5f, -.5f, .5f), new Vector3(.5f, -.5f, -.5f));
            for (int i = 0; i < idx.Count; i += 3)
                list.Add(new SurfaceTriangle(v[idx[i]], v[idx[i + 1]], v[idx[i + 2]], uv[idx[i]], uv[idx[i + 1]], uv[idx[i + 2]], renderer, slot, material));
        }

        /// <summary>yolu-core の cube_sphere と同じ式（faces は含める面の番号の文字）。</summary>
        static void AddSphere(List<SurfaceTriangle> list, int n, float radius, string faces, int renderer, int slot, int material)
        {
            var frames = new[]
            {
                (new Vector3(0, 0, -1), new Vector3(1, 0, 0), new Vector3(0, 1, 0)),
                (new Vector3(0, 0, 1), new Vector3(-1, 0, 0), new Vector3(0, 1, 0)),
                (new Vector3(-1, 0, 0), new Vector3(0, 0, -1), new Vector3(0, 1, 0)),
                (new Vector3(1, 0, 0), new Vector3(0, 0, 1), new Vector3(0, 1, 0)),
                (new Vector3(0, 1, 0), new Vector3(1, 0, 0), new Vector3(0, 0, 1)),
                (new Vector3(0, -1, 0), new Vector3(1, 0, 0), new Vector3(0, 0, -1)),
            };
            float nf = n;
            for (int face = 0; face < 6; face++)
            {
                var (center, right, up) = frames[face];
                float u0 = (face % 3) / 3f + 0.01f, v0 = (face / 3) * 0.5f + 0.01f;
                float w = 1f / 3f - 0.02f, h = 0.48f;
                var pos = new Vector3[(n + 1) * (n + 1)]; var uvs = new Vector2[(n + 1) * (n + 1)];
                for (int j = 0; j <= n; j++)
                    for (int i = 0; i <= n; i++)
                    {
                        float s = (float)(2 * i - n) / nf, t = (float)(2 * j - n) / nf;
                        var p = new Vector3(center.x + right.x * s + up.x * t, center.y + right.y * s + up.y * t, center.z + right.z * s + up.z * t);
                        float len = (float)Math.Sqrt(p.x * p.x + p.y * p.y + p.z * p.z);
                        pos[j * (n + 1) + i] = new Vector3(p.x / len * radius, p.y / len * radius, p.z / len * radius);
                        uvs[j * (n + 1) + i] = new Vector2(u0 + w * (i / nf), v0 + h * (j / nf));
                    }
                if (faces.IndexOf((char)('0' + face)) < 0) continue;
                int row = n + 1;
                for (int j = 0; j < n; j++)
                    for (int i = 0; i < n; i++)
                    {
                        int a = j * row + i, b = a + row, c = a + 1, d = a + row + 1;
                        list.Add(new SurfaceTriangle(pos[a], pos[b], pos[c], uvs[a], uvs[b], uvs[c], renderer, slot, material));
                        list.Add(new SurfaceTriangle(pos[c], pos[b], pos[d], uvs[c], uvs[b], uvs[d], renderer, slot, material));
                    }
            }
        }

        static void AddPlate(List<SurfaceTriangle> list, float x0, float y0, float x1, float y1, float z, int renderer, int slot, int material)
        {
            Vector3 a = new Vector3(x0, y0, z), b = new Vector3(x0, y1, z), c = new Vector3(x1, y0, z), d = new Vector3(x1, y1, z);
            list.Add(new SurfaceTriangle(a, b, c, new Vector2(0, 0), new Vector2(0, 1), new Vector2(1, 0), renderer, slot, material));
            list.Add(new SurfaceTriangle(c, b, d, new Vector2(1, 0), new Vector2(0, 1), new Vector2(1, 1), renderer, slot, material));
        }

        static void AddSoup(List<SurfaceTriangle> list, ulong seed, int count, float scale, int renderer, int slot, int material)
        {
            var r = new SplitMix(seed);
            for (int k = 0; k < count; k++)
            {
                float ax = r.S(), ay = r.S(), az = r.S();
                var a = new Vector3(ax, ay, az) * scale;
                float bx = r.F(), by = r.F(), bz = r.F();
                var b = a + new Vector3(bx * 0.2f, by * 0.2f, bz * 0.2f) * scale;
                float cx = r.F(), cy = r.F(), cz = r.F();
                var c = a + new Vector3(cx * 0.2f, cy * 0.2f, cz * 0.2f) * scale;
                float u0 = r.F(), v0 = r.F(), u1 = r.F(), v1 = r.F(), u2 = r.F(), v2 = r.F();
                list.Add(new SurfaceTriangle(a, b, c, new Vector2(u0, v0), new Vector2(u1, v1), new Vector2(u2, v2), renderer, slot, material));
            }
        }

        // ───────── 台本 ─────────

        sealed class Case
        {
            public string Name; public List<SurfaceTriangle> Triangles = new List<SurfaceTriangle>(); public SurfaceGeometry Geometry;
            public SurfaceBrushBudget Budget = new SurfaceBrushBudget(); public SurfaceVisibilityCache Cache; public List<string> Outputs = new List<string>();
            public Fnv Params = new Fnv();
        }

        static float Fl(Case c, string s)
        {
            float v = s == "inf" ? float.PositiveInfinity : float.Parse(s, NumberStyles.Float, CultureInfo.InvariantCulture);
            c.Params.Float(v); return v;
        }
        static int In(Case c, string s) { int v = int.Parse(s, CultureInfo.InvariantCulture); c.Params.Int(v); return v; }

        static void Run(string casesPath, string outDir)
        {
            var cases = new List<Case>(); Case cur = null;
            foreach (var raw in File.ReadAllLines(casesPath))
            {
                int hash = raw.IndexOf('#'); var line = (hash >= 0 ? raw.Substring(0, hash) : raw).Trim();
                if (line.Length == 0) continue;
                var t = line.Split((char[])null, StringSplitOptions.RemoveEmptyEntries);
                if (t[0] == "case") { cur = new Case { Name = t[1] }; cases.Add(cur); continue; }
                if (cur == null) throw new InvalidDataException("case の前の命令: " + line);
                foreach (var tok in t) foreach (char ch in tok) cur.Params.Byte((byte)ch);
                Step(cur, t);
            }
            var sb = new StringBuilder();
            sb.Append("# 生成: tools/csharp-golden（手で書き換えない）。Unity 版の SurfaceGeometry（Editor/Preview の原文）の出力。\n");
            sb.Append("# source: " + (Environment.GetEnvironmentVariable("GOLDEN_SOURCE") ?? "不明") + "\n");
            foreach (var c in cases)
            {
                sb.Append("case " + c.Name + " params=" + c.Params.Hex + "\n");
                for (int i = 0; i < c.Outputs.Count; i++) sb.Append("out " + i + " " + c.Outputs[i] + "\n");
            }
            Directory.CreateDirectory(outDir);
            File.WriteAllText(Path.Combine(outDir, "index.txt"), sb.ToString());
        }

        static void Step(Case c, string[] t)
        {
            var g = c.Geometry;
            switch (t[0])
            {
                case "clear": c.Triangles.Clear(); break;
                case "cube": AddCube(c.Triangles, In(c, t[1]), In(c, t[2]), In(c, t[3])); break;
                case "sphere": AddSphere(c.Triangles, In(c, t[1]), Fl(c, t[2]), t.Length > 6 ? t[6] : "012345", In(c, t[3]), In(c, t[4]), In(c, t[5])); break;
                case "plate": AddPlate(c.Triangles, Fl(c, t[1]), Fl(c, t[2]), Fl(c, t[3]), Fl(c, t[4]), Fl(c, t[5]), In(c, t[6]), In(c, t[7]), In(c, t[8])); break;
                case "soup": AddSoup(c.Triangles, ulong.Parse(t[1]), In(c, t[2]), Fl(c, t[3]), In(c, t[4]), In(c, t[5]), In(c, t[6])); break;
                case "build": c.Geometry = new SurfaceGeometry(c.Triangles, In(c, t[1]), t.Length > 2 ? Fl(c, t[2]) : 0.000001f); break;
                case "budget":
                    c.Budget = new SurfaceBrushBudget();
                    for (int i = 1; i < t.Length; i++)
                    {
                        if (t[i] == "default") continue;
                        var kv = t[i].Split('='); int v = In(c, kv[1]);
                        switch (kv[0])
                        {
                            case "triangles": c.Budget.MaxTriangles = v; break;
                            case "pixels": c.Budget.MaxCandidatePixels = v; break;
                            case "rays": c.Budget.MaxVisibilityRays = v; break;
                            case "tests": c.Budget.MaxRayTriangleTests = v; break;
                            case "visits": c.Budget.MaxRayNodeVisits = v; break;
                            default: throw new InvalidDataException("予算の鍵: " + kv[0]);
                        }
                    }
                    break;
                case "cache": c.Cache = t[1] == "new" ? new SurfaceVisibilityCache() : null; break;
                case "out": c.Outputs.Add(Structure(g)); break;
                case "ray":
                {
                    var o = new Vector3(Fl(c, t[1]), Fl(c, t[2]), Fl(c, t[3])); var to = new Vector3(Fl(c, t[4]), Fl(c, t[5]), Fl(c, t[6]));
                    bool cull = In(c, t[7]) != 0; float max = Fl(c, t[8]);
                    c.Outputs.Add(g.TryRaycast(new Ray(o, to - o), out var hit, cull, max) ? "ray " + Hit(hit) : "ray none");
                    break;
                }
                case "rays":
                {
                    var r = new SplitMix(ulong.Parse(t[1])); int count = In(c, t[2]); bool cull = In(c, t[3]) != 0;
                    var center = g.Bounds.center; float radius = Mathf.Max(0.0001f, g.Bounds.extents.magnitude);
                    var f = new Fnv(); int hits = 0;
                    for (int k = 0; k < count; k++)
                    {
                        float ox = r.S(), oy = r.S(), oz = r.S(), tx = r.S(), ty = r.S(), tz = r.S();
                        var origin = center + new Vector3(ox, oy, oz) * (radius * 3);
                        var target = center + new Vector3(tx, ty, tz) * (radius * 0.5f);
                        if (g.TryRaycast(new Ray(origin, target - origin), out var hit, cull)) { hits++; f.Int(k); HashHit(f, hit); }
                    }
                    c.Outputs.Add("rays n=" + count + " hits=" + hits + " hash=" + f.Hex);
                    break;
                }
                case "dab":
                {
                    var cam = new Vector3(Fl(c, t[1]), Fl(c, t[2]), Fl(c, t[3])); var to = new Vector3(Fl(c, t[4]), Fl(c, t[5]), Fl(c, t[6]));
                    float radius = Fl(c, t[7]); int w = In(c, t[8]), h = In(c, t[9]); float hardness = Fl(c, t[10]);
                    bool ignore = t.Length > 11 && t[11] == "ignore";
                    if (!g.TryRaycast(new Ray(cam, to - cam), out var hit, true)) { c.Outputs.Add("dab nohit"); break; }
                    var d = g.BuildSurfaceDabs(hit, radius, w, h, cam, hardness, c.Budget, c.Cache, ignore);
                    var f = new Fnv(); float minimum = float.PositiveInfinity;
                    foreach (var p in d.Pixels) { f.Int(p.X); f.Int(p.Y); f.Float(p.Coverage); f.Int(p.TriangleIndex); f.V3(p.Position); if (p.Coverage < minimum) minimum = p.Coverage; }
                    c.Outputs.Add("dab tri=" + hit.TriangleIndex + " pixels=" + d.Pixels.Count + " hash=" + f.Hex + " cand=" + d.CandidatePixels + " rays=" + d.VisibilityRays +
                        " tests=" + d.RayTriangleTests + " visited=" + d.VisitedTriangles + " visits=" + d.RayNodeVisits + " mincov=" + H(minimum) + " clipped=" + (d.WasClipped ? 1 : 0) +
                        " why=" + Why(d.Diagnostic) + (c.Cache != null ? " cachehits=" + c.Cache.Hits + " cached=" + c.Cache.Count : ""));
                    if (Environment.GetEnvironmentVariable("SURFACE_DUMP") == c.Name)
                        foreach (var p in d.Pixels) Console.WriteLine(p.X + " " + p.Y + " " + H(p.Coverage) + " " + p.TriangleIndex + " " + H3(p.Position));
                    break;
                }
                case "closest":
                {
                    var p = new Vector3(Fl(c, t[1]), Fl(c, t[2]), Fl(c, t[3])); float max = Fl(c, t[4]);
                    var facing = new Vector3(Fl(c, t[5]), Fl(c, t[6]), Fl(c, t[7])); int visits = In(c, t[8]), material = In(c, t[9]);
                    bool found = g.TryFindClosestPoint(p, max, facing, visits, out var hit, out bool exceeded, material);
                    c.Outputs.Add(exceeded ? "closest exceeded" : found ? "closest " + Hit(hit) : "closest none");
                    break;
                }
                case "region":
                {
                    int tri = In(c, t[1]);
                    var kind = (SurfaceRegionKind)Enum.Parse(typeof(SurfaceRegionKind), t[2]);
                    var list = SurfaceRegions.Region(g, tri, kind); var f = new Fnv(); foreach (int i in list) f.Int(i);
                    c.Outputs.Add("region n=" + list.Count + " hash=" + f.Hex);
                    break;
                }
                default: throw new InvalidDataException("知らない命令: " + string.Join(" ", t));
            }
        }

        static string Why(string diagnostic)
        {
            if (string.IsNullOrEmpty(diagnostic)) return "-";
            if (diagnostic.Contains("snapshot changed")) return "snapshot";
            if (diagnostic.Contains("Invalid surface")) return "invalid";
            if (diagnostic.Contains("binding")) return "binding";
            if (diagnostic.Contains("triangle budget")) return "triangles";
            if (diagnostic.Contains("pixel budget")) return "pixels";
            if (diagnostic.Contains("visibility budget")) return "visibility";
            if (diagnostic.Contains("BVH work budget")) return "bvh";
            return "?" + diagnostic;
        }

        static string Hit(SurfaceHit h) =>
            "tri=" + h.TriangleIndex + " r=" + h.RendererIndex + " s=" + h.MaterialSlot + " m=" + h.Material + " dist=" + H(h.Distance) +
            " pos=" + H3(h.Position) + " n=" + H3(h.Normal) + " bary=" + H3(h.Barycentric) + " uv=" + H2(h.UV);

        static void HashHit(Fnv f, SurfaceHit h)
        {
            f.Int(h.TriangleIndex); f.Float(h.Distance); f.V3(h.Position); f.V3(h.Normal); f.V3(h.Barycentric); f.V2(h.UV);
        }

        const BindingFlags Private = BindingFlags.NonPublic | BindingFlags.Instance;

        static string Structure(SurfaceGeometry g)
        {
            var b = g.Bounds;
            float eps = (float)typeof(SurfaceGeometry).GetField("visibilityEpsilon", Private).GetValue(g);
            var neighbors = (IReadOnlyList<int[]>)typeof(SurfaceGeometry).GetProperty("Neighbors", Private).GetValue(g);
            var adj = new Fnv();
            foreach (var list in neighbors) { adj.Int(list.Length); foreach (int n in list) adj.Int(n); }
            var nodes = (IList)typeof(SurfaceGeometry).GetField("nodes", Private).GetValue(g);
            var indices = (int[])typeof(SurfaceGeometry).GetField("indices", Private).GetValue(g);
            var bvh = new Fnv();
            foreach (var node in nodes)
            {
                var type = node.GetType();
                var nb = (Bounds)type.GetField("Bounds").GetValue(node);
                int count = (int)type.GetField("Count").GetValue(node);
                bvh.V3(nb.center); bvh.V3(nb.extents);
                bvh.Int(count == 0 ? (int)type.GetField("Left").GetValue(node) : -1);
                bvh.Int(count == 0 ? (int)type.GetField("Right").GetValue(node) : -1);
                bvh.Int((int)type.GetField("Start").GetValue(node)); bvh.Int(count);
            }
            foreach (int i in indices) bvh.Int(i);
            return "structure tris=" + g.TriangleCount + " nonmanifold=" + g.NonManifoldEdgeCount + " center=" + H3(b.center) + " extents=" + H3(b.extents) +
                " eps=" + H(eps) + " adj=" + adj.Hex + " nodes=" + nodes.Count + " bvh=" + bvh.Hex;
        }

        // ───────── 速さ ─────────

        static void Bench(int repeat)
        {
            // 共通ベンチから両言語の並列数を揃える。未指定なら従来どおり。
            string threads = Environment.GetEnvironmentVariable("BENCH_THREADS");
            if (!string.IsNullOrEmpty(threads)) Yozolab.YoluPainter.Core.CoreParallelism.MaxDegreeOfParallelism = int.Parse(threads);
            var triangles = new List<SurfaceTriangle>();
            AddSphere(triangles, 76, 0.5f, "012345", 0, 0, 0);
            var build = new List<double>(); var adjacency = new List<double>(); var bvhTimes = new List<double>(); SurfaceGeometry g = null;
            for (int i = 0; i < repeat; i++)
            {
                var clock = Stopwatch.StartNew(); g = new SurfaceGeometry(triangles); build.Add(clock.Elapsed.TotalMilliseconds);
                adjacency.Add((double)typeof(SurfaceGeometry).GetProperty("AdjacencyMilliseconds", Private).GetValue(g));
                bvhTimes.Add((double)typeof(SurfaceGeometry).GetProperty("BvhMilliseconds", Private).GetValue(g));
            }
            Console.WriteLine("C#（Mono）球 " + triangles.Count + " 三角形: 組み立て " + Median(build).ToString("F2") + " ms（隣り合わせ " + Median(adjacency).ToString("F2") + " ms・BVH " + Median(bvhTimes).ToString("F2") + " ms）、" + repeat + " 回の中央値");
            var r = new SplitMix(7); var center = g.Bounds.center; float radius = g.Bounds.extents.magnitude;
            var rays = new List<Ray>();
            for (int k = 0; k < 20000; k++)
            {
                float ox = r.S(), oy = r.S(), oz = r.S(), tx = r.S(), ty = r.S(), tz = r.S();
                var origin = center + new Vector3(ox, oy, oz) * (radius * 3); var target = center + new Vector3(tx, ty, tz) * (radius * 0.5f);
                rays.Add(new Ray(origin, target - origin));
            }
            var per = new List<double>(); int hits = 0;
            for (int i = 0; i < repeat; i++)
            {
                var clock = Stopwatch.StartNew(); hits = 0;
                foreach (var ray in rays) if (g.TryRaycast(ray, out _, true)) hits++;
                per.Add(clock.Elapsed.TotalMilliseconds * 1000 / rays.Count);
            }
            Console.WriteLine("C#（Mono）レイ 1 本 " + Median(per).ToString("F3") + " µs（" + rays.Count + " 本・当たり " + hits + "）");
            var cam = new Vector3(0.3f, 0.6f, -2f);
            g.TryRaycast(new Ray(cam, -cam), out var hit, true);
            foreach (float rad in new[] { 0.05f, 0.15f })
            {
                var dab = new List<double>(); int pixels = 0;
                for (int i = 0; i < repeat; i++)
                {
                    var clock = Stopwatch.StartNew(); var d = g.BuildSurfaceDabs(hit, rad, 2048, 2048, cam, 0.8f); dab.Add(clock.Elapsed.TotalMilliseconds); pixels = d.Pixels.Count;
                }
                Console.WriteLine("C#（Mono）ダブ 2048² 半径 " + rad + ": " + Median(dab).ToString("F2") + " ms（" + pixels + " 画素）");
            }
        }

        static double Median(List<double> v) { var s = v.OrderBy(x => x).ToList(); return s[s.Count / 2]; }
    }
}
