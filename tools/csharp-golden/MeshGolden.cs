// Unity 同梱の Mono で Core の保存正本を作る。実データを使わない。
using System;
using System.IO;
using Yozolab.YoluPainter.Core.MeshMaps;
static partial class MeshGolden
{
    static void Bvh(string output, bool tied)
    {
        const int n = 97;
        var corners = new float[n * 9]; var uvs = new float[n * 6]; var slots = new int[n]; var indices = new int[n];
        for (int t = 0; t < n; t++)
        {
            float x = tied ? 0 : (t * 37 % 101) / 8f, y = tied ? 0 : (t * 19 % 71) / 8f, z = tied ? 0 : (t * 7 % 29) / 16f;
            int k = t * 9; corners[k] = x; corners[k+1] = y; corners[k+2] = z;
            corners[k+3] = x+0.5f; corners[k+4] = y; corners[k+5] = z;
            corners[k+6] = x; corners[k+7] = y+0.75f; corners[k+8] = z;
            uvs[t*6+2] = 1; uvs[t*6+5] = 1; indices[t] = t;
        }
        var input = new MeshBakeInput(corners, null, uvs, slots);
        var bvh = new MeshRayBvh(corners, indices);
        bvh.Flatten(out var bounds, out var first, out var count, out var triangles, out var original);
        using (var w = new BinaryWriter(File.Create(Path.Combine(output, tied ? "bvh-tied.bin" : "bvh-spread.bin"))))
        {
            w.Write(input.Hash); w.Write(input.TopologyHash); w.Write(bvh.NodeCount);
            for (int i = 0; i < bvh.NodeCount; i++) { for (int j=0;j<6;j++) w.Write(bounds[i*6+j]); w.Write(first[i]); w.Write(count[i]); }
            for (int i=0;i<n;i++) { w.Write(original[i]); for(int j=0;j<10;j++) w.Write(triangles[i*10+j]); }
            for(int t=0;t<n;t++)
            {
                int k=t*9; double ox=corners[k]+0.1, oy=corners[k+1]+0.1;
                double distance=bvh.Trace(ox,oy,5,0,0,-1,10,-1,false,false,new int[128],new double[128],out int hit,out double u,out double v);
                w.Write(distance);w.Write(hit);w.Write(u);w.Write(v);
            }
        }
    }
    static MeshBakeInput Cube(float scale, int scenario=0)
    {
        var corners = new float[12*9]; var uvs = new float[12*6]; var slots = new int[12];
        // 軸ごとに2面。共有辺の向きが揃うよう符号で角の順を反転する。
        int[] order = {0,1,2,0,2,3};
        for(int face=0;face<6;face++)
        {
            int axis=face/2; float sign=face%2==0?1:-1;
            for(int c=0;c<6;c++)
            {
                int v=order[c]; float u=v==0||v==3?0:1, w=v<2?0:1;
                int k=(face*6+c)*3; corners[k+axis]=sign*scale;
                corners[k+(axis+1)%3]=(u*2-1)*scale;corners[k+(axis+2)%3]=(w*2-1)*sign*scale;
                int q=(face*6+c)*2;uvs[q]=(face%3+u)/3;uvs[q+1]=(face/3+w)/2;
            }
            slots[face*2]=slots[face*2+1]=face;
        }
        if(scenario==11) for(int t=0;t<12;t++) Array.Copy(new float[]{0,0,1,0,0,1},0,uvs,t*6,6);
        if(scenario==14) for(int t=0;t<12;t++) for(int a=0;a<3;a++) { float v=corners[t*9+3+a];corners[t*9+3+a]=corners[t*9+6+a];corners[t*9+6+a]=v; }
        float[] normals=scenario==10?MeshBakeInput.ReconstructNormals(corners,180):null;
        float[] colors=null; if(scenario==5) { colors=new float[12*12]; for(int j=0;j<colors.Length;j++) colors[j]=(j*17%31)/30f; }
        string[] names=scenario==9?new[]{scale==1?"shell_low":"other_high"}:null;
        string[] materials=null;if(scenario==7) { materials=new string[12];for(int t=0;t<12;t++) materials[t]=t%3==0?"合成A":t%3==1?"合成B":null; }
        return new MeshBakeInput(corners,normals,uvs,slots,colors:colors,rendererNames:names,materialKeys:materials);

    }
    static void BakeCases(string output)
    {
        for(int scenario=0;scenario<16;scenario++)
        {
            var input=Cube(1,scenario); var reference=scenario==2||scenario==9?Cube(1.01f,scenario):null;
            var settings=new MeshBakeSettings { Width=24,Height=16,TargetSlot=-1,Padding=scenario==3?2:0,Antialiasing=scenario==1?2:1,
                Maps=(MeshMapKind[])Enum.GetValues(typeof(MeshMapKind)),AoSamples=8,ThicknessSamples=8,CurvatureRadius=0.2,ThicknessMaxDistance=1,AoMaxDistance=1 };
            if(scenario==3) { settings.TargetSlot=0;settings.IdSource=MeshIdSource.UvIsland; }
            if(scenario==4) settings.IdSource=MeshIdSource.Mesh;
            if(scenario==5) settings.IdSource=MeshIdSource.VertexColor;
            if(scenario==6) settings.IdSource=MeshIdSource.MeshPart;
            if(scenario==7) settings.IdSource=MeshIdSource.MaterialAsset;
            if(scenario==8) {var parts=new IdPartIndex(input);settings.ManualIdColors=new IdColorAssignments(parts.Binding,new System.Collections.Generic.Dictionary<int,int>{{0,0x123456},{2,0xfedcba}});}
            if(scenario==9) settings.ReferenceMatchByName=true;
            if(scenario==12) {settings.TargetSlot=0;settings.TargetSlots=new[]{0,2};settings.Antialiasing=4;settings.Padding=3;settings.Occluders=MeshOccluders.TargetSlotOnly;}
            if(scenario==15) { settings.AoMaxDistance=1e-5;settings.ThicknessMaxDistance=0.10000000000000002; }
            if(scenario==13) {settings.AoIgnoreBackfaces=true;settings.AoFalloff=MeshOcclusionFalloff.None;}
            var result=MeshBaker.Bake(input,settings,new MeshBakeBudget {MaxDegreeOfParallelism=1}, reference:reference);
            foreach(var map in result.Maps) File.WriteAllBytes(Path.Combine(output,"bake-"+scenario+"-"+map.Kind+".bin"),MeshMapBinary.Write(map));
        }
    }
    static MeshBakeInput BenchmarkModel()
    {
        const int divisions=76;
        int triangles=12*divisions*divisions;
        var corners=new float[triangles*9];var uvs=new float[triangles*6];var slots=new int[triangles];
        int[] order={0,1,2,0,2,3};int at=0;
        for(int face=0;face<6;face++) for(int y=0;y<divisions;y++) for(int x=0;x<divisions;x++)
        {
            int axis=face/2;float sign=face%2==0?1:-1;
            for(int c=0;c<6;c++)
            {
                int v=order[c];float u=(x+(v==0||v==3?0:1))/(float)divisions,w=(y+(v<2?0:1))/(float)divisions;
                int k=at*3;corners[k+axis]=sign;corners[k+(axis+1)%3]=u*2-1;corners[k+(axis+2)%3]=(w*2-1)*sign;
                uvs[at*2]=(face%3+u)/3;uvs[at*2+1]=(face/3+w)/2;slots[at/3]=face;at++;
            }
        }
        return new MeshBakeInput(corners,null,uvs,slots);
    }
    static void Benchmark()
    {
        var input=BenchmarkModel();
        Console.WriteLine("triangles="+input.TriangleCount+",size=2048x2048,samples=8,threads=4,padding=0,antialiasing=1");
        foreach(MeshMapKind kind in Enum.GetValues(typeof(MeshMapKind)))
        {
            var settings=new MeshBakeSettings {Width=2048,Height=2048,TargetSlot=-1,Padding=0,Maps=new[]{kind},AoSamples=8,ThicknessSamples=8};
            var result=MeshBaker.Bake(input,settings,new MeshBakeBudget {MaxDegreeOfParallelism=4});
            Console.WriteLine(kind+","+result.Report.TotalSeconds.ToString("F6",System.Globalization.CultureInfo.InvariantCulture)+","+result.Report.Rays+","+PayloadSha(result.Maps[0]));
        }
    }
    static void Main(string[] args)
    {
        if(args[0]=="bench") { Benchmark(); return; }
        Directory.CreateDirectory(args[0]);
        foreach (MeshMapKind kind in Enum.GetValues(typeof(MeshMapKind)))
        {
            const int width = 19, height = 13;
            int channels = BakedMeshMap.ChannelCount(kind);
            var data = new ushort[width * height * channels];
            var coverage = new byte[width * height];
            uint state = 0x12345678;
            for (int i = 0; i < data.Length; i++) { state ^= state << 13; state ^= state >> 17; state ^= state << 5; data[i] = (ushort)state; }
            for (int i = 0; i < coverage.Length; i++) coverage[i] = (byte)(i % 4);
            var p = new MeshMapProvenance(kind, 2, "synthetic-mesh", "synthetic-topology", 3, width, height, 2, 4, 3,
                "fixture=保存試験", "SnapshotWorld", "StaticSnapshot", "Self", new[] { -1.25, 0.0, -3.0 }, new[] { 4.0, 5.5, 6.0 }, new[] { 2, 7 });
            byte[] bytes = MeshMapBinary.Write(new BakedMeshMap(p, data, coverage));
            File.WriteAllBytes(Path.Combine(args[0], kind + ".v3.bin"), bytes);
            // 旧形式は現在の書き手が生成しないので、追加された欄だけを除いた形を読み手でも検証する。
            int aa = 8 + 12 + 4 + 14 + 4 + 18 + 5 * 4;
            using (var stream = new MemoryStream())
            {
                stream.Write(bytes, 0, aa + 8);
                stream.Write(bytes, aa + 20, bytes.Length - aa - 20);
                var v2 = stream.ToArray(); Array.Copy(BitConverter.GetBytes(2), 0, v2, 8, 4);
                MeshMapBinary.Read(v2);
                File.WriteAllBytes(Path.Combine(args[0], kind + ".v2.bin"), v2);
                using (var old = new MemoryStream())
                {
                    old.Write(v2, 0, aa); old.Write(v2, aa + 4, v2.Length - aa - 4);
                    var v1 = old.ToArray(); Array.Copy(BitConverter.GetBytes(1), 0, v1, 8, 4);
                    MeshMapBinary.Read(v1);
                    File.WriteAllBytes(Path.Combine(args[0], kind + ".v1.bin"), v1);
                }
            }
        }
        Bvh(args[0], false); Bvh(args[0], true);
        BakeCases(args[0]);
        BvhModes(args[0]); BvhRough(args[0]); IdPaletteOutput(args[0]);
        string dump = args.Length > 1 ? args[1] : null; if (dump != null) Directory.CreateDirectory(dump);
        RoughCasesOutput(args[0], dump);
        Console.WriteLine("保存正解: 10種類 × 3形式、BVH 4事例、不規則・高ポリの事例");
    }
}
