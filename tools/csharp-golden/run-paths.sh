#!/usr/bin/env bash
# run-paths.sh [出力先]。人工パスを C# の評価器へ通す。ビルドの生成物は target/csharp-paths/ 内。
# 出力先の既定は target/csharp-paths/golden。試験が読む正解（crates/yolu-core/tests/golden/paths）を作り直すときは、そこを出力先に渡す。
set -euo pipefail
here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
repo="$(cd "$here/../.." && pwd)"
source_dir="${YOLUPAINTER_UNITY_SOURCE:-/workspace}"
unity="${YOLUPAINTER_CORE_UNITY_DATA:-/opt/unity/Editor/Data}"
build="$repo/target/csharp-paths"
out="${1:-$build/golden}"
mkdir -p "$build/src" "$out"
api="$unity/UnityReferenceAssemblies/unity-4.8-api"
managed="$unity/Managed/UnityEngine"
for f in SurfaceGeometry.cs SurfaceGeometry.Sampling.cs SurfaceRegions.cs; do
 sed -E 's/([A-Za-z_][A-Za-z0-9_.]*(\[[^]]*\])?(\.[A-Za-z_][A-Za-z0-9_]*)*)\.SqrDistance\(/NativeBounds.SqrDistance(\1, /g' "$source_dir/Editor/Preview/$f" > "$build/src/$f"
done
# Slerp はネイティブ。平行法線だけ恒等写像で代用し、それ以外は必ず拒否する。
sed 's/Vector3.Slerp(/PathGolden.ParallelSlerp(/g' "$source_dir/Editor/Preview/SurfacePathRenderer.cs" > "$build/src/SurfacePathRenderer.cs"
{
 echo '-nologo'; echo '-target:exe'; echo '-langversion:9.0'; echo '-optimize+'; echo '-nostdlib+'; echo '-main:PathGolden'; echo '-nowarn:CS1591,CS0618'
 echo "-out:\"$build/paths.exe\""
 for f in "$api"/*.dll "$api"/Facades/*.dll; do echo "-r:\"$f\""; done
 echo "-r:\"$managed/UnityEngine.CoreModule.dll\""
 find "$source_dir/Runtime/Core" -name '*.cs' | LC_ALL=C sort | sed 's/.*/"&"/'
 for f in "$build"/src/*.cs "$here/SurfaceGolden.cs" "$here/PathGolden.cs"; do echo "\"$f\""; done
} > "$build/build.rsp"
"$unity/NetCoreRuntime/dotnet" exec "$unity/DotNetSdkRoslyn/csc.dll" /noconfig "@$build/build.rsp" > "$build/build.log" 2>&1 || { cat "$build/build.log"; exit 3; }
MONO_PATH="$managed" "$unity/MonoBleedingEdge/bin/mono" "$build/paths.exe" "$out"
(cd "$source_dir" && git rev-parse HEAD; sha256sum Runtime/Core/Paths/*.cs Editor/Preview/SurfacePathRenderer.cs) > "$out/source.txt"
