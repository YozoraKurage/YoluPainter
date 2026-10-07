#!/usr/bin/env bash
# run-seam.sh [dump <事例名> <出力>]。効果とレイヤーのロック・レイヤーの操作のつなぎ目の正解（事例ごとの SHA-256）を作る。
# 既定の生成先は crates/yolu-core/tests/golden/seam.txt。生成物の途中経過は target/csharp-seam/ に置く。
set -euo pipefail
here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
repo="$(cd "$here/../.." && pwd)"
core="${YOLUPAINTER_UNITY_SOURCE:-/workspace}/Runtime/Core"
unity="${YOLUPAINTER_CORE_UNITY_DATA:-/opt/unity/Editor/Data}"
api="$unity/UnityReferenceAssemblies/unity-4.8-api"
build="$repo/target/csharp-seam"
mkdir -p "$build"
{
 echo '-nologo'; echo '-target:exe'; echo '-langversion:9.0'; echo '-optimize+'; echo '-nostdlib+'; echo '-nowarn:CS1591,CS0618'
 echo "-out:\"$build/seam.exe\""
 for f in "$api"/*.dll "$api"/Facades/*.dll; do echo "-r:\"$f\""; done
 find "$core" -name '*.cs' | LC_ALL=C sort | sed 's/.*/"&"/'
 echo "\"$here/SeamGolden.cs\""
} > "$build/build.rsp"
"$unity/NetCoreRuntime/dotnet" exec "$unity/DotNetSdkRoslyn/csc.dll" /noconfig "@$build/build.rsp" > "$build/build.log" 2>&1 || { cat "$build/build.log"; exit 3; }
fingerprint="$(cd "$core" && find . -name '*.cs' | LC_ALL=C sort | xargs sha256sum | sha256sum | cut -c1-16)"
export GOLDEN_SOURCE="C# Runtime/Core=$fingerprint Mono Release"
mono="$unity/MonoBleedingEdge/bin/mono"
if [[ "${1:-}" == "dump" ]]; then exec "$mono" "$build/seam.exe" dump "$2" "$3"; fi
out="${1:-$repo/crates/yolu-core/tests/golden/seam.txt}"
"$mono" "$build/seam.exe" > "$build/index.txt"
cp "$build/index.txt" "$out"
echo "書いた: $out（$(grep -vc '^#' "$out") 事例）"
