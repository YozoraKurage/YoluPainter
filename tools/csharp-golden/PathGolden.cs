// 人工入力のみ。2D の正本と、同一法線の面（平面・離れた平面・高さの違う平面・急な面を重ねた面）の上の 3D の正本を実行する。
using System;
using System.IO;
using System.Linq;
using System.Collections.Generic;
using System.Diagnostics;
using UnityEngine;
using Yozolab.YoluPainter.Core;
using Yozolab.YoluPainter.Core.Paths;
using Yozolab.YoluPainter.Editor.Preview;
static class PathGolden
{
 public static Vector3 ParallelSlerp(Vector3 a,Vector3 b,float t) {
 if(a.x!=b.x || a.y!=b.y || a.z!=b.z) throw new NotSupportedException("平行法線以外は Unity 実行環境が必要"); return a;
 }
 static PathBrush Brush(int i,bool surface) => new PathBrush {RadiusWorld=surface?.08:3.25,Hardness=i%3==0?1:.7,Spacing=.17,Opacity=.83,Flow=.42,Color=new Rgba32(201,37,89,219),PressureSize=i%2==0,PressureOpacity=i%3==1,PressureFlow=i%3==2,Erase=i==7};
 static CanvasPoint[] Points(int i) {
 var p=new[]{new CanvasPoint(4.5,7.25,.3),new CanvasPoint(29.5,51.5,.9),new CanvasPoint(57.25,9.75,.55)};
 if(i==0)return new CanvasPoint[0];if(i==1)return p.Take(1).ToArray();if(i==2)return p.Take(2).ToArray();
 if(i==4)return new[]{p[0],p[0],p[1],p[2]};if(i==5)return new[]{new CanvasPoint(-12,2),p[1],new CanvasPoint(80,64,0)};
 return p;
 }
 static SurfaceGeometry Plane() {
 var a=new Vector3(0,0,0);var b=new Vector3(1,0,0);var c=new Vector3(1,1,0);var d=new Vector3(0,1,0);
 return new SurfaceGeometry(new List<SurfaceTriangle>{new SurfaceTriangle(a,b,c,new Vector2(0,0),new Vector2(1,0),new Vector2(1,1)),new SurfaceTriangle(a,c,d,new Vector2(0,0),new Vector2(1,1),new Vector2(0,1))});
 }
 static SurfaceTriangle[] Quad(float x0,float y0,float z,float x1,float y1,float u0,float v0,float u1,float v1,int renderer=0,int slot=0) {
 var a=new Vector3(x0,y0,z);var b=new Vector3(x1,y0,z);var c=new Vector3(x1,y1,z);var d=new Vector3(x0,y1,z);
 var ua=new Vector2(u0,v0);var ub=new Vector2(u1,v0);var uc=new Vector2(u1,v1);var ud=new Vector2(u0,v1);
 return new[]{new SurfaceTriangle(a,b,c,ua,ub,uc,renderer,slot),new SurfaceTriangle(a,c,d,ua,uc,ud,renderer,slot)};
 }
 static float NegZero() => BitConverter.ToSingle(BitConverter.GetBytes(int.MinValue),0);
 // 指紋の入力: 平面のほか、離れた平面・急な面（別レンダラー・スロット）と、レンダラー・スロットが非零・負で UV が小数・負・-0.0 の形。
 static readonly string[] ShapeNames={"plane","gap","steep","slots"};
 static SurfaceGeometry Shape(string name) {
 var t=new List<SurfaceTriangle>();
 switch(name) {
 case "plane": return Plane();
 case "gap": t.AddRange(Quad(0,0,0,1,1,0,0,1,.5f)); t.AddRange(Quad(2,0,0,3,1,0,.5f,1,1)); break;
 case "stacked": t.AddRange(Quad(0,0,0,1,1,0,0,1,.5f)); t.AddRange(Quad(1.2f,0,.15f,2.2f,1,0,.5f,1,1)); break;
 case "steep":
 t.AddRange(Quad(0,0,0,1,1,0,0,1,1));
 t.Add(new SurfaceTriangle(new Vector3(.25f,.75f,.02f),new Vector3(.25f,.05f,.02f),new Vector3(.45f,.4f,1.1f),new Vector2(0,0),new Vector2(1,0),new Vector2(.5f,1),1,2));
 break;
 default: {
 Func<int,Vector3> at=k=>new Vector3(k*2,0,0);
 t.Add(new SurfaceTriangle(at(0),at(0)+Vector3.right,at(0)+Vector3.up,new Vector2(.1f,.3333333f),new Vector2(.7234567f,-.25f),new Vector2(1.75f,.5f),1,2));
 t.Add(new SurfaceTriangle(at(1),at(1)+Vector3.right,at(1)+Vector3.up,new Vector2(NegZero(),.9f),new Vector2(.125f,2),new Vector2(1e-7f,.99999994f),3,0));
 t.Add(new SurfaceTriangle(at(2),at(2)+Vector3.right,at(2)+Vector3.up,new Vector2(0,0),new Vector2(1,0),new Vector2(0,1),0,5));
 t.Add(new SurfaceTriangle(at(3),at(3)+Vector3.right,at(3)+Vector3.up,new Vector2(0,0),new Vector2(1,0),new Vector2(0,1),-1,-1));
 break; }
 }
 return new SurfaceGeometry(t);
 }
 // 3D だけの入力 10..14: 離れた平面（穴）・高さの違う平面・急な面（法線の判定と下からのレイ）・筆圧 0 の点（ブラシの筆圧の割り当てが違う 2 通り）。
 static readonly string[] SceneShapes={"gap","stacked","steep","plane","plane"};
 static readonly int[] SceneBrushes={10,11,12,12,13};
 static PathPoint[] ScenePoints(int k) {
 if(k==10||k==11)return new[]{new PathPoint(1,.2,.3,.3),new PathPoint(0,.5,.3,.9),new PathPoint(2,.5,.3,.55),new PathPoint(3,.5,.3,1)};
 if(k==12)return new[]{new PathPoint(1,.2,.3,.6),new PathPoint(0,.5,.3,.9),new PathPoint(1,.5,.3,.8)};
 return new[]{new PathPoint(1,.2,.3,0),new PathPoint(0,.5,.3,.9),new PathPoint(1,.5,.3,0),new PathPoint(0,.1,.1,.6)};
 }
 static PathPoint[] SurfacePoints(int i) {
 var p=new[]{new PathPoint(1,.2,.3,.3),new PathPoint(0,.5,.3,.9),new PathPoint(1,.5,.3,.55)};
 if(i==0)return new PathPoint[0];if(i==1)return p.Take(1).ToArray();if(i==2)return p.Take(2).ToArray();if(i==4)return new[]{p[0],p[0],p[1],p[2]};return p;
 }
 static byte[] Bytes(SparseTileSurface s) {var b=new byte[64*64*4];for(int y=0;y<64;y++)for(int x=0;x<64;x++){var p=s.GetPixel(x,y);int k=(y*64+x)*4;b[k]=p.R;b[k+1]=p.G;b[k+2]=p.B;b[k+3]=p.A;}return b;}
 static string Refusal(Action action) {
 try { action(); return "accepted"; } catch(Exception e) {
 if(e.Message.Contains("Source tile payload"))return "source";
 if(e.Message.Contains("rollback budget"))return "stroke";
 if(e.Message.Contains("too long"))return "samples";
 if(e.Message.Contains("another model"))return "fingerprint";
 if(e is ArgumentException)return "invalid";
 throw;
 }
 }
 public static void Main(string[] args) {
 var outdir=args[0];var g=Plane();
 File.WriteAllLines(Path.Combine(outdir,"fingerprint.txt"),ShapeNames.Select(n=>n+" "+SurfacePathRenderer.Fingerprint(Shape(n))));
 var watch=Stopwatch.StartNew();var lines=new List<string>();
 for(int i=0;i<10;i++) {
 var doc=new PaintDocument(64,64,16);
 var material=i==9?new[]{new ChannelPaint(PaintChannel.Color,new Rgba32(31,87,231)),new ChannelPaint(PaintChannel.Roughness,new Rgba32(140,140,140))}:null;
 var canvas=new CanvasPath(Guid.Empty,PaintChannel.Color,Brush(i,false),Points(i),material);
 foreach(var p in CanvasPathRenderer.RenderChannels(doc,canvas)) {string name="canvas-"+i+"-"+(int)p.Key;File.WriteAllBytes(Path.Combine(outdir,name+".rgba"),Bytes(p.Value));lines.Add(name);}
 var surface=new SurfacePath(Guid.Empty,PaintChannel.Color,SurfacePathRenderer.Fingerprint(g),Brush(i,true),SurfacePoints(i),material);
 try {var r=SurfacePathRenderer.Render(doc,g,surface);foreach(var p in r.Channels){string name="surface-"+i+"-"+(int)p.Key;File.WriteAllBytes(Path.Combine(outdir,name+".rgba"),Bytes(p.Value));lines.Add(name+" "+r.Dabs+" "+r.Gaps);}}
 catch(ArgumentOutOfRangeException){lines.Add("surface-"+i+" rejected-coverage");}
 }
 for(int k=10;k<15;k++) {
 var sg=Shape(SceneShapes[k-10]);var sb=Brush(SceneBrushes[k-10],true);if(k==12)sb.RadiusWorld=.15;
 var sp=new SurfacePath(Guid.Empty,PaintChannel.Color,SurfacePathRenderer.Fingerprint(sg),sb,ScenePoints(k));
 var r=SurfacePathRenderer.Render(new PaintDocument(64,64,16),sg,sp);
 foreach(var p in r.Channels){string name="surface-"+k+"-"+(int)p.Key;File.WriteAllBytes(Path.Combine(outdir,name+".rgba"),Bytes(p.Value));lines.Add(name+" "+r.Dabs+" "+r.Gaps);}
 }
 File.WriteAllLines(Path.Combine(outdir,"index.txt"),lines);
 var refusals=new List<string>();
 var cp=new CanvasPath(Guid.Empty,PaintChannel.Color,Brush(3,false),Points(3));
 foreach(var kind in new[]{"source","stroke"}) {
 var d=new PaintDocument(64,64,16);if(kind=="source")d.SourceBudgetBytes=0;else d.ActiveStrokeBudgetBytes=0;
 refusals.Add(kind+" "+Refusal(()=>CanvasPathRenderer.Render(d,cp)));
 }
 var small=Brush(2,false);small.RadiusWorld=.001;
 refusals.Add("samples "+Refusal(()=>CanvasPathRenderer.Render(new PaintDocument(64,64,16),new CanvasPath(Guid.Empty,PaintChannel.Color,small,new[]{new CanvasPoint(4.5,7.25,.3),new CanvasPoint(1000000,51.5,.9)}))));
 refusals.Add("fingerprint "+Refusal(()=>SurfacePathRenderer.Render(new PaintDocument(64,64,16),g,new SurfacePath(Guid.Empty,PaintChannel.Color,"other",Brush(3,true),SurfacePoints(3)))));
 refusals.Add("canvas-point "+Refusal(()=>{new CanvasPoint(double.NaN,0);}));
 refusals.Add("surface-point "+Refusal(()=>{new PathPoint(0,.8,.8);}));
 refusals.Add("brush "+Refusal(()=>{var b=Brush(3,false);b.RadiusWorld=4097;new CanvasPath(Guid.Empty,PaintChannel.Color,b,Points(3));}));
 refusals.Add("material "+Refusal(()=>{new CanvasPath(Guid.Empty,PaintChannel.Color,Brush(3,false),Points(3),new ChannelPaint[0]);}));
 var sp0=SurfacePoints(3);
 Action<string,string,PathBrush,IEnumerable<PathPoint>,IEnumerable<ChannelPaint>,PaintChannel> surfaceCase=(name,fp,b,pts,m,ch)=>refusals.Add(name+" "+Refusal(()=>{new SurfacePath(Guid.Empty,ch,fp,b,pts,m);}));
 var okFp=SurfacePathRenderer.Fingerprint(g);
 surfaceCase("surface-fingerprint-empty","",Brush(3,true),sp0,null,PaintChannel.Color);
 surfaceCase("surface-fingerprint-128",new string('a',128),Brush(3,true),sp0,null,PaintChannel.Color);
 surfaceCase("surface-fingerprint-129",new string('a',129),Brush(3,true),sp0,null,PaintChannel.Color);
 surfaceCase("surface-fingerprint-wide-64",string.Concat(Enumerable.Repeat("\U0001F600",64)),Brush(3,true),sp0,null,PaintChannel.Color);
 surfaceCase("surface-fingerprint-wide-65",string.Concat(Enumerable.Repeat("\U0001F600",65)),Brush(3,true),sp0,null,PaintChannel.Color);
 surfaceCase("surface-points-4096",okFp,Brush(3,true),Enumerable.Repeat(sp0[0],4096),null,PaintChannel.Color);
 surfaceCase("surface-points-4097",okFp,Brush(3,true),Enumerable.Repeat(sp0[0],4097),null,PaintChannel.Color);
 var rb=Brush(3,true);rb.RadiusWorld=1e6;surfaceCase("surface-radius-1e6",okFp,rb,sp0,null,PaintChannel.Color);
 rb=Brush(3,true);rb.RadiusWorld=1000001;surfaceCase("surface-radius-over",okFp,rb,sp0,null,PaintChannel.Color);
 surfaceCase("surface-channel",okFp,Brush(3,true),sp0,null,(PaintChannel)6);
 surfaceCase("surface-material-duplicate",okFp,Brush(3,true),sp0,new[]{new ChannelPaint(PaintChannel.Color,new Rgba32(1,2,3)),new ChannelPaint(PaintChannel.Color,new Rgba32(4,5,6))},PaintChannel.Color);
 File.WriteAllLines(Path.Combine(outdir,"refusals.txt"),refusals);
 Console.WriteLine("C# パス 20 入力 + 3D 5 入力: "+watch.Elapsed.TotalMilliseconds+" ms（正解の書き出し込み）");
 }
}
