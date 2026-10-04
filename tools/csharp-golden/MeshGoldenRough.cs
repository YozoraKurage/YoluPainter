// 不規則な座標・数千面のメッシュ、名前・接線・頂点法線・高ポリの組み合わせ、BVH の探索の向きと打ち切り、ID の色の並び、
// 条件の鍵、古さの判定を Unity 同梱の Mono で作る。入力は整数のハッシュと 1 文の演算（1 つずつ float に入れる）だけで作り、
// Rust と同じビット列になる。実データを使わない。
using System;
using System.Collections.Generic;
using System.Globalization;
using System.IO;
using System.Linq;
using System.Security.Cryptography;
using System.Text;
using Yozolab.YoluPainter.Core.MeshMaps;

static partial class MeshGolden
{
    static uint Mix(uint x) { x ^= x >> 16; x *= 0x7feb352d; x ^= x >> 15; x *= 0x846ca68b; x ^= x >> 16; return x; }
    static float Unit(uint h) => (h >> 8) * (1f / 16777216f);
    static float Signed(uint h) { float d = Unit(h); d = d * 2f; d = d - 1f; return d; }

    sealed class Gen { public float[] Corners, Uvs; public int[] Slots; }

    static void Lattice(int x, int y, int z, int n, float amp, uint seed, float[] p, int o)
    {
        float s = amp * Signed(Mix((uint)((x * 257 + y) * 257 + z) * 0x9e3779b1u ^ seed));
        s = 1f + s;
        float cx = x * 2f; cx = cx / n; cx = cx - 1f;
        float cy = y * 2f; cy = cy / n; cy = cy - 1f;
        float cz = z * 2f; cz = cz / n; cz = cz - 1f;
        p[o] = cx * s; p[o + 1] = cy * s; p[o + 2] = cz * s;
    }

    static void Shrink(float[] q, int dim)
    {
        for (int a = 0; a < dim; a++)
        {
            float s = q[a] + q[dim + a]; float s2 = q[2 * dim + a] + q[3 * dim + a]; s = s + s2; s = s * 0.25f;
            for (int v = 0; v < 4; v++) { float d = q[v * dim + a] - s; d = d * 0.9f; q[v * dim + a] = s + d; }
        }
    }

    /// <summary>格子の立方体を 1 つ、頂点を半径方向にハッシュでずらす（隣り合う面は同じ頂点を共有する）。shatter なら四角形ごとに縮めて離す。</summary>
    static Gen Rough(int n, float amp, uint seed, bool shatter)
    {
        int tris = 12 * n * n;
        var g = new Gen { Corners = new float[tris * 9], Uvs = new float[tris * 6], Slots = new int[tris] };
        int[] order = { 0, 1, 2, 0, 2, 3 };
        int t = 0;
        var qp = new float[12]; var qu = new float[8];
        for (int face = 0; face < 6; face++)
        {
            int axis = face / 2, sign = face % 2 == 0 ? 1 : -1;
            for (int j = 0; j < n; j++)
                for (int i = 0; i < n; i++)
                {
                    for (int v = 0; v < 4; v++)
                    {
                        int u = v == 0 || v == 3 ? 0 : 1, w = v < 2 ? 0 : 1;
                        int iu = i + u, jw = j + w;
                        var c = new int[3];
                        c[axis] = sign > 0 ? n : 0; c[(axis + 1) % 3] = iu; c[(axis + 2) % 3] = sign > 0 ? jw : n - jw;
                        Lattice(c[0], c[1], c[2], n, amp, seed, qp, v * 3);
                        float fu = (float)iu / n; fu = fu + face % 3; fu = fu / 3f;
                        float fv = (float)jw / n; fv = fv + face / 3; fv = fv / 2f;
                        qu[v * 2] = fu; qu[v * 2 + 1] = fv;
                    }
                    if (shatter) { Shrink(qp, 3); Shrink(qu, 2); }
                    for (int k = 0; k < 2; k++)
                    {
                        for (int cc = 0; cc < 3; cc++)
                        {
                            int v = order[k * 3 + cc];
                            Array.Copy(qp, v * 3, g.Corners, t * 9 + cc * 3, 3);
                            Array.Copy(qu, v * 2, g.Uvs, t * 6 + cc * 2, 2);
                        }
                        g.Slots[t] = face; t++;
                    }
                }
        }
        return g;
    }

