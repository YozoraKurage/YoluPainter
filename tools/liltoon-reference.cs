using System.IO;
using System.Globalization;
using UnityEngine.Rendering;
// lilToon の再現を比べる正解の絵を、Unity の本物の lilToon で撮る（常駐の Unity の `unity-do.sh run` に渡すメソッドの本体）。
//
// 使い方: `crates/yolu-app/tests/gui_view3d/liltoon_reference.rs` の export_and_render が書いたフォルダ（scenes.txt・*.mesh.txt・*.rgba）の
// 場所を、下の __DIR__ に置き換えて渡す。
//   sed "s#__DIR__#$DIR#" tools/liltoon-reference.cs > /tmp/snippet.cs && .devcontainer/unity/unity-do.sh run /tmp/snippet.cs
// 場面ごとに unity_<名前>.png を同じフォルダに書く。Live Link が送るマテリアルの値（Unity のパッケージの LiveLinkMaterialValues が
// 読むものと同じ。プロパティ・キーワード・描いた絵を見せるプロパティ・描いていないスロットの絵）も link_<名前>.txt と
// link_<名前>_<スロット>.rgba に書く（`crates/yolu-app/tests/gui_view3d/liltoon_reference.rs` の compare_through_live_link が、その値だけで 3D ビューを描いて比べる）。
//
// 撮る間だけ、プロジェクトの色空間をリニアにし（VRChat と同じ。lilToon の式はリニアで比べる。替わったことを確かめてから撮る。
// テストプロジェクトはガンマのままにしておく）、lilToon のシェーダー設定の機能を
// 全部入れる（テクスチャを読む機能は、使うマテリアルがあるときに lilToon が自分で入れるのと同じ）。終わったら両方を元に戻す。
// lilToon の設定のファイル（ProjectSettings/lilToonSetting.json）が無いプロジェクトでは何も変えずに断る（撮る間に lilToon が
// ファイルを作り、元の「ファイルが無い」状態へは戻せないため）。
// 光は 1 つの平行光（白・強さ 0.769。ビルトインの既定の「リニアの強さを使わない」で _LightColor0 = sRGB→リニア(0.769)）、
// 環境光は 3D ビューの空の SH（scenes.txt の sh。9 つの係数を Unity の SphericalHarmonicsL2 へ最小二乗で当てはめる）、影は落とさない。
// cube の行は一様な色のキューブマップをそのスロットに入れる（環境光の反射の場面: マテリアルの反射の差し替えで一様な環境を映す）。
var dir = "__DIR__";
var log = new System.Text.StringBuilder();
var inv = CultureInfo.InvariantCulture;
float F(string s) => float.Parse(s, inv);

// ── 色空間と lilToon の設定を、撮る間だけ変える ──
var oldSpace = PlayerSettings.colorSpace;
var settingPath = "ProjectSettings/lilToonSetting.json";
var oldSetting = File.Exists(settingPath) ? File.ReadAllText(settingPath) : null;
var settingType = System.Type.GetType("lilToonSetting, lilToon.Editor");
if (settingType != null && oldSetting == null)
    return settingPath + " が無いので撮らない（元に戻せない）";
System.Reflection.MethodInfo apply = null;
if (settingType != null)
{
    var setting = ScriptableObject.CreateInstance(settingType);
    var load = settingType.GetMethod("LoadShaderSetting", System.Reflection.BindingFlags.Static | System.Reflection.BindingFlags.NonPublic | System.Reflection.BindingFlags.Public);
    var on = settingType.GetMethod("TurnOnAllShaderSetting", System.Reflection.BindingFlags.Static | System.Reflection.BindingFlags.Public);
    apply = settingType.GetMethods(System.Reflection.BindingFlags.Static | System.Reflection.BindingFlags.NonPublic | System.Reflection.BindingFlags.Public)
        .First(m => m.Name == "ApplyShaderSetting" && m.GetParameters().Length == 2);
    var args = new object[] { setting };
    load.Invoke(null, args);
    on.Invoke(null, args);
    // テクスチャの数が限られる API（エディタの OpenGL）では TurnOnAll がテクスチャの機能を入れないので、場面が使うテクスチャ
    // （scenes.txt の texture）と、テクスチャを割り当てずに既定のテクスチャで読ませる機能（feature）だけを入れる（全部を入れると
    // シェーダーのテクスチャの数の上限を超えて、シェーダーが壊れる）
    var wanted = new System.Collections.Generic.HashSet<string>();
    foreach (var line in File.ReadAllLines(Path.Combine(dir, "scenes.txt")))
    {
        var q = line.Split(' ');
        if (q[0] == "texture") wanted.Add(q[1].TrimStart('_'));
        if (q[0] == "feature") wanted.Add(q[1]);
    }
    foreach (var tex in wanted)
    {
        var field = settingType.GetField("LIL_FEATURE_" + tex);
        // メインのテクスチャは機能の入切が無い（いつも読む）
        if (field != null) field.SetValue(args[0], true);
        else if (tex != "MainTex") log.AppendLine("lilToon の機能が無い: " + tex);
    }
    apply.Invoke(null, new object[] { args[0], null });
    log.AppendLine("lilToon の機能を全部入れた");
}
else log.AppendLine("lilToonSetting が見つからない");
PlayerSettings.colorSpace = ColorSpace.Linear;

