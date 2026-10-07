#!/usr/bin/env bash
# C# Core を原文のままビルドする。fill.sh golden [出力先] / fill.sh bench [回数] [並列度]
set -euo pipefail
here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
repo="$(cd "$here/../.." && pwd)"
source_dir="${YOLUPAINTER_UNITY_SOURCE:-/workspace}"
unity="${YOLUPAINTER_CORE_UNITY_DATA:-/opt/unity/Editor/Data}"
build="$repo/target/csharp-golden/fill"
mkdir -p "$build"
api="$unity/UnityReferenceAssemblies/unity-4.8-api"
{
  echo '-nologo -target:exe -langversion:9.0 -optimize+ -nostdlib+ -nowarn:CS1591,CS0618'
  echo "-out:\"$build/fill.exe\""
  for f in "$api"/*.dll "$api"/Facades/*.dll; do echo "-r:\"$f\""; done
  find "$source_dir/Runtime/Core" -name '*.cs' | LC_ALL=C sort | sed 's/.*/"&"/'
  echo "\"$here/FillGolden.cs\""
} > "$build/build.rsp"
"$unity/NetCoreRuntime/dotnet" exec "$unity/DotNetSdkRoslyn/csc.dll" /noconfig "@$build/build.rsp" > "$build/build.log" 2>&1 || { cat "$build/build.log"; exit 3; }
if [[ "${1:-golden}" == bench ]]; then
  "$unity/MonoBleedingEdge/bin/mono" "$build/fill.exe" bench "${2:-3}" "${3:-4}"
else
  out="${2:-$repo/crates/yolu-core/tests/golden/fill-image}"
  "$unity/MonoBleedingEdge/bin/mono" "$build/fill.exe" golden "$out"
  git -C "$source_dir" rev-parse HEAD > "$out/source.txt"
  (cd "$source_dir/Runtime/Core" && sha256sum FillProjection.cs FillImageSampler.cs ShapeGradient.cs) >> "$out/source.txt"
fi
