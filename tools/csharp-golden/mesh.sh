#!/usr/bin/env bash
# 純 C# Core と Unity 同梱のコンパイラ・Mono でメッシュマップの正解を作る。
set -euo pipefail
here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
repo="$(cd "$here/../.." && pwd)"
source_dir="${YOLUPAINTER_UNITY_SOURCE:-/workspace}"
unity="${YOLUPAINTER_CORE_UNITY_DATA:-/opt/unity/Editor/Data}"
build="$repo/target/csharp-golden/mesh"
out="${1:-$repo/crates/yolu-io/tests/golden/mesh}"
mkdir -p "$build"
[[ "$out" == bench ]] || mkdir -p "$out"
api="$unity/UnityReferenceAssemblies/unity-4.8-api"
{
  echo '-nologo'; echo '-target:exe'; echo '-langversion:9.0'; echo '-optimize+'; echo '-nostdlib+'
  echo "-out:\"$build/mesh.exe\""
  for f in "$api"/*.dll "$api"/Facades/*.dll; do echo "-r:\"$f\""; done
  find "$source_dir/Runtime/Core" -name '*.cs' | LC_ALL=C sort | sed 's/.*/"&"/'
  echo "\"$here/MeshGolden.cs\""
  echo "\"$here/MeshGoldenRough.cs\""
} > "$build/build.rsp"
"$unity/NetCoreRuntime/dotnet" exec "$unity/DotNetSdkRoslyn/csc.dll" /noconfig "@$build/build.rsp" > "$build/build.log" 2>&1 || { cat "$build/build.log"; exit 3; }
"$unity/MonoBleedingEdge/bin/mono" "$build/mesh.exe" "$out" ${2:+"$2"}

if [[ "$out" != bench ]]; then
  commit="$(git -C "$source_dir" rev-parse HEAD)"
  fingerprint="$(cd "$source_dir/Runtime/Core" && find . -name '*.cs' | LC_ALL=C sort | xargs sha256sum | sha256sum | cut -d' ' -f1)"
  printf 'Unity Core commit: %s\nCore SHA-256: %s\nCompiler: Unity bundled Roslyn, optimize+\nRuntime: Unity bundled Mono\n' "$commit" "$fingerprint" > "$out/source.txt"
fi