    /// <summary>b を scale 倍して a の外側に重ねる（スロットは offset ずらし、UV は同じなので重なる）。a の面から外へ出たレイは b の裏面に当たる。</summary>
    static Gen Nested(Gen a, Gen b, float scale, int slotOffset)
    {
        int na = a.Slots.Length, nb = b.Slots.Length;
        var g = new Gen { Corners = new float[(na + nb) * 9], Uvs = new float[(na + nb) * 6], Slots = new int[na + nb] };
        Array.Copy(a.Corners, g.Corners, na * 9); Array.Copy(a.Uvs, g.Uvs, na * 6); Array.Copy(a.Slots, g.Slots, na);
        for (int i = 0; i < nb * 9; i++) { float v = b.Corners[i] * scale; g.Corners[na * 9 + i] = v; }
        Array.Copy(b.Uvs, 0, g.Uvs, na * 6, nb * 6);
        for (int t = 0; t < nb; t++) g.Slots[na + t] = b.Slots[t] + slotOffset;
        return g;
    }

    /// <summary>三角形の 4 つに 1 つは接線なし（0）、残りは辺の向きの接線で w は 3 つに 1 つが負。</summary>
    static float[] RoughTangents(float[] corners)
    {
        int n = corners.Length / 9; var tg = new float[n * 12];
        for (int t = 0; t < n; t++)
        {
            if (t % 4 == 0) continue;
            for (int c = 0; c < 3; c++)
            {
                int a = t * 9 + c * 3, b = t * 9 + (c + 1) % 3 * 3, o = t * 12 + c * 4;
                for (int k = 0; k < 3; k++) tg[o + k] = corners[b + k] - corners[a + k];
                tg[o + 3] = (t + c) % 3 == 0 ? -1f : 1f;
            }
        }
        return tg;
    }

    /// <summary>角ごとの色。4 色の見本（黒・白・2 つの任意の色）から選ぶので、三角形の中で 2 つ同じ・3 つ違うが両方起きる。</summary>
    static float[] RoughColors(int n, uint seed)
    {
        var palette = new float[4 * 4];
        for (int c = 0; c < 4; c++)
        {
            for (int k = 0; k < 3; k++) palette[c * 4 + k] = c == 0 ? 0f : c == 1 ? 1f : Unit(Mix((uint)(c * 7 + k) ^ seed));
            palette[c * 4 + 3] = 1f;
        }
        var colors = new float[n * 12];
        for (int t = 0; t < n; t++)
            for (int c = 0; c < 3; c++)
            {
                int pick = (int)(Mix((uint)(t * 3 + c) + 17u ^ seed) % 4);
                Array.Copy(palette, pick * 4, colors, t * 12 + c * 4, 4);
            }
        return colors;
    }

    static MeshBakeInput RoughInput(Gen g, float[] normals = null, string normalSource = null, float[] tangents = null, float[] colors = null, int[] renderers = null,
        string[] names = null, string[] keys = null)
        => new MeshBakeInput(g.Corners, normals, g.Uvs, g.Slots, 0, normalSource, tangents, colors, renderers, names, keys);

    static MeshBakeSettings RoughSettings(int width, int height) => new MeshBakeSettings
    {
        Width = width, Height = height, TargetSlot = -1, Padding = 0, Antialiasing = 1, Maps = (MeshMapKind[])Enum.GetValues(typeof(MeshMapKind)),
        AoSamples = 16, ThicknessSamples = 16, CurvatureRadius = 0.05, AoMaxDistance = 0.1, ThicknessMaxDistance = 0.3,
    };

