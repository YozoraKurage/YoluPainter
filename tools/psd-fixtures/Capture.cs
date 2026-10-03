using System;
using System.Collections.Generic;
using System.IO;
using System.Linq;
using System.Reflection;
using System.Security.Cryptography;
using System.Text;
using NUnit.Framework;
using Yozolab.YoluPainter.Core.Psd;

// 試験だけの採取口。製品コードや Unity の作業木は変更しない。
public static class Capture
{
    static BinaryWriter output;
    static HashSet<string> seen = new HashSet<string>();
    static int count, editable, rewrites, passed;
    static string current;
    static int nextGuid;
    public static Guid NextGuid()
    {
        using (var sha = SHA256.Create()) return new Guid(sha.ComputeHash(Encoding.ASCII.GetBytes("psd-fixture-" + (++nextGuid))).Take(16).ToArray());
    }
    static void Blob(byte[] b) { output.Write(b == null ? -1 : b.Length); if (b != null) output.Write(b); }
    static void Text(string s) { Blob(Encoding.UTF8.GetBytes(s)); }
    public static void Save(byte[] bytes, PsdLimits l, PsdReadResult r)
    {
        var limits = new long[] { l.MaxSourceBytes,l.MaxOutputBytes,l.MaxDimension,l.MaxCanvasPixels,l.MaxLayers,l.MaxDecodedBytes,l.MaxMetadataBytes,l.MaxNameCodeUnits,l.MaxDiagnostics,l.MaxGroupDepth };
        string key;
        using (var sha = SHA256.Create()) key = Convert.ToBase64String(sha.ComputeHash(bytes)) + ":" + string.Join(",", limits);
        if (!seen.Add(key)) return;
        byte[] rewrite = null;
        if (r.Mode == PsdCompatibilityMode.EditableRaster) { editable++; try { rewrite = PsdCodec.Write(r.Document, l); rewrites++; } catch (ArgumentException) { } }
        count++; Text(current); Blob(bytes); output.Write((int)r.Mode);
        foreach (long n in limits) output.Write(n);
        output.Write(r.HasOriginalBytes); Text(string.Join(",", r.Diagnostics.Select(x => x.Code))); Blob(rewrite);
    }
    public static int Main(string[] args)
    {
        using (output = new BinaryWriter(File.Create(args[0])))
        {
            foreach (var type in Assembly.GetExecutingAssembly().GetTypes().Where(t => t.Namespace == "Yozolab.YoluPainter.Tests").OrderBy(t => t.Name))
            foreach (var method in type.GetMethods().OrderBy(m => m.Name))
            {
                var cases = method.GetCustomAttributes(typeof(TestCaseAttribute), false).Cast<TestCaseAttribute>().Select(a => a.Arguments).ToList();
                if (method.GetCustomAttributes(typeof(TestAttribute), false).Length > 0) cases.Add(new object[0]);
                foreach (var parameters in cases)
                {
                    current = type.Name + "." + method.Name + "(" + string.Join(",", parameters.Select(x => x?.ToString())) + ")";
                    try { method.Invoke(Activator.CreateInstance(type), parameters); passed++; }
                    catch (Exception ex) { Console.Error.WriteLine(current + ": " + (ex.InnerException ?? ex)); return 1; }
                }
            }
        }
        var summary = $"C# 試験: {passed} 成功\n固有の入力・予算: {count} 件\n編集可能: {editable} 件\n書き戻しの正解: {rewrites} 件\n";
        File.WriteAllText(args[0] + ".summary", summary); Console.Write(summary); return 0;
    }
}
