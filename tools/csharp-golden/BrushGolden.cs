// ブラシ形式の取り込みの正解: Unity 版の GimpBrushReader・PhotoshopBrushReader・PhotoshopPatternReader に、Rust の試験が組んだ
// 入力（手で組んだ事例と、壊した入力）を通し、取り込めたか・断ったか・取り込んだ設定の指紋を書く。
// 使い方: BrushGolden.exe <inputs.bin> <出力先>   （brushes.sh が呼ぶ。出力は cases.txt と fuzz.txt）
// 環境変数 BRUSH_GOLDEN_SHOW=記録の番号,... で、その記録の完全な指紋を標準出力へ出す（違いの調査用。番号は試験の失敗に出る）。
// inputs.bin: "YBRI" 版 1、続いて記録の並び [tag u8: 1 = 手で組んだ事例、0 = 壊した入力][kind u8: 0 gbr 1 gih 2 vbr 3 abr 4 pat]
//   [名前の長さ u8][名前][長さ u32 ビッグエンディアン][バイト列]。
// 名前は、Rust の読み手と同じ整え方（制御文字を除く・UTF-8 由来は U+FFFD の連続を 1 つに・前後の空白・128 文字まで）にしてから比べる。
using System;
using System.Collections.Generic;
using System.IO;
using System.Linq;
using System.Security.Cryptography;
using System.Text;
using Yozolab.YoluPainter.Core;
using Yozolab.YoluPainter.Core.Brushes;

static class BrushGolden
{
    static string Bits(double v) { return BitConverter.DoubleToInt64Bits(v).ToString("x16"); }
    static string Hex(byte[] b, int n) { var sb = new StringBuilder(); for (int i = 0; i < n; i++) sb.Append(b[i].ToString("x2")); return sb.ToString(); }
    static string Sha(byte[] b, int n) { using (var sha = SHA256.Create()) return Hex(sha.ComputeHash(b), n); }
    static string Flag(bool b) { return b ? "1" : "0"; }

    static byte[] TipBytes(BrushTip t)
    {
        var ms = new MemoryStream();
        ms.Write(BitConverter.GetBytes(t.Width), 0, 4); ms.Write(BitConverter.GetBytes(t.Height), 0, 4);
        var a = t.CopyAlpha(); ms.Write(a, 0, a.Length);
        return ms.ToArray();
    }
    static string TipDesc(BrushTip t) { return t == null ? "-" : t.Width + "x" + t.Height + ":" + Sha(TipBytes(t), 8); }
    static string TipsDesc(BrushTip[] ts)
    {
        if (ts == null) return "-";
        var ms = new MemoryStream();
        foreach (var t in ts) { var b = TipBytes(t); ms.Write(b, 0, b.Length); }
        return ts.Length + ":" + Sha(ms.ToArray(), 8);
    }

    // 名前の整え方（Rust の short_text と同じ。UTF-8 由来のときは U+FFFD の連続を 1 つにする）
    static string Clean(string s, bool collapse)
    {
        var sb = new StringBuilder();
        foreach (char c in s) if (!char.IsControl(c)) sb.Append(c);
        string t = sb.ToString();
        if (collapse) { var o = new StringBuilder(); foreach (char c in t) { if (c == '\uFFFD' && o.Length > 0 && o[o.Length - 1] == '\uFFFD') continue; o.Append(c); } t = o.ToString(); }
        t = t.Trim();
        var r = new StringBuilder(); int n = 0;
        for (int i = 0; i < t.Length && n < 128; i++)
        {
            r.Append(t[i]);
            if (char.IsHighSurrogate(t[i]) && i + 1 < t.Length && char.IsLowSurrogate(t[i + 1])) r.Append(t[++i]);
            n++;
        }
        return r.ToString();
    }
    static string Esc(string s) { var sb = new StringBuilder(); foreach (char c in s) sb.Append(((int)c).ToString("x4")); return sb.Length == 0 ? "-" : sb.ToString(); }
    // 名前が「素直」か（整えても変わらない）。素直でない名前は、Rust 側が空の名前の代わりの名前を使うことがあるので比べない
    static bool Plain(string raw) { return raw.Length > 0 && raw.Length <= 100 && raw == raw.Trim() && !raw.Any(c => char.IsControl(c) || c == '\uFFFD'); }

