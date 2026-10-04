// Unity のエディタ内でも Probe.Run(reader, writer) をそのまま呼べる。
using System;
using System.IO;
using System.Globalization;
public static class Probe
{
    public static void Main(string[] args) { if (args.Length > 0 && args[0] == "--mathf") RunMathF(Console.In, Console.Out); else Run(Console.In, Console.Out); }
    // MathF が無い Mono では黙って Math に代えず、未対応として止める。
    public static void RunMathF(TextReader input, TextWriter output)
    {
        var type = typeof(Math).Assembly.GetType("System.MathF");
        if (type == null) throw new NotSupportedException("このランタイムには MathF がありません");
        string line;
        while ((line = input.ReadLine()) != null)
        {
            var t = line.Split(' ');
            float x = (float)BitConverter.Int64BitsToDouble(unchecked((long)ulong.Parse(t[2], NumberStyles.HexNumber)));
            float y = (float)BitConverter.Int64BitsToDouble(unchecked((long)ulong.Parse(t[3], NumberStyles.HexNumber)));
            bool two = t[1] == "pow" || t[1] == "atan2";
            string name = char.ToUpperInvariant(t[1][0]) + t[1].Substring(1);
            var method = type.GetMethod(name, two ? new[] { typeof(float), typeof(float) } : new[] { typeof(float) });
            if (method == null) throw new NotSupportedException("MathF の関数がありません: " + name);
            float value = (float)method.Invoke(null, two ? new object[] { x,y } : new object[] { x });
            uint bits = BitConverter.ToUInt32(BitConverter.GetBytes(value), 0);
            output.WriteLine(line + " " + bits.ToString("x8"));
        }
    }
    public static void Run(TextReader input, TextWriter output)
    {
        string line;
        while ((line = input.ReadLine()) != null)
        {
            var t = line.Split(' ');
            double x = BitConverter.Int64BitsToDouble(unchecked((long)ulong.Parse(t[2], NumberStyles.HexNumber)));
            double y = BitConverter.Int64BitsToDouble(unchecked((long)ulong.Parse(t[3], NumberStyles.HexNumber)));
            double z, max = Math.PI / 2;
            switch (t[1])
            {
                case "sin": z = Math.Sin(x); break;
                case "cos": z = Math.Cos(x); break;
                case "tan": z = Math.Tan(x); break;
                case "atan": z = Math.Atan(x); break;
                case "atan2": z = Math.Atan2(x,y); break;
                case "sqrt": z = Math.Sqrt(x); break;
                case "exp": z = Math.Exp(x); break;
                case "pow": z = Math.Pow(x,y); break;
                case "log": z = Math.Log(x); break;
                case "amount":
                    double a = Math.Abs(x), b = Math.Abs(y);
                    if (a<=0 && b<=0) z=0;
                    else if (a>=max-1e-9 || b>=max-1e-9) z=1;
                    else { a=Math.Tan(a); b=Math.Tan(b); z=Math.Min(1,Math.Atan(Math.Sqrt(a*a+b*b))/max); }
                    break;
                case "azimuth":
                    z = x==0 && y==0 ? 0 : Math.Atan2(Math.Tan(Math.Max(-max+1e-9,Math.Min(max-1e-9,y))), Math.Tan(Math.Max(-max+1e-9,Math.Min(max-1e-9,x))));
                    break;
                default: throw new ArgumentException("未知の関数: " + t[1]);
            }
            output.WriteLine(line + " " + unchecked((ulong)BitConverter.DoubleToInt64Bits(z)).ToString("x16"));
        }
    }
}
