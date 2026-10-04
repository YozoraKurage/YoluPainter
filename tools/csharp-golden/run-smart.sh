#!/usr/bin/env bash
# run-smart.sh [出力先]。既定の生成先は target/csharp-smart/golden/。
set -euo pipefail
here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
repo="$(cd "$here/../.." && pwd)"
core="${YOLUPAINTER_UNITY_SOURCE:-/workspace}/Runtime/Core"
unity="${YOLUPAINTER_CORE_UNITY_DATA:-/opt/unity/Editor/Data}"
api="$unity/UnityReferenceAssemblies/unity-4.8-api"
build="$repo/target/csharp-smart"
mkdir -p "$build"
{
 echo '-nologo'; echo '-target:exe'; echo '-langversion:9.0'; echo '-optimize+'; echo '-nostdlib+'; echo '-nowarn:CS1591,CS0618'
 echo "-out:\"$build/smart.exe\""
 for f in "$api"/*.dll "$api"/Facades/*.dll; do echo "-r:\"$f\""; done
 find "$core" -name '*.cs' | LC_ALL=C sort | sed 's/.*/"&"/'
 echo "\"$here/SmartGolden.cs\""
} > "$build/build.rsp"
"$unity/NetCoreRuntime/dotnet" exec "$unity/DotNetSdkRoslyn/csc.dll" /noconfig "@$build/build.rsp" > "$build/build.log" 2>&1 || { cat "$build/build.log"; exit 3; }
fingerprint="$(cd "$core" && find . -name '*.cs' | LC_ALL=C sort | xargs sha256sum | sha256sum | cut -c1-16)"
export GOLDEN_SOURCE="C# Runtime/Core=$fingerprint Mono Release"
"$unity/MonoBleedingEdge/bin/mono" "$build/smart.exe" "${1:-$build/golden}"
