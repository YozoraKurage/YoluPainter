#!/usr/bin/env bash
# ブラシ形式の取り込みの正解（crates/yolu-io/tests/fixtures/brushes/）を作る。
#   brushes.sh [出力先]
# 1. Rust の試験（brush_golden の export_corpus）が、手で組んだ事例と壊した入力を inputs.bin に書く。
# 2. Unity 版の Runtime/Core（ブラシの読み手を含む）を Unity の同梱の Roslyn で原文のまま組み、Mono で inputs.bin を全部読ませる。
# 3. cases.txt（手で組んだ事例の完全な指紋）と fuzz.txt（壊した入力ごとの結果と指紋の短縮）を出力先へ書く。
# 要るもの: Unity 2022.3 のエディタの同梱の道具（$YOLUPAINTER_CORE_UNITY_DATA）。Unity の原文は $YOLUPAINTER_UNITY_SOURCE（既定 /workspace）。
set -euo pipefail
here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
repo="$(cd "$here/../.." && pwd)"
source_dir="${YOLUPAINTER_UNITY_SOURCE:-/workspace}"
unity="${YOLUPAINTER_CORE_UNITY_DATA:-/opt/unity/Editor/Data}"
out="${1:-$repo/crates/yolu-io/tests/fixtures/brushes}"
build="$repo/target/csharp-golden/brushes"
mkdir -p "$build" "$out"
api="$unity/UnityReferenceAssemblies/unity-4.8-api"
export CARGO_INCREMENTAL=0
BRUSH_CORPUS_OUT="$build/inputs.bin" cargo test -p yolu-io --test brush_golden export_corpus -- --ignored --exact >/dev/null
{
  echo '-nologo -target:exe -langversion:9.0 -optimize+ -nostdlib+ -nowarn:CS1591,CS0618'
  echo "-out:\"$build/brushes.exe\""
  for f in "$api"/*.dll "$api"/Facades/*.dll; do echo "-r:\"$f\""; done
  find "$source_dir/Runtime/Core" -name '*.cs' | LC_ALL=C sort | sed 's/.*/"&"/'
  echo "\"$here/BrushGolden.cs\""
} > "$build/build.rsp"
"$unity/NetCoreRuntime/dotnet" exec "$unity/DotNetSdkRoslyn/csc.dll" /noconfig "@$build/build.rsp" > "$build/build.log" 2>&1 || { cat "$build/build.log"; exit 3; }
"$unity/MonoBleedingEdge/bin/mono" "$build/brushes.exe" "$build/inputs.bin" "$out"
{
  git -C "$source_dir" rev-parse HEAD
  (cd "$source_dir/Runtime/Core/Brushes" && sha256sum *.cs)
} > "$out/source.txt"
echo "書いた: $out（cases.txt $(wc -l < "$out/cases.txt") 行、fuzz.txt $(wc -l < "$out/fuzz.txt") 行）"