    sealed class RoughCase
    {
        public string Name; public MeshBakeInput Input, Reference; public MeshBakeSettings Settings;
    }

    static IEnumerable<RoughCase> RoughCases()
    {
        var a = Rough(12, 0.2f, 1, false);
        var self = RoughInput(a);
        var s1 = RoughSettings(64, 64); s1.Padding = 4;
        yield return new RoughCase { Name = "rough-self", Input = self, Settings = s1 };

        var s2 = RoughSettings(48, 40); s2.AoIgnoreBackfaces = true; s2.AoFalloff = MeshOcclusionFalloff.None; s2.Antialiasing = 2;
        yield return new RoughCase { Name = "rough-backfaces-none", Input = self, Settings = s2 };

        var nested = RoughInput(Nested(Rough(8, 0.1f, 21, false), Rough(8, 0.1f, 22, false), 1.2f, 6));
        var s2a = RoughSettings(64, 64); s2a.AoIgnoreBackfaces = true; s2a.AoMaxDistance = 0.12;
        yield return new RoughCase { Name = "nested-ignore-backfaces", Input = nested, Settings = s2a };
        var s2b = RoughSettings(48, 48); s2b.AoFalloff = MeshOcclusionFalloff.None; s2b.Antialiasing = 2; s2b.AoMaxDistance = 0.12; s2b.Padding = 2;
        yield return new RoughCase { Name = "nested-occlude-none", Input = nested, Settings = s2b };

        var low = Rough(10, 0.06f, 2, false); var high = Rough(16, 0.06f, 3, false);
        var highInput = RoughInput(high);
        var lowTangents = RoughInput(low, MeshBakeInput.ReconstructNormals(low.Corners, 60), "reconstructed-crease-60", RoughTangents(low.Corners));
        var s3 = RoughSettings(64, 64); s3.ReferenceAverageNormals = false; s3.ReferenceFrontal = 0.03; s3.ReferenceRear = 0.03; s3.Padding = 3;
        yield return new RoughCase { Name = "rough-ref-tangents-vertex-cage", Input = lowTangents, Reference = highInput, Settings = s3 };

        var lowPlain = RoughInput(low);
        var s4 = RoughSettings(56, 48); s4.IdSource = MeshIdSource.MeshPart; s4.ReferenceFrontal = 0.05; s4.ReferenceRear = 0.02; s4.Antialiasing = 3;
        yield return new RoughCase { Name = "rough-ref-fallback-tangents", Input = lowPlain, Reference = highInput, Settings = s4 };

        var lowRenderers = new int[low.Slots.Length]; for (int t = 0; t < lowRenderers.Length; t++) lowRenderers[t] = low.Slots[t] % 4;
        var highRenderers = new int[high.Slots.Length]; for (int t = 0; t < highRenderers.Length; t++) highRenderers[t] = high.Slots[t] % 4;
        var lowNamed = RoughInput(low, renderers: lowRenderers, names: new[] { "arm_low", "LEG_Low", "tail_low", "Head" });
        var highNamed = RoughInput(high, renderers: highRenderers, names: new[] { "ARM_HIGH", "leg", "Tail_high", "body_high" });
        var s5 = RoughSettings(64, 48); s5.ReferenceMatchByName = true; s5.IdSource = MeshIdSource.Mesh; s5.ReferenceFrontal = 0.04; s5.ReferenceRear = 0.04;
        yield return new RoughCase { Name = "rough-ref-names", Input = lowNamed, Reference = highNamed, Settings = s5 };

        var shattered = Rough(6, 0.05f, 4, true); var smoothHigh = Rough(10, 0.05f, 5, false);
        var lowKeys = new string[shattered.Slots.Length]; for (int t = 0; t < lowKeys.Length; t++) lowKeys[t] = (t / 2) % 5 == 0 ? null : "m" + (t / 2) % 40;
        var highKeys = new string[smoothHigh.Slots.Length]; for (int t = 0; t < highKeys.Length; t++) highKeys[t] = smoothHigh.Slots[t] < 3 ? null : "m" + t % 7;
        var lowShattered = RoughInput(shattered, keys: lowKeys); var highKeyed = RoughInput(smoothHigh, keys: highKeys);
        var s6 = RoughSettings(64, 64); s6.IdSource = MeshIdSource.MaterialAsset; s6.Padding = 2; s6.ReferenceFrontal = 0.08; s6.ReferenceRear = 0.08;
        yield return new RoughCase { Name = "shattered-material-asset-ref", Input = lowShattered, Reference = highKeyed, Settings = s6 };

        var lowS = RoughInput(shattered); var highS = RoughInput(smoothHigh);
        var s7 = RoughSettings(64, 64); s7.IdSource = MeshIdSource.MeshPart; s7.ReferenceFrontal = 0.08; s7.ReferenceRear = 0.08;
        yield return new RoughCase { Name = "shattered-mesh-part-ref", Input = lowS, Reference = highS, Settings = s7 };

        var s8 = RoughSettings(64, 64); s8.IdSource = MeshIdSource.UvIsland; s8.Padding = 3; s8.Antialiasing = 2; s8.ReferenceFrontal = 0.08; s8.ReferenceRear = 0.08;
        yield return new RoughCase { Name = "shattered-uv-island-ref", Input = lowS, Reference = highS, Settings = s8 };

        var lowColored = RoughInput(shattered, colors: RoughColors(shattered.Slots.Length, 11)); var highColored = RoughInput(smoothHigh, colors: RoughColors(smoothHigh.Slots.Length, 12));
        var s9 = RoughSettings(64, 64); s9.IdSource = MeshIdSource.VertexColor; s9.ReferenceFrontal = 0.08; s9.ReferenceRear = 0.08;
        yield return new RoughCase { Name = "shattered-vertex-color-ref", Input = lowColored, Reference = highColored, Settings = s9 };

        var parts = new IdPartIndex(lowS);
        var s10 = RoughSettings(64, 64); s10.IdSource = MeshIdSource.MeshPart; s10.TargetSlot = 1; s10.TargetSlots = new[] { 1, 4 }; s10.Occluders = MeshOccluders.TargetSlotOnly;
        s10.ManualIdColors = new IdColorAssignments(parts.Binding, new Dictionary<int, int> { { 40, 0x00ff80 }, { 150, 0x102030 } });
        yield return new RoughCase { Name = "shattered-manual-colors", Input = lowS, Settings = s10 };

        var big = RoughInput(Rough(24, 0.1f, 6, false));
        var s11 = RoughSettings(96, 70); s11.Padding = 6;
        yield return new RoughCase { Name = "rough-large", Input = big, Settings = s11 };

        var s12 = new MeshBakeSettings { Width = 256, Height = 256, TargetSlot = -1, Padding = 0, Maps = (MeshMapKind[])Enum.GetValues(typeof(MeshMapKind)), AoSamples = 8, ThicknessSamples = 8 };
        yield return new RoughCase { Name = "bench-256", Input = BenchmarkModel(), Settings = s12 };
    }

