#!/usr/bin/env bash
# run-generator.sh [golden|bench]。生成物は target/csharp-generator/ に置く。
set -euo pipefail
here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
repo="$(cd "$here/../.." && pwd)"
core="${YOLUPAINTER_UNITY_SOURCE:-/workspace}/Runtime/Core"
unity="${YOLUPAINTER_CORE_UNITY_DATA:-/opt/unity/Editor/Data}"
api="$unity/UnityReferenceAssemblies/unity-4.8-api"
build="$repo/target/csharp-generator"
mkdir -p "$build"
{
 echo '-nologo'; echo '-target:exe'; echo '-langversion:9.0'; echo '-optimize+'; echo '-nostdlib+'; echo '-nowarn:CS1591,CS0618'
 echo "-out:\"$build/generator.exe\""
 for f in "$api"/*.dll "$api"/Facades/*.dll; do echo "-r:\"$f\""; done
 find "$core" -name '*.cs' | LC_ALL=C sort | sed 's/.*/"&"/'
 echo "\"$here/GeneratorGolden.cs\""
} > "$build/build.rsp"
"$unity/NetCoreRuntime/dotnet" exec "$unity/DotNetSdkRoslyn/csc.dll" /noconfig "@$build/build.rsp" > "$build/build.log" 2>&1 || { cat "$build/build.log"; exit 3; }
fingerprint="$(cd "$core" && find . -name '*.cs' | LC_ALL=C sort | xargs sha256sum | sha256sum | cut -c1-16)"
export GOLDEN_SOURCE="C# Runtime/Core=$fingerprint Mono Release"
"$unity/MonoBleedingEdge/bin/mono" "$build/generator.exe" "${1:-golden}" "$build/golden"