var ambientMode = RenderSettings.ambientMode;
var ambientProbe = RenderSettings.ambientProbe;
var made = new System.Collections.Generic.List<Object>();
// 開いている場面のほかの光は、撮る間だけ切る
var others = Object.FindObjectsOfType<Light>().Where(l => l.enabled).ToList();
foreach (var l in others) l.enabled = false;
try
{
    // 色空間はその場で替わる（QualitySettings.activeColorSpace。2026-10-05 に Unity 2022.3 で確かめた）。替わらなければ撮らない
    if (QualitySettings.activeColorSpace != ColorSpace.Linear)
        throw new System.InvalidOperationException("リニアの色空間にならない: " + QualitySettings.activeColorSpace);
    log.AppendLine("色空間: " + QualitySettings.activeColorSpace);
    var lines = File.ReadAllLines(Path.Combine(dir, "scenes.txt"));
    int at = 0;
    while (at < lines.Length)
    {
        var head = lines[at++].Split(' ');
        if (head[0] != "scene") continue;
        string name = head[1];
        int width = 0, height = 0;
        Vector3 camPos = Vector3.zero; Quaternion camRot = Quaternion.identity; float fov = 30;
        Vector3 toLight = Vector3.up; float intensity = 1;
        float[] sh = new float[27];
        Color background = Color.black;
        string meshFile = null, shaderName = "lilToon";
        var floats = new System.Collections.Generic.List<(string, float)>();
        var colors = new System.Collections.Generic.List<(string, Color)>();
        var vectors = new System.Collections.Generic.List<(string, Vector4)>();
        var textures = new System.Collections.Generic.List<(string, string, int, int, bool)>();
        var cubes = new System.Collections.Generic.List<(string, Color)>();
        while (at < lines.Length)
        {
            var p = lines[at++].Split(' ');
            if (p[0] == "end") break;
            switch (p[0])
            {
                case "size": width = int.Parse(p[1]); height = int.Parse(p[2]); break;
                case "camera": camPos = new Vector3(F(p[1]), F(p[2]), F(p[3])); camRot = new Quaternion(F(p[4]), F(p[5]), F(p[6]), F(p[7])); fov = F(p[8]); break;
                case "light": toLight = new Vector3(F(p[1]), F(p[2]), F(p[3])); intensity = F(p[4]); break;
                case "sh": for (int i = 0; i < 27; i++) sh[i] = F(p[1 + i]); break;
                case "background": background = new Color(F(p[1]), F(p[2]), F(p[3]), 0); break;
                case "mesh": meshFile = p[1]; break;
                case "shader": shaderName = string.Join(" ", p.Skip(1)); break;
                case "float": floats.Add((p[1], F(p[2]))); break;
                case "color": colors.Add((p[1], new Color(F(p[2]), F(p[3]), F(p[4]), F(p[5])))); break;
                case "vector": vectors.Add((p[1], new Vector4(F(p[2]), F(p[3]), F(p[4]), F(p[5])))); break;
                case "texture": textures.Add((p[1], p[2], int.Parse(p[3]), int.Parse(p[4]), p[5] == "1")); break;
                case "cube": cubes.Add((p[1], new Color(F(p[2]), F(p[3]), F(p[4]), 1))); break;
            }
        }

        // 環境光: 3D ビューの SH（evaluate_sh の係数）を、Unity の SphericalHarmonicsL2 の 9 つの基底に最小二乗で当てはめる
        var dirs = new System.Collections.Generic.List<Vector3>();
        for (int x = -1; x <= 1; x++) for (int y = -1; y <= 1; y++) for (int z = -1; z <= 1; z++)
            if (x != 0 || y != 0 || z != 0) dirs.Add(new Vector3(x, y, z).normalized);
        var dirArray = dirs.ToArray();
        int n = dirArray.Length;
        var basis = new double[n, 9];
        for (int i = 0; i < 9; i++)
        {
            var unit = new SphericalHarmonicsL2();
            unit[0, i] = 1;
            var results = new Color[n];
            unit.Evaluate(dirArray, results);
            for (int j = 0; j < n; j++) basis[j, i] = results[j].r;
        }
        float Ours(int c, Vector3 d)
        {
            float[] b = { 1, d.y, d.z, d.x, d.x * d.y, d.y * d.z, 3 * d.z * d.z - 1, d.x * d.z, d.x * d.x - d.y * d.y };
            float s = 0;
            for (int k = 0; k < 9; k++) s += sh[k * 3 + c] * b[k];
            return s;
        }
        var probe = new SphericalHarmonicsL2();
        for (int c = 0; c < 3; c++)
        {
            var ata = new double[9, 10];
            for (int j = 0; j < n; j++)
            {
                double f = Ours(c, dirArray[j]);
                for (int a = 0; a < 9; a++)
                {
                    for (int b = 0; b < 9; b++) ata[a, b] += basis[j, a] * basis[j, b];
                    ata[a, 9] += basis[j, a] * f;
                }
            }
            for (int col = 0; col < 9; col++)
            {
                int piv = col;
                for (int r = col + 1; r < 9; r++) if (System.Math.Abs(ata[r, col]) > System.Math.Abs(ata[piv, col])) piv = r;
                for (int k = 0; k < 10; k++) { var t = ata[col, k]; ata[col, k] = ata[piv, k]; ata[piv, k] = t; }
                for (int r = 0; r < 9; r++)
                {
                    if (r == col) continue;
                    double fct = ata[r, col] / ata[col, col];
                    for (int k = col; k < 10; k++) ata[r, k] -= fct * ata[col, k];
                }
            }
            for (int i = 0; i < 9; i++) probe[c, i] = (float)(ata[i, 9] / ata[i, i]);
        }
        RenderSettings.ambientMode = AmbientMode.Custom;
        RenderSettings.ambientProbe = probe;

        // メッシュ
        var meshes = new System.Collections.Generic.List<Mesh>();
        {
            var ml = File.ReadAllLines(Path.Combine(dir, meshFile));
            int k = 0;
            while (k < ml.Length)
            {
                var p = ml[k].Split(' ');
                if (p[0] != "mesh") { k++; continue; }
                int count = int.Parse(p[1]); k++;
                var v = new Vector3[count]; var nn = new Vector3[count]; var uv = new Vector2[count];
                for (int i = 0; i < count; i++, k++)
                {
                    var q = ml[k].Split(' ');
                    v[i] = new Vector3(F(q[1]), F(q[2]), F(q[3]));
                    nn[i] = new Vector3(F(q[4]), F(q[5]), F(q[6]));
                    uv[i] = new Vector2(F(q[7]), F(q[8]));
                }
                var subs = new System.Collections.Generic.List<int[]>();
                while (k < ml.Length && ml[k].StartsWith("sub "))
                {
                    int ic = int.Parse(ml[k].Split(' ')[1]); k++;
                    var idx = new int[ic];
                    for (int t = 0; t < ic / 3; t++, k++)
                    {
                        var q = ml[k].Split(' ');
                        idx[t * 3] = int.Parse(q[1]); idx[t * 3 + 1] = int.Parse(q[2]); idx[t * 3 + 2] = int.Parse(q[3]);
                    }
                    subs.Add(idx);
                }
                var mesh = new Mesh { indexFormat = IndexFormat.UInt32 };
                mesh.vertices = v; mesh.normals = nn; mesh.uv = uv;
                mesh.subMeshCount = subs.Count;
                for (int s = 0; s < subs.Count; s++) mesh.SetTriangles(subs[s], s);
                mesh.RecalculateTangents();
                mesh.RecalculateBounds();
                meshes.Add(mesh);
                made.Add(mesh);
            }
        }

        // マテリアル
        var shader = Shader.Find(shaderName);
        if (shader == null) { log.AppendLine(name + ": シェーダーが無い " + shaderName); continue; }
        var mat = new Material(shader);
        made.Add(mat);
        foreach (var (k, v) in floats) mat.SetFloat(k, v);
        foreach (var (k, c) in colors) mat.SetColor(k, c);
        foreach (var (k, v) in vectors) mat.SetVector(k, v);
        foreach (var (slot, file, w, h, srgb) in textures)
        {
            var tex = new Texture2D(w, h, TextureFormat.RGBA32, true, !srgb);
            var raw = File.ReadAllBytes(Path.Combine(dir, file));
            var texels = new Color32[w * h];
            for (int i = 0; i < texels.Length; i++) texels[i] = new Color32(raw[i * 4], raw[i * 4 + 1], raw[i * 4 + 2], raw[i * 4 + 3]);
            tex.SetPixels32(texels);
            tex.Apply(true);
            tex.wrapMode = TextureWrapMode.Repeat;
            tex.filterMode = FilterMode.Trilinear;
            made.Add(tex);
            mat.SetTexture(slot, tex);
        }
        // 一様な色のキューブマップ（リニアの半精度。環境光の反射を一様な環境で比べる場面の、反射の差し替え）
        foreach (var (slot, c) in cubes)
        {
            var cube = new Cubemap(4, TextureFormat.RGBAHalf, false);
            var face = Enumerable.Repeat(c, 16).ToArray();
            foreach (CubemapFace f in new[] { CubemapFace.PositiveX, CubemapFace.NegativeX, CubemapFace.PositiveY, CubemapFace.NegativeY, CubemapFace.PositiveZ, CubemapFace.NegativeZ })
                cube.SetPixels(face, f);
            cube.Apply(false);
            made.Add(cube);
            mat.SetTexture(slot, cube);
        }

        // Live Link が送る値（読むだけ。マテリアルは変えない）
        {
            var binding = Yozolab.YoluPainter.Editor.Preview.PreviewMaterialBindings.Resolve(mat);
            var link = new System.Text.StringBuilder();
            if (!Yozolab.YoluPainter.Editor.LiveLink.LiveLinkMaterialValues.IsVerifiedLilToon(binding))
                link.AppendLine("none");
            else
            {
                var shown = binding.Channels.Where(c => c.Keywords.All(k => mat.IsKeywordEnabled(k))).ToList();
                // Live Link と同じく、スタンドアロンが描いた絵で見せるのは Color の流し込み先だけ
                var routed = shown.Where(c => c.Channel == Yozolab.YoluPainter.Core.PaintChannel.Color).Select(c => c.Property).ToList();
                var snap = Yozolab.YoluPainter.Editor.LiveLink.LiveLinkMaterialValues.Read(mat, binding, routed);
                link.AppendLine("shader " + snap.Shader);
                link.AppendLine("source " + snap.Source);
                foreach (var c in shown) link.AppendLine("route " + (int)c.Channel + " " + c.Property);
                foreach (var pr in snap.Properties)
                    link.AppendLine(string.Format(inv, "prop {0} {1} {2:R} {3:R} {4:R} {5:R}", pr.Type, pr.Name, pr.Value.x, pr.Value.y, pr.Value.z, pr.Value.w));
                foreach (var k in snap.Keywords) link.AppendLine("keyword " + k);
                foreach (var sl in snap.Slots)
                {
                    if (sl.Texture == null) { link.AppendLine("slot " + sl.Name + " empty"); continue; }
                    var size = Yozolab.YoluPainter.Editor.LiveLink.LiveLinkMaterialValues.SendSize(sl.Texture);
                    var pixels = Yozolab.YoluPainter.Editor.LiveLink.LiveLinkMaterialValues.ReadPixels(sl.Texture, size.x, size.y, sl.Srgb);
                    if (pixels == null) { link.AppendLine("slot " + sl.Name + " unreadable"); continue; }
                    var file = "link_" + name + "_" + sl.Name + ".rgba";
                    File.WriteAllBytes(Path.Combine(dir, file), pixels);
                    link.AppendLine("slot " + sl.Name + " " + size.x + " " + size.y + " " + (sl.Srgb ? 1 : 0) + " " + file);
                }
            }
            File.WriteAllText(Path.Combine(dir, "link_" + name + ".txt"), link.ToString());
        }

        // 物・光・カメラ
        var roots = new System.Collections.Generic.List<GameObject>();
        foreach (var mesh in meshes)
        {
            var go = new GameObject("lilToon の比べ");
            go.AddComponent<MeshFilter>().sharedMesh = mesh;
            var mr = go.AddComponent<MeshRenderer>();
            var mats = new Material[mesh.subMeshCount];
            for (int s = 0; s < mats.Length; s++) mats[s] = mat;
            mr.sharedMaterials = mats;
            mr.shadowCastingMode = ShadowCastingMode.Off;
            mr.receiveShadows = false;
            mr.lightProbeUsage = LightProbeUsage.Off;
            mr.reflectionProbeUsage = ReflectionProbeUsage.Off;
            roots.Add(go);
        }
        var lightGo = new GameObject("光");
        var light = lightGo.AddComponent<Light>();
        light.type = LightType.Directional;
        light.color = Color.white;
        light.intensity = intensity;
        light.shadows = LightShadows.None;
        lightGo.transform.rotation = Quaternion.LookRotation(-toLight);
        var camGo = new GameObject("カメラ");
        var cam = camGo.AddComponent<Camera>();
        cam.transform.SetPositionAndRotation(camPos, camRot);
        cam.fieldOfView = fov;
        cam.nearClipPlane = 0.01f;
        cam.farClipPlane = 100f;
        cam.clearFlags = CameraClearFlags.SolidColor;
        cam.backgroundColor = background;
        cam.allowHDR = false;
        cam.allowMSAA = false;
        var rt = new RenderTexture(width, height, 24, RenderTextureFormat.ARGB32, RenderTextureReadWrite.sRGB);
        rt.antiAliasing = 1;
        cam.targetTexture = rt;
        cam.Render();
        var prev = RenderTexture.active;
        RenderTexture.active = rt;
        var shot = new Texture2D(width, height, TextureFormat.RGBA32, false, false);
        shot.ReadPixels(new Rect(0, 0, width, height), 0, 0);
        shot.Apply();
        RenderTexture.active = prev;
        // EncodeToPNG はテクスチャの行（下から）を絵の向きで書く。アルファは捨てる（比べるのは色だけ）
        var px = shot.GetPixels32();
        for (int i = 0; i < px.Length; i++) px[i].a = 255;
        var outTex = new Texture2D(width, height, TextureFormat.RGBA32, false, false);
        outTex.SetPixels32(px);
        outTex.Apply();
        File.WriteAllBytes(Path.Combine(dir, "unity_" + name + ".png"), outTex.EncodeToPNG());
        cam.targetTexture = null;
        Object.DestroyImmediate(rt);
        Object.DestroyImmediate(shot);
        Object.DestroyImmediate(outTex);
        Object.DestroyImmediate(camGo);
        Object.DestroyImmediate(lightGo);
        foreach (var go in roots) Object.DestroyImmediate(go);
        log.AppendLine(name + ": " + width + "x" + height);
    }
}
finally
{
    foreach (var l in others) if (l != null) l.enabled = true;
    foreach (var o in made) if (o != null) Object.DestroyImmediate(o);
    RenderSettings.ambientMode = ambientMode;
    RenderSettings.ambientProbe = ambientProbe;
    PlayerSettings.colorSpace = oldSpace;
    if (oldSetting != null && settingType != null)
    {
        File.WriteAllText(settingPath, oldSetting);
        var setting = ScriptableObject.CreateInstance(settingType);
        var load = settingType.GetMethod("LoadShaderSetting", System.Reflection.BindingFlags.Static | System.Reflection.BindingFlags.NonPublic | System.Reflection.BindingFlags.Public);
        var args = new object[] { setting };
        load.Invoke(null, args);
        apply.Invoke(null, new object[] { args[0], null });
        log.AppendLine("lilToon の設定を戻した");
    }
}
return log.ToString();