    static string Sha(byte[] bytes)
    {
        using (var sha = SHA256.Create()) return string.Concat(sha.ComputeHash(bytes).Select(b => b.ToString("x2")));
    }
    /// <summary>由来と圧縮を除いた中身の SHA-256（テクセルの由来の並び + 16 bit の値のリトルエンディアン）。</summary>
    static string PayloadSha(BakedMeshMap map)
    {
        var bytes = new byte[map.Coverage.Length + map.Data.Length * 2];
        Buffer.BlockCopy(map.Coverage, 0, bytes, 0, map.Coverage.Length);
        Buffer.BlockCopy(map.Data, 0, bytes, map.Coverage.Length, map.Data.Length * 2);
        return Sha(bytes);
    }

    static string Inv(long v) => v.ToString(CultureInfo.InvariantCulture);

    /// <summary>診断を、Rust の MeshBakeNote と同じ並び・同じ数値の短い記号にする（文言は言語ごとに違う）。</summary>
    static string NoteOf(string d)
    {
        int space = d.IndexOf(' ');
        string head = space > 0 ? d.Substring(0, space) : d;
        if (d.Contains(" texels are covered by more than one triangle")) return "overlap=" + head;
        if (d.Contains(" triangles in the target slot have no UV area")) return "zero_uv=" + head;
        if (d.StartsWith("No vertex normals were given")) return "normals=face";
        if (d.StartsWith("Vertex normals were reconstructed from the shape (")) return "normals=" + d.Substring(d.IndexOf('(') + 1, d.IndexOf(')') - d.IndexOf('(') - 1);
        if (d.Contains(" triangles had no tangents")) return "tangent_fallback=" + head;
        if (d.Contains(" edges are non-manifold or have flipped winding")) return "bad_edges=" + head;
        if (d.Contains(" samples found no high-poly surface within the frontal/rear distances"))
            return "missed=" + head + "/" + d.Split(' ')[2] + (d.Contains("(or no high-poly part with a matching name)") ? "/name" : "");
        if (d.StartsWith("Manual ID colours override the source on ")) return "manual_parts=" + d.Split(' ')[7];
        if (d.StartsWith("The model has no vertex colours")) return "no_vertex_colors";
        if (d.StartsWith("ID map: ")) return "id_parts=" + d.Split(' ')[2] + "/" + Between(d, ") (", ")") + "/sep=" + d.Substring(d.IndexOf("at least ") + 9).Split(' ')[0];
        if (d.StartsWith("No high-poly reference: the tangent-space normal is flat")) return "no_reference";
        return "unknown:" + d;
    }

