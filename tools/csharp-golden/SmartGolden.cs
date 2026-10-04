// 人工データだけでスマート素材の配置画素と棚の索引を生成する。
using System;
using System.IO;
using System.Linq;
using System.Collections.Generic;
using System.Text;
using Yozolab.YoluPainter.Core;
using Yozolab.YoluPainter.Core.Shelf;
using Yozolab.YoluPainter.Core.Persistence;
static class SmartGolden {
 static byte[] CanvasBytes(SparseTileSurface surface,int w,int h){var pixels=new byte[w*h*4];for(int y=0;y<h;y++)for(int x=0;x<w;x++){var p=surface.GetPixel(x,y);int o=(y*w+x)*4;pixels[o]=p.R;pixels[o+1]=p.G;pixels[o+2]=p.B;pixels[o+3]=p.A;}return pixels;}
 static void Main(string[] args) {
  string output=args[0];Directory.CreateDirectory(output);
  var writer=new YlpWriterInfo("YoluPainter","test","none");
  var d=new PaintDocument(3,2,8);var l=d.AddLayer("画素");
  for(int y=0;y<2;y++)for(int x=0;x<3;x++)l.GetChannel(PaintChannel.Color).SetPixel(x,y,new Rgba32((byte)(x*71+y*13),(byte)(x*9+y*81),(byte)(x*37+y*11),(byte)((x+y)%3==0?0:50+x*60+y*20)));
  var material=d.CaptureSmartMaterial(new[]{l.Id},"試験素材");
  File.WriteAllBytes(Path.Combine(output,"raster.ylsmart"),SmartMaterialFile.Write(material,writer));
  foreach(var size in new[]{new[]{3,2},new[]{7,5},new[]{2,1},new[]{2,5}})
   foreach(CanvasResampling how in Enum.GetValues(typeof(CanvasResampling))) {
    var t=new PaintDocument(size[0],size[1],16);var placed=t.PlaceSmartMaterial(material,new SmartPlacement {Resampling=how});
    var surface=t.GetLayer(placed.LayerId).GetChannel(PaintChannel.Color);var pixels=new byte[size[0]*size[1]*4];
    for(int y=0;y<size[1];y++)for(int x=0;x<size[0];x++){var p=surface.GetPixel(x,y);int o=(y*size[0]+x)*4;pixels[o]=p.R;pixels[o+1]=p.G;pixels[o+2]=p.B;pixels[o+3]=p.A;}
    File.WriteAllBytes(Path.Combine(output,$"resize-{size[0]}-{size[1]}-{(int)how}.bin"),pixels);
   }
  d.AddLayerMask(l.Id);l.Mask.Surface.SetPixel(1,1,new Rgba32(0,0,0,178));
  var mask=d.CaptureSmartMask(l.Id,"マスク");File.WriteAllBytes(Path.Combine(output,"mask.ylsmart"),SmartMaterialFile.Write(mask,writer));
  var rich=d.AddFillLayer("塗り",new Dictionary<PaintChannel,Rgba32>{{PaintChannel.Color,new Rgba32(30,70,110,255)},{PaintChannel.Roughness,new Rgba32(70,70,70,255)}});
  var multi=d.CaptureSmartMaterial(new[]{l.Id,rich.Id},"複数層");File.WriteAllBytes(Path.Combine(output,"multi.ylsmart"),SmartMaterialFile.Write(multi,writer));
  var project=new ProjectResources();var image=ImageContent.FromPixels(new byte[]{42,3,9,0,4,5,6,200},2,1);
  project.Add("透明画像",image,ResourceOrigin.Library("test.png",new string('a',64),12),ResourceColorSpace.Srgb,out _,new Guid("10000000-0000-0000-0000-000000000001"));
  project.Add("参照",ImageContent.FromPixels(new byte[]{1,2,3,4},1,1),ResourceOrigin.UnityAsset(new string('b',32),"Assets/Test.png","stamp",false,123),ResourceColorSpace.Linear,out _,new Guid("10000000-0000-0000-0000-000000000002"));
  var files=new Dictionary<string,byte[]>();ResourceIndex.AddTo(files,project);
  foreach(var f in files){string path=Path.Combine(output,f.Key);Directory.CreateDirectory(Path.GetDirectoryName(path));File.WriteAllBytes(path,f.Value);}
  var withImages=d.CaptureSmartMaterial(new[]{l.Id},"画像付き",project,_=>project.Images.Select(i=>i.Id));
  File.WriteAllBytes(Path.Combine(output,"images.ylsmart"),SmartMaterialFile.Write(withImages,writer));
  var filtered=d.CaptureSmartMaterial(new[]{rich.Id},"未対応属性");
  d.AddFilter(rich.Id,FilterTarget.Content,FilterSettings.GaussianBlur(2),new[]{PaintChannel.Color});
  filtered=d.CaptureSmartMaterial(new[]{rich.Id},"フィルター付き");
  File.WriteAllBytes(Path.Combine(output,"filtered.ylsmart"),SmartMaterialFile.Write(filtered,writer));
  foreach(var item in new[]{Tuple.Create(material,"raster",ResourceKind.SmartMaterial),Tuple.Create(mask,"mask",ResourceKind.SmartMask),Tuple.Create(multi,"multi",ResourceKind.Material)}) {
   var bytes=File.ReadAllBytes(Path.Combine(output,item.Item2+".ylsmart"));
   project.AddSmart(item.Item2,bytes,item.Item1,ResourceOrigin.BuiltIn("test",1),out _,new Guid("20000000-0000-0000-0000-"+((int)item.Item3).ToString("D12")),item.Item3);
  }
  var brush=BrushResourceFile.Write(new Dictionary<string,byte[]>{{"state.json",Encoding.UTF8.GetBytes("{\"schema\":3}")},{"tip-0.png",image.EncodePng()}});
  File.WriteAllBytes(Path.Combine(output,"brush.ylbrush"),brush);
  project.AddBrush("ブラシ",brush,ResourceOrigin.File("/synthetic/brush.ylbrush",GenerationStore.Hash(brush),brush.Length),out _,new Guid("30000000-0000-0000-0000-000000000001"));
  files.Clear();ResourceIndex.AddTo(files,project);
  foreach(var f in files){string path=Path.Combine(output,"all",f.Key);Directory.CreateDirectory(Path.GetDirectoryName(path));File.WriteAllBytes(path,f.Value);}
  File.WriteAllText(Path.Combine(output,"used.txt"),project.UsedBytes.ToString(),new UTF8Encoding(false));
  // 疎な元（一様・ばらばら・無いタイル・端のタイルが混ざる）のタイル分けと寸法を変える。タイルを飛ばす・一様に埋める道の照合。
  {
   var sd=new PaintDocument(9,7,2);var sl=sd.AddLayer("疎");var sc=sl.GetChannel(PaintChannel.Color);
   for(int y=0;y<4;y++)for(int x=0;x<4;x++)sc.SetPixel(x,y,new Rgba32(90,100,110,255));
   for(int y=2;y<4;y++)for(int x=6;x<8;x++)sc.SetPixel(x,y,new Rgba32(200,30,60,128));
   for(int y=4;y<6;y++)for(int x=4;x<6;x++)sc.SetPixel(x,y,new Rgba32((byte)((x*40+y*3)%256),(byte)((y*50+x*7)%256),(byte)(((x+y)*20)%256),(byte)((x+y)%3==0?0:100+x*10)));
   sc.SetPixel(8,6,new Rgba32(10,20,30,255));sc.SetPixel(8,0,new Rgba32(50,60,70,255));sc.SetPixel(8,1,new Rgba32(50,60,70,255));
   var sparse=sd.CaptureSmartMaterial(new[]{sl.Id},"疎な素材");
   foreach(var size in new[]{new[]{5,3,4},new[]{20,15,8},new[]{13,11,2},new[]{9,7,4},new[]{4,9,2}})
    foreach(CanvasResampling how in Enum.GetValues(typeof(CanvasResampling))) {
     var t=new PaintDocument(size[0],size[1],size[2]);var placed=t.PlaceSmartMaterial(sparse,new SmartPlacement {Resampling=how});
     File.WriteAllBytes(Path.Combine(output,$"resize-sparse-{size[0]}-{size[1]}-{size[2]}-{(int)how}.bin"),CanvasBytes(t.GetLayer(placed.LayerId).GetChannel(PaintChannel.Color),size[0],size[1]));
    }
   // Normal のチャンネル: 平均してから単位ベクトルに正規化し直す（一様な所は変えない）
   var nd=new PaintDocument(4,3,2);var nl=nd.AddLayer("法線");var nc=nl.GetChannel(PaintChannel.Normal);
   for(int y=0;y<2;y++)for(int x=0;x<2;x++)nc.SetPixel(x,y,new Rgba32(128,128,255,255));
   nc.SetPixel(2,0,new Rgba32(200,128,220,255));nc.SetPixel(3,0,new Rgba32(60,190,200,200));nc.SetPixel(2,1,new Rgba32(128,60,230,0));nc.SetPixel(3,1,new Rgba32(90,90,250,255));nc.SetPixel(2,2,new Rgba32(255,128,128,255));
   var normal=nd.CaptureSmartMaterial(new[]{nl.Id},"法線の素材");
   foreach(var size in new[]{new[]{7,5,4},new[]{2,1,2},new[]{2,5,2},new[]{4,3,4}})
    foreach(CanvasResampling how in Enum.GetValues(typeof(CanvasResampling))) {
     var t=new PaintDocument(size[0],size[1],size[2]);var placed=t.PlaceSmartMaterial(normal,new SmartPlacement {Resampling=how});
     File.WriteAllBytes(Path.Combine(output,$"resize-normal-{size[0]}-{size[1]}-{size[2]}-{(int)how}.bin"),CanvasBytes(t.GetLayer(placed.LayerId).GetChannel(PaintChannel.Normal),size[0],size[1]));
    }
   // マスク: 隠す量（アルファ）だけの面
   var md=new PaintDocument(5,4,2);var ml=md.AddLayer("マスクの層");md.AddLayerMask(ml.Id);
   foreach(var m in new[]{new[]{0,0,120},new[]{1,1,200},new[]{3,0,77},new[]{4,3,255}})ml.Mask.Surface.SetPixel(m[0],m[1],new Rgba32(0,0,0,(byte)m[2]));
   var smartMask=md.CaptureSmartMask(ml.Id,"マスクの素材");
   foreach(var size in new[]{new[]{11,9,4},new[]{3,2,2},new[]{5,4,4},new[]{8,3,8}})
    foreach(CanvasResampling how in Enum.GetValues(typeof(CanvasResampling))) {
     var t=new PaintDocument(size[0],size[1],size[2]);var target=t.AddLayer("先");t.ApplySmartMask(smartMask,target.Id,how);
     File.WriteAllBytes(Path.Combine(output,$"resize-mask-{size[0]}-{size[1]}-{size[2]}-{(int)how}.bin"),CanvasBytes(target.Mask.Surface,size[0],size[1]));
    }
  }
  File.WriteAllText(Path.Combine(output,"source.txt"),Environment.GetEnvironmentVariable("GOLDEN_SOURCE")+"\n",new UTF8Encoding(false));
 }
}