    static string Settings(BrushSettings s, string source, IReadOnlyList<string> warnings)
    {
        var d = s.Dual;
        string dual = d == null ? "-" : string.Join(",", new[] { Bits(d.Radius), Bits(d.Hardness), Bits(d.Spacing), Bits(d.Angle), Bits(d.Roundness), Bits(d.Scatter), d.Count.ToString(), ((int)d.Mode).ToString(), TipDesc(d.Tip) });
        var f = new List<string>
        {
            "src=" + source, "r=" + Bits(s.Radius), "h=" + Bits(s.Hardness), "sp=" + Bits(s.Spacing), "op=" + Bits(s.Opacity), "fl=" + Bits(s.Flow),
            "an=" + Bits(s.Angle), "ro=" + Bits(s.Roundness), "sj=" + Bits(s.SizeJitter), "aj=" + Bits(s.AngleJitter), "rj=" + Bits(s.RoundnessJitter),
            "oj=" + Bits(s.OpacityJitter), "fj=" + Bits(s.FlowJitter), "sc=" + Bits(s.Scatter), "ct=" + s.Count,
            "ps=" + Flag(s.PressureSize), "po=" + Flag(s.PressureOpacity), "pf=" + Flag(s.PressureFlow), "fd=" + Flag(s.FollowDirection),
            "fb=" + Bits(s.ForegroundBackgroundJitter), "hu=" + Bits(s.HueJitter), "sa=" + Bits(s.SaturationJitter), "br=" + Bits(s.BrightnessJitter), "pu=" + Bits(s.Purity), "pt=" + Flag(s.ColorPerTip),
            "fs=" + s.FadeSize, "fo=" + s.FadeOpacity, "ff=" + s.FadeFlow,
            "ti=" + Flag(s.TiltSize), "to=" + Flag(s.TiltOpacity), "tf=" + Flag(s.TiltFlow), "ta=" + Flag(s.TiltAngle),
            "tip=" + TipDesc(s.Tip), "tips=" + TipsDesc(s.Tips), "sel=" + (int)s.TipSelection,
            "tex=" + TipDesc(s.Texture) + (s.Texture == null ? "" : ",td=" + Bits(s.TextureDepth) + ",ts=" + Bits(s.TextureScale)),
            "dual=" + dual, "warn=" + warnings.Count,
        };
        return string.Join(";", f);
    }

    static IReadOnlyList<ImportedBrush> Read(int kind, byte[] data, out bool utf8Names)
    {
        utf8Names = kind <= 2;
        switch (kind)
        {
            case 0: return new[] { GimpBrushReader.ReadGbr(data, "F") };
            case 1: return new[] { GimpBrushReader.ReadGih(data, "F") };
            case 2:
            {
                int skip = data.Length >= 3 && data[0] == 0xEF && data[1] == 0xBB && data[2] == 0xBF ? 3 : 0;
                return new[] { GimpBrushReader.ReadVbr(Encoding.UTF8.GetString(data, skip, data.Length - skip), "F") };
            }
            case 3: return PhotoshopBrushReader.Read(data, "F");
            default: return PhotoshopPatternReader.ReadPatBrushes(data);
        }
    }

    // outcome: OK / ERR（BrushImportException）/ EXC（それ以外の例外。Rust は範囲に収めるか断るので比べない）
    static void Run(int kind, byte[] data, out string outcome, out string noname, out string names, out bool plain)
    {
        outcome = "ERR"; noname = ""; names = ""; plain = true;
        try
        {
            bool utf8;
            var brushes = Read(kind, data, out utf8);
            outcome = "OK";
            noname = string.Join("||", brushes.Select(b => Settings(b.Settings, b.Source, b.Warnings)));
            names = string.Join("||", brushes.Select(b => Esc(Clean(b.Name, utf8))));
            plain = brushes.All(b => Plain(b.Name));
        }
        catch (BrushImportException) { outcome = "ERR"; }
        catch (Exception) { outcome = "EXC"; }
    }

    static int Main(string[] args)
    {
        var bytes = File.ReadAllBytes(args[0]);
        if (Encoding.ASCII.GetString(bytes, 0, 4) != "YBRI" || bytes[4] != 1) { Console.Error.WriteLine("inputs.bin の形式が違う"); return 2; }
        int p = 5;
        var cases = new StringBuilder(); var fuzz = new StringBuilder();
        int count = 0, ok = 0, err = 0, exc = 0;
        while (p < bytes.Length)
        {
            int tag = bytes[p++], kind = bytes[p++], nameLength = bytes[p++];
            string label = Encoding.UTF8.GetString(bytes, p, nameLength); p += nameLength;
            int length = (bytes[p] << 24) | (bytes[p + 1] << 16) | (bytes[p + 2] << 8) | bytes[p + 3]; p += 4;
            var data = new byte[length]; Buffer.BlockCopy(bytes, p, data, 0, length); p += length;
            string outcome, noname, names; bool plain;
            Run(kind, data, out outcome, out noname, out names, out plain);
            var show = Environment.GetEnvironmentVariable("BRUSH_GOLDEN_SHOW");
            if (show != null && show.Split(',').Contains(count.ToString())) Console.WriteLine("#" + count + " " + outcome + " " + noname + " || 名前 " + names);
            count++; if (outcome == "OK") ok++; else if (outcome == "ERR") err++; else exc++;
            if (tag == 1) cases.Append(label).Append('\t').Append(outcome).Append('\t').Append(noname).Append('\t').Append(names).Append('\n');
            else if (outcome == "OK") fuzz.Append("O ").Append(Sha(Encoding.UTF8.GetBytes(noname), 3)).Append(' ').Append(Sha(Encoding.UTF8.GetBytes(names), 3)).Append(' ').Append(plain ? 'P' : 'Q').Append('\n');
            else fuzz.Append(outcome == "ERR" ? "E - - -" : "X - - -").Append('\n');
        }
        Directory.CreateDirectory(args[1]);
        File.WriteAllText(Path.Combine(args[1], "cases.txt"), cases.ToString(), new UTF8Encoding(false));
        File.WriteAllText(Path.Combine(args[1], "fuzz.txt"), fuzz.ToString(), new UTF8Encoding(false));
        Console.WriteLine("入力 " + count + " 件: 取り込めた " + ok + "、断った " + err + "、ほかの例外 " + exc);
        return 0;
    }
}