    static string Between(string text, string start, string end)
    {
        int from = text.IndexOf(start) + start.Length;
        return text.Substring(from, text.IndexOf(end, from) - from);
    }

    static string ReportLine(string name, MeshBakeReport r)
    {
        var text = new StringBuilder("report " + name);
        void F(string key, long value) { text.Append(' ').Append(key).Append('=').Append(Inv(value)); }
        F("estimated", r.EstimatedBytes); F("receiving", r.ReceivingTriangles); F("zero_uv", r.ZeroUvAreaTriangles); F("degenerate", r.DegenerateTriangles);
        F("occluders", r.OccluderTriangles); F("reference", r.ReferenceTriangles); F("id_parts", r.IdParts); F("rays", r.Rays); F("projected", r.ProjectedSamples);
        F("missed", r.MissedSamples); F("covered", r.CoveredTexels); F("overlap", r.OverlapTexels); F("padded", r.PaddedTexels); F("empty", r.EmptyTexels);
        F("boundary", r.BoundaryEdges); F("non_manifold", r.NonManifoldEdges); F("inconsistent", r.InconsistentWindingEdges); F("segments", r.CurvatureSegments);
        text.Append(" notes=").Append(string.Join(",", r.Diagnostics.Select(NoteOf)));
        return text.ToString();
    }

