using System;
using System.Collections.Generic;
using System.IO;
using System.Linq;
using System.Text;
using Yozolab.YoluPainter.Core;
using Yozolab.YoluPainter.Core.Persistence;

/// 復旧用の世代の置き場（`GenerationStore`）を、Unity 版の実際のコードで書き・読む。
/// - `--generation <出力>`: 世代を 2 つ（確定を 2 回。変わらない中身は共有）積んだ置き場を書く。Rust の `GenerationStore` が
///   Unity 版の出力を読めることの正解。
/// - `--generation-reads <置き場を並べたフォルダ> <出力.txt>`: Rust が書いた置き場（サブフォルダごと）を Unity 版の `Load` に読ませ、
///   読めたか・断られたか（例外の種類と文）を 1 行ずつ記録する。Rust の名前の範囲が Unity 版より広いこと
///   （`sets/<ID>/` の下の入れ子の名前を、Unity 版は断る）の記録。
static class GenerationFixture
{
    static readonly Guid SetId = new Guid(3001, 0x1234, 0x5678, new byte[] { 0x80, 1, 2, 3, 4, 5, 6, 7 });

    public static bool Run(string[] args)
    {
        if (args[0] == "--generation") { Write(args[1]); return true; }
        if (args[0] == "--generation-reads") { Reads(args[1], args[2]); return true; }
        return false;
    }

    static byte[] Native(string layerName, double opacity)
    {
        var doc = new PaintDocument(16, 16, 8, 1024 * 1024, new Guid(3002, 0x1234, 0x5678, new byte[] { 0x80, 1, 2, 3, 4, 5, 6, 7 }));
        var layer = doc.AddLayer(layerName, new Guid(3003, 0x1234, 0x5678, new byte[] { 0x80, 1, 2, 3, 4, 5, 6, 7 }));
        doc.SetLayerOpacity(layer.Id, opacity);
        return DocumentBinary.Write(doc);
    }

    static Dictionary<string, byte[]> Files(string layerName, double opacity, string title)
    {
        var shared = new byte[4096]; for (int i = 0; i < shared.Length; i++) shared[i] = (byte)(i % 7);
        return new Dictionary<string, byte[]>(StringComparer.Ordinal)
        {
            [YlpFormat.SetEntry(SetId, YlpArchive.NativeName)] = Native(layerName, opacity),
            [YlpFormat.SetEntry(SetId, SelectionBinary.EntryName)] = shared,
            ["resources/" + new string('a', 64) + ".png"] = new byte[] { 1, 2, 3, 4 },
            ["recovery.json"] = Encoding.UTF8.GetBytes("{\"title\":\"" + title + "\",\"projectPath\":\"\",\"projectToken\":\"\",\"unchanged\":false}"),
        };
    }

    static void Write(string output)
    {
        if (Directory.Exists(output)) Directory.Delete(output, true);
        var first = GenerationStore.Commit(output, Files("一つ目", .5, "first"), null, null, 3, shareContents: true);
        GenerationStore.Commit(output, Files("二つ目", .75, "second"), first.Token, null, 3, shareContents: true);
        var lockFile = Path.Combine(output, ".save.lock"); if (File.Exists(lockFile)) File.Delete(lockFile);
        Console.WriteLine("Unity 版の GenerationStore で世代を 2 つ書きました: " + output);
    }

    static void Reads(string roots, string output)
    {
        var lines = new List<string>();
        foreach (var dir in Directory.GetDirectories(roots).OrderBy(d => d, StringComparer.Ordinal))
        {
            var name = Path.GetFileName(dir);
            try
            {
                var loaded = GenerationStore.Load(dir);
                lines.Add(name + ": 読めた（" + loaded.Files.Count + " エントリ）: " + string.Join(", ", loaded.Files.Keys.OrderBy(k => k, StringComparer.Ordinal).Select(k => k.Contains("sets/") ? k.Substring(0, 5) + "<ID>" + k.Substring(41) : k)));
            }
            catch (Exception ex) { lines.Add(name + ": 断られた（" + ex.GetType().Name + "）: " + ex.Message); }
        }
        File.WriteAllText(output, string.Join("\n", lines) + "\n", new UTF8Encoding(false));
    }
}
