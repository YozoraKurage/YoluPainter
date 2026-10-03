using System;
using System.Collections.Generic;
using System.IO;
using System.Linq;
using Yozolab.YoluPainter.Core;
using Yozolab.YoluPainter.Core.Persistence;
using Yozolab.YoluPainter.Core.Shelf;

static class CompositeFixture
{
    static int next = 1000;
    static Guid Id() => new Guid(next++, 0x1234, 0x5678, new byte[] { 0x80, 1, 2, 3, 4, 5, 6, 7 });
    public static bool Run(string[] args)
    {
        if (args[0] == "--composite") { Export(args[1], args[2]); return true; }
        if (args[0] == "--roundtrip-native") { File.WriteAllBytes(args[2], DocumentBinary.Write(DocumentBinary.Read(File.ReadAllBytes(args[1])))); return true; }
        if (args[0] != "--m1") return false;
        if (YlpFormat.Current != 7 || DocumentBinary.CurrentVersion != 21) throw new Exception("形式7・正本21の書き手が必要です");
        Directory.CreateDirectory(args[1]);
        for (int mode = 0; mode < 26; mode++)
        {
            var doc = new PaintDocument(17, 11, 8, 1024 * 1024, Id());
            for (int l = 0; l < 5; l++)
            {
                var layer = doc.AddLayer("日本語の層 " + l, Id());
                doc.SetLayerOpacity(layer.Id, new[]{.83, .625, .375, .97, .51}[l]);
                doc.SetLayerBlendMode(layer.Id, l == 0 ? LayerBlendMode.Normal : (LayerBlendMode)((mode + l - 1) % 26));
                doc.SetLayerClipping(layer.Id, l == 0 || l == 2 || l == 3);
                doc.SetLayerVisibility(layer.Id, l != 4);
                for (int ty = 0; ty < 2; ty++) for (int tx = 0; tx < 3; tx++)
                {
                    var tile = new byte[8 * 8 * 4];
                    for (int y = 0; y < 8; y++) for (int x = 0; x < 8; x++)
                    {
                        int px = tx * 8 + x, py = ty * 8 + y, i = (y * 8 + x) * 4;
                        if (px >= doc.Width || py >= doc.Height) continue;
                        tile[i] = (byte)(px * 37 + py * 19 + l * 57 + 11);
                        tile[i+1] = (byte)(px * 17 + py * 67 + l * 39 + 41);
                        tile[i+2] = (byte)(px * 71 + py * 11 + l * 17 + 73);
                        tile[i+3] = (px + py) % 5 == 0 ? (byte)0 : (byte)(px * 13 + py * 29 + l * 83);
                    }
                    layer.GetChannel(PaintChannel.Color).ImportTile(new TileCoord(tx, ty), tile);
                }
            }
            var setId = Id();
            var sets = new List<YlpTextureSetInfo>();
            var files = new Dictionary<string, byte[]>();
            // mode 0は先頭と現在セットを変え、materialの全種類を同じ実ファイルで検証する。
            if (mode == 0)
            {
                var blank = new PaintDocument(17, 11, 8, 1024 * 1024, Id());
                var materials = new[]{YlpMaterialRef.UnassignedSlots, YlpMaterialRef.Material("同名"), YlpMaterialRef.Material("同名", new string('a',32), long.MinValue)};
                for (int i=0;i<materials.Length;i++) {
                    var id = Id(); sets.Add(new YlpTextureSetInfo(id,"空のセット " + i, materials[i]));
                    files[YlpFormat.SetEntry(id,"document.utpaint")] = DocumentBinary.Write(blank);
                }
            }
            sets.Add(new YlpTextureSetInfo(setId, "合成セット", YlpMaterialRef.PendingSlot(65535)));
            files[YlpFormat.SetEntry(setId,"document.utpaint")] = DocumentBinary.Write(doc);
            files["project.json"] = YlpFormat.WriteProject(new YlpProjectInfo(sets, setId));
            var writer = new YlpWriterInfo("io-fixtures", "1", "none");
            YlpFormat.Stamp(files, writer, writer);
            var path = Path.Combine(args[1], "m1-mode-" + mode.ToString("D2") + ".ylp");
            File.WriteAllBytes(path, YlpArchive.Write(files));
            Export(path, Path.ChangeExtension(path, ".png"));
        }
        Pattern(args[1]);
        Console.WriteLine("形式7のM1合成26モード＋圧縮用パターン1件をC#で生成しました");
        return true;
    }
    static void Pattern(string root)
    {
        var doc = new PaintDocument(65, 33, 8, 1024 * 1024, Id());
        var layer = doc.AddLayer("圧縮用の縞とグラデーション", Id());
        for (int ty=0;ty<5;ty++) for (int tx=0;tx<9;tx++) {
            var tile = new byte[256];
            for (int y=0;y<8;y++) for (int x=0;x<8;x++) {
                int px=tx*8+x, py=ty*8+y, i=(y*8+x)*4;
                if (px>=65 || py>=33) continue;
                tile[i]=(byte)(px*3); tile[i+1]=(byte)(py*7); tile[i+2]=80; tile[i+3]=(byte)(px%4==0 ? 127 : 255);
            }
            layer.GetChannel(PaintChannel.Color).ImportTile(new TileCoord(tx,ty),tile);
        }
        var set = Id();
        var files = new Dictionary<string,byte[]> {
            {YlpFormat.SetEntry(set,"document.utpaint"), DocumentBinary.Write(doc)},
            {"project.json",YlpFormat.WriteProject(new YlpProjectInfo(new[]{new YlpTextureSetInfo(set,"パターン",YlpMaterialRef.UnassignedSlots)},set))}
        };
        var writer=new YlpWriterInfo("io-fixtures","1","none"); YlpFormat.Stamp(files,writer,writer);
        var path=Path.Combine(root,"m1-pattern.ylp"); File.WriteAllBytes(path,YlpArchive.Write(files));
        Export(path,Path.ChangeExtension(path,".png"));
    }
    static void Export(string input, string output)
    {
        var project = YlpFormat.Open(YlpArchive.Read(File.ReadAllBytes(input)));
        var doc = DocumentBinary.Read(project.SetFiles(project.Project.CurrentSet)["document.utpaint"]);
        File.WriteAllBytes(output, RgbaPng.Encode(doc.Composite(PaintChannel.Color), doc.Width, doc.Height));
    }
}