    static void RoughCasesOutput(string output, string dump)
    {
        var lines = new List<string>();
        // Mono が float の式を float のまま計算することの確認（Rust の f32 と同じ前提）。double なら 1、float なら 0。
        float[] probe = { 16777216f, 1f, -16777216f };
        float sum = probe[0] + probe[1] + probe[2];
        lines.Add("probe float_sum=" + sum.ToString("R", CultureInfo.InvariantCulture));
        foreach (var c in RoughCases())
        {
            var result = MeshBaker.Bake(c.Input, c.Settings, new MeshBakeBudget { MaxDegreeOfParallelism = 1 }, reference: c.Reference);
            var withContext = c.Settings.WithIdContext(c.Input, c.Reference, c.Settings.ManualIdColors);
            foreach (var map in result.Maps)
            {
                byte[] bytes = MeshMapBinary.Write(map);
                lines.Add("map " + c.Name + " " + map.Kind + " " + Sha(bytes) + " " + PayloadSha(map));
                lines.Add("key " + c.Name + " " + map.Kind + " " + MeshBaker.ConditionKey(c.Input, withContext, map.Kind, c.Reference));
                if (dump != null) File.WriteAllBytes(Path.Combine(dump, c.Name + "-" + map.Kind + ".bin"), bytes);
            }
            lines.Add(ReportLine(c.Name, result.Report));
            if (c.Name == "rough-ref-tangents-vertex-cage" || c.Name == "rough-self")
                foreach (var map in result.Maps)
                    if (map.Kind == MeshMapKind.WorldNormal || map.Kind == MeshMapKind.AmbientOcclusion || map.Kind == MeshMapKind.Id)
                        CheckLines(lines, c, map);
        }
        File.WriteAllLines(Path.Combine(output, "rough.txt"), lines);
    }

    // ---- 古さの判定 ----
    static string ReasonCode(string r)
    {
        if (r.StartsWith("baked by mesh-map engine version")) return "engine";
        if (r.StartsWith("baked in space")) return "space_pose";
        if (r.StartsWith("baked without a high-poly reference")) return "reference_added";
        if (r.StartsWith("baked from a high-poly reference; none")) return "reference_removed";
        if (r.StartsWith("the high-poly reference or its projection")) return "reference_changed";
        if (r.StartsWith("baked at ")) return "size";
        if (r.StartsWith("the texture set's material is not in")) return "material_not_in_model";
        if (r.StartsWith("baked for material slot")) return "slots";
        if (r.StartsWith("baked from UV")) return "uv_channel";
        if (r.Contains(" texels of padding")) return "padding";
        if (r.Contains(" antialiasing, the settings ask")) return "antialiasing";
        if (r.StartsWith("bake settings changed")) return "settings";
        if (r.StartsWith("the model's shape changed")) return "shape_changed";
        if (r.StartsWith("the model changed since")) return "model_changed";
        return "unknown:" + r;
    }

