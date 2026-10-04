using System;
using System.Collections.Generic;
using System.IO;
using System.Linq;
using System.Text;
using Yozolab.YoluPainter.Core;
using Yozolab.YoluPainter.Core.Paths;
using Yozolab.YoluPainter.Core.MeshMaps;
using Yozolab.YoluPainter.Core.Persistence;

static class Generate
{
    static int next = 1;
    static Guid Id() => new Guid(next++, 0x1234, 0x5678, new byte[] { 0x80, 1, 2, 3, 4, 5, 6, 7 });
    static void Text(BinaryWriter w, string text) { var b = Encoding.UTF8.GetBytes(text); w.Write(b.Length); w.Write(b); }
    static byte[] Baseline(int v)
    {
        using var s = new MemoryStream(); using var w = new BinaryWriter(s);
        w.Write(Encoding.ASCII.GetBytes("DOTPAINT")); w.Write(v); w.Write(Id().ToByteArray());
        w.Write(9); w.Write(10); w.Write(8);
        if (v >= 7) { w.Write(1); w.Write(false); w.Write(-0.0); w.Write(0); w.Write(1); }
        w.Write(1); w.Write(Id().ToByteArray()); Text(w, "日本語の層"); w.Write(true); w.Write(.625); w.Write(7);
        if (v >= 5) w.Write((byte)1);
        if (v >= 3) { w.Write(0); if (v >= 6) w.Write(Guid.Empty.ToByteArray()); w.Write(0); }
        w.Write(1); w.Write(0); w.Write(true); w.Write(1); w.Write(0); w.Write(0); w.Write(256);
        var tile = new byte[256]; for (int i = 0; i < tile.Length; i++) tile[i] = (byte)(i * 37 + 11); tile[3] = 0; w.Write(tile);
        if (v >= 2) { w.Write(true); w.Write(true); w.Write(false); w.Write(.75); w.Write(1); w.Write(1); w.Write(1); w.Write(256); var mask = new byte[256]; mask[3] = 93; mask[35] = 128; w.Write(mask); }
        if (v >= 8) w.Write(false); if (v >= 9) w.Write(false); if (v >= 10) w.Write(false);
        w.Flush(); return s.ToArray();
    }
    static void Save(string root, string name, PaintDocument doc)
    {
        var bytes = DocumentBinary.Write(doc); var roundtrip = DocumentBinary.Write(DocumentBinary.Read(bytes));
        if (!bytes.SequenceEqual(roundtrip)) throw new Exception("C# 再保存不一致: " + name);
        File.WriteAllBytes(Path.Combine(root, name), bytes);
    }
    static void Main(string[] args)
    {
        if (CompositeFixture.Run(args)) return;
        if (M2Fixture.Run(args)) return;
        if (DocumentBinary.CurrentVersion != 21) throw new Exception("正本21の書き手が必要です");
        var root = args[0]; Directory.CreateDirectory(root);
        for (int v = 1; v <= 21; v++) { var b = Baseline(v); DocumentBinary.Read(b); File.WriteAllBytes(Path.Combine(root, "native-v" + v + ".utpaint"), b); }
        var doc = new PaintDocument(16, 16, 8, 1024 * 1024, Id());
        doc.SetNormalSettings(new NormalSettings(true, -3.25, HeightEdgeMode.Wrap, NormalYDirection.DirectX));
        var raster = doc.AddLayer("画素・マスク・全チャンネル", Id());
        foreach (PaintChannel c in Enum.GetValues(typeof(PaintChannel))) { var tile = new byte[256]; for (int i = 0; i < tile.Length; i++) tile[i] = (byte)(i * 13 + (int)c); tile[3] = 0; raster.GetChannel(c).ImportTile(new TileCoord(0, 0), tile); }
        doc.SetChannelEnabled(raster.Id, PaintChannel.Metallic, false);
        doc.SetLayerOpacity(raster.Id, .4); doc.SetLayerBlendMode(raster.Id, LayerBlendMode.ColorDodge);
        doc.SetChannelBlendForLoad(raster, PaintChannel.Color, new ChannelBlend(LayerBlendMode.Multiply, .375));
        var mask = doc.AddLayerMask(raster.Id); var mt = new byte[256]; mt[3] = 79; mask.Surface.ImportTile(new TileCoord(0, 0), mt);
        doc.SetLayerMaskDensity(raster.Id, .6); doc.SetLayerMaskInverted(raster.Id, true);
        var anchor = doc.AddAnchor(raster.Id, AnchorPlacement.Layer, "下層", Id());
        doc.AddAnchor(raster.Id, AnchorPlacement.Mask, "マスク", Id());
        var filters = new[] { FilterSettings.GaussianBlur(2), FilterSettings.Sharpen(2,.75,17), FilterSettings.Noise(.3,-123,true), FilterSettings.Levels(.1,.9,1.7,.2,.8), FilterSettings.Invert(), FilterSettings.Normalize() };
        foreach (var f in filters) doc.AddFilter(raster.Id, FilterTarget.Content, f, new[]{PaintChannel.Color}, id:Id(), strength:.7);
        doc.AddFilter(raster.Id, FilterTarget.Mask, FilterSettings.Noise(.2,57,true), id:Id());
        var fill = doc.AddFillLayer("画像・投影・ランプ", new Dictionary<PaintChannel,Rgba32> { { PaintChannel.Color,new Rgba32(17,39,61,87) }, {PaintChannel.Height,new Rgba32(128,128,128,255)} }, Id());
        doc.SetFillImagesForLoad(fill, new[] { new KeyValuePair<PaintChannel,Guid>(PaintChannel.Color,Id()) }, FillProjection.DecalAt(FillProjection.DefaultPlacement));
        doc.SetFillGradientsForLoad(fill, new[] { new KeyValuePair<PaintChannel,GeneratorSettings>(PaintChannel.Height,GeneratorSettings.Default(GeneratorType.ShapeGradient).WithRamp(GradientRamp.Default).WithBlend(GeneratorBlend.Replace)) });
        doc.AddAdjustmentLayer("レベル", AdjustmentSettings.Levels(.1,.9,2,.15,.8), new[]{PaintChannel.Color}, Id());
        doc.AddAdjustmentLayer("色相", AdjustmentSettings.HueSaturation(75,.2,-.1), new[]{PaintChannel.Emission}, Id());
        var generators = doc.AddLayer("Generator", Id());
        foreach (GeneratorType t in Enum.GetValues(typeof(GeneratorType))) { var g=GeneratorSettings.Default(t).WithPin(MeshMapKind.Position,new string('b',64)); if(t==GeneratorType.IdColor)g=g.WithIdColors(new[]{0x123456,0xabcdef}).WithIdTolerance(27); if(t==GeneratorType.ShapeGradient)g=g.WithVolume(new ShapeVolume(GeneratorShape.Sphere,1,2,3,15,30,45,2,3,4,.6)); doc.AddFilter(generators.Id, FilterTarget.Content, FilterSettings.FromGenerator(g), new[]{PaintChannel.Height}, id:Id()); }
        doc.AddFilter(generators.Id, FilterTarget.Content, FilterSettings.FromGenerator(GeneratorSettings.Default(GeneratorType.ShapeGradient).WithRamp(GradientRamp.Default)), new[]{PaintChannel.Color}, id:Id());
        var last = generators.Filters.First(f=>f.Settings.Generator.Type==GeneratorType.Anchor);
        doc.SetGeneratorAnchor(generators.Id,last.Id,anchor.Id,PaintChannel.Height,AnchorRead.Coverage);
        var canvas = doc.AddLayer("2D パス", Id()); canvas.GetChannel(PaintChannel.Color); canvas.Path = new CanvasPath(Id(), PaintChannel.Color, new PathBrush(), new[]{new CanvasPoint(1,2,.4),new CanvasPoint(8,12,.9)}, new[]{new ChannelPaint(PaintChannel.Color,new Rgba32(1,2,3,4))});
        var surface = doc.AddLayer("3D パス", Id()); surface.GetChannel(PaintChannel.Color); surface.Path = new SurfacePath(Id(), PaintChannel.Color, "synthetic-model", new PathBrush(), new[]{new PathPoint(0,.2,.3,.7)}, new[]{new ChannelPaint(PaintChannel.Color,new Rgba32(5,6,7,8))});
        var group = doc.AddGroup("グループ", Id()); doc.SetParentForLoad(surface,group.Id);doc.SetLayerBlendMode(group.Id,LayerBlendMode.PassThrough);
        doc.SetLocksForLoad(raster,LayerLocks.Transparency | LayerLocks.Position);
        doc.SetIdColors(new IdColorAssignments(new string('a',64),new Dictionary<int,int>{{0,0x123456},{42,0xffffff}}));
        Save(root,"native-rich-v21.utpaint",doc);
        var selection = SelectionMask.FromAmountTiles(16,16,8,new[]{new KeyValuePair<TileCoord,byte[]>(new TileCoord(0,0),Enumerable.Range(0,64).Select(i=>(byte)(i*3)).ToArray())});
        File.WriteAllBytes(Path.Combine(root,"selection-v1.bin"),SelectionBinary.Write(selection));
        Console.WriteLine("正本1〜21の基礎21件、C#書き手の拡張正本1件、選択範囲1件を生成・検証しました");
    }
}