    static void CheckLines(List<string> lines, RoughCase c, BakedMeshMap map)
    {
        var moved = RoughInput(Rough(10, 0.06f, 2, false)); var other = RoughInput(Rough(10, 0.06f, 2, true));
        var variants = new List<KeyValuePair<string, Action<MeshMapExpectation>>>();
        void V(string name, Action<MeshMapExpectation> change) => variants.Add(new KeyValuePair<string, Action<MeshMapExpectation>>(name, change));
        V("exact", e => { });
        V("no-model", e => { e.MeshHash = null; e.TopologyHash = null; });
        V("width", e => e.Width += 1);
        V("height", e => e.Height *= 2);
        V("slot", e => e.TargetSlot = 3);
        V("outside-model", e => e.TargetSlot = -2);
        V("uv1", e => e.UvChannel = 1);
        V("padding", e => e.Settings.Padding += 1);
        V("antialiasing", e => e.Settings.Antialiasing = e.Settings.Antialiasing == 1 ? 2 : 1);
        V("ao-samples", e => e.Settings.AoSamples = 32);
        V("curvature-radius", e => e.Settings.CurvatureRadius = 0.04);
        V("id-source", e => e.Settings.IdSource = MeshIdSource.MeshPart);
        V("moved-vertex", e => { e.MeshHash = moved.Hash; e.TopologyHash = moved.TopologyHash; });
        V("new-uv", e => { e.MeshHash = other.Hash; e.TopologyHash = other.TopologyHash; });
        V("reference-removed", e => e.ReferenceHash = null);
        V("reference-changed", e => e.ReferenceHash = "1234");
        V("reference-settings", e => e.Settings.ReferenceFrontal = 0.02);
        V("reference-no-settings", e => { e.Settings = null; e.ReferenceHash = "1234"; });
        V("no-settings", e => e.Settings = null);
        V("slot-list", e => { e.TargetSlot = 0; e.TargetSlots = new[] { 0, 1 }; });
        foreach (var v in variants)
        {
            var e = new MeshMapExpectation
            {
                MeshHash = c.Input.Hash, TopologyHash = c.Input.TopologyHash, ReferenceHash = c.Reference?.Hash, Width = c.Settings.Width, Height = c.Settings.Height,
                TargetSlot = c.Settings.TargetSlot, TargetSlots = c.Settings.TargetSlots, UvChannel = c.Input.UvChannel, Settings = c.Settings.WithIdContext(c.Input, c.Reference, c.Settings.ManualIdColors),
            };
            v.Value(e);
            var check = map.Provenance.Check(e);
            lines.Add("check " + c.Name + " " + map.Kind + " " + v.Key + " " + check.State + " " + (check.State == MeshMapState.Current || check.State == MeshMapState.Unverified ? "-" : string.Join(",", check.Reasons.Select(ReasonCode))));
        }
        var p = map.Provenance;
        var min = new[] { p.BoundsMin(0), p.BoundsMin(1), p.BoundsMin(2) }; var max = new[] { p.BoundsMax(0), p.BoundsMax(1), p.BoundsMax(2) };
        foreach (var variant in new[] { "engine", "space", "pose" })
        {
            var changed = new MeshMapProvenance(p.Kind, variant == "engine" ? 1 : p.EngineVersion, p.MeshHash, p.TopologyHash, p.UvChannel, p.Width, p.Height, p.TargetSlot, p.Padding,
                p.Antialiasing, p.SettingsKey, variant == "space" ? "Other" : p.Space, variant == "pose" ? "Animated" : p.Pose, p.Source, min, max, p.TargetSlots.ToArray());
            var e = new MeshMapExpectation
            {
                MeshHash = c.Input.Hash, TopologyHash = c.Input.TopologyHash, ReferenceHash = c.Reference?.Hash, Width = c.Settings.Width, Height = c.Settings.Height,
                TargetSlot = c.Settings.TargetSlot, TargetSlots = c.Settings.TargetSlots, UvChannel = c.Input.UvChannel, Settings = c.Settings.WithIdContext(c.Input, c.Reference, c.Settings.ManualIdColors),
            };
            var check = changed.Check(e);
            lines.Add("check " + c.Name + " " + map.Kind + " " + variant + " " + check.State + " " + (check.State == MeshMapState.Current || check.State == MeshMapState.Unverified ? "-" : string.Join(",", check.Reasons.Select(ReasonCode))));
        }
    }

    // ---- BVH: 探索の打ち切り・裏面・無視する三角形、不規則な座標 ----
    static readonly bool[][] BvhModesList =
    {
        new[] { false, false, false }, new[] { true, false, false }, new[] { false, true, false }, new[] { true, true, false },
        new[] { false, false, true }, new[] { false, true, true }, new[] { true, false, true },
    };

    static void BvhDump(string path, MeshBakeInput input, float[] corners, List<double[]> rays)
    {
        int n = corners.Length / 9; var indices = Enumerable.Range(0, n).ToArray();
        var bvh = new MeshRayBvh(corners, indices);
        bvh.Flatten(out var bounds, out var first, out var count, out var triangles, out var original);
        using (var w = new BinaryWriter(File.Create(path)))
        {
            w.Write(input.Hash); w.Write(input.TopologyHash); w.Write(bvh.NodeCount);
            for (int i = 0; i < bvh.NodeCount; i++) { for (int j = 0; j < 6; j++) w.Write(bounds[i * 6 + j]); w.Write(first[i]); w.Write(count[i]); }
            for (int i = 0; i < n; i++) { w.Write(original[i]); for (int j = 0; j < 10; j++) w.Write(triangles[i * 10 + j]); }
            int rayIndex = 0;
            foreach (var r in rays)
            {
                foreach (var mode in BvhModesList)
                {
                    int skip = mode[2] ? rayIndex % n : -1;
                    double distance = bvh.Trace(r[0], r[1], r[2], r[3], r[4], r[5], 12, skip, mode[0], mode[1], new int[128], new double[128], out int hit, out double u, out double v);
                    w.Write(distance); w.Write(hit); w.Write(u); w.Write(v);
                }
                rayIndex++;
            }
        }
    }

    /// <summary>97 面（3 つに 1 つは巻きを逆にして裏向き）で、上と下からのレイを 7 つの探索の設定で。</summary>
    static void BvhModes(string output)
    {
        const int n = 97;
        var corners = new float[n * 9]; var uvs = new float[n * 6]; var slots = new int[n];
        for (int t = 0; t < n; t++)
        {
            float x = (t * 37 % 101) / 8f, y = (t * 19 % 71) / 8f, z = (t * 7 % 29) / 16f;
            int k = t * 9; corners[k] = x; corners[k + 1] = y; corners[k + 2] = z;
            bool flipped = t % 3 == 0;
            corners[k + 3] = flipped ? x : x + 0.5f; corners[k + 4] = flipped ? y + 0.75f : y; corners[k + 5] = z;
            corners[k + 6] = flipped ? x + 0.5f : x; corners[k + 7] = flipped ? y : y + 0.75f; corners[k + 8] = z;
            uvs[t * 6 + 2] = 1; uvs[t * 6 + 5] = 1;
        }
        var input = new MeshBakeInput(corners, null, uvs, slots);
        var rays = new List<double[]>();
        for (int t = 0; t < n; t++)
        {
            int k = t * 9; double ox = corners[k] + 0.1, oy = corners[k + 1] + 0.1;
            rays.Add(new[] { ox, oy, 5, 0, 0, -1.0 });
            rays.Add(new[] { ox, oy, -5, 0, 0, 1.0 });
        }
        BvhDump(Path.Combine(output, "bvh-modes.bin"), input, corners, rays);
    }

    /// <summary>不規則な座標の 1728 面。三角形の重心から 3 方向（上から・下から・横から）。</summary>
    static void BvhRough(string output)
    {
        var g = Rough(12, 0.2f, 1, false); var input = RoughInput(g);
        var rays = new List<double[]>();
        int n = g.Corners.Length / 9;
        for (int t = 0; t < n; t += 7)
        {
            int k = t * 9;
            double cx = ((double)g.Corners[k] + g.Corners[k + 3] + g.Corners[k + 6]) / 3, cy = ((double)g.Corners[k + 1] + g.Corners[k + 4] + g.Corners[k + 7]) / 3,
                cz = ((double)g.Corners[k + 2] + g.Corners[k + 5] + g.Corners[k + 8]) / 3;
            rays.Add(new[] { cx, cy, 5, 0, 0, -1.0 });
            rays.Add(new[] { cx, cy, -5, 0, 0, 1.0 });
            rays.Add(new[] { -5, cy, cz, 1.0, 0, 0 });
        }
        BvhDump(Path.Combine(output, "bvh-rough.bin"), input, g.Corners, rays);
    }

    // ---- ID パレット ----
    static void IdPaletteOutput(string output)
    {
        var lines = new List<string>();
        foreach (int n in new[] { 0, 1, 2, 6, 7, 24, 25, 60, 61, 120, 121, 250, 1000, 4080, 4081, 100000, 1000000 })
        {
            var colors = IdPalette.Colors(n);
            var bytes = new byte[colors.Length * 4]; Buffer.BlockCopy(colors, 0, bytes, 0, bytes.Length);
            lines.Add("palette " + n + " " + IdPalette.Levels(n) + " " + IdPalette.MinimumSeparation(n) + " " + Sha(bytes));
        }
        lines.Add("colors 61 " + string.Join(",", IdPalette.Colors(61).Select(c => c.ToString("x6"))));
        File.WriteAllLines(Path.Combine(output, "id-palette.txt"), lines);
    }
}
