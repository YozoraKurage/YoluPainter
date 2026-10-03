#!/usr/bin/env bash
# Unity 版の C# の Core から yolu-core の正解のファイルを作る・C# の速さを測る。
#   run.sh [golden]            台本（crates/yolu-core/tests/golden/cases.txt）を走らせ、同じフォルダへ index.txt と .rgba を書く
#   run.sh bench [回数]         4096² の合成と半径 40 のストロークの時間（Mono）
# オプション:
#   --source DIR   Unity 版のリポジトリ（Runtime/Core を読む。既定 /workspace か $YOLUPAINTER_UNITY_SOURCE）
#   --out DIR      golden の出力先（既定は台本と同じフォルダ）
#   --release      Release で組む（既定は Unity のエディタの既定と同じ Debug。浮動小数の結果は変わらないはず）
# 要るもの: Unity 2022.3 のエディタ（同梱の .NET・Roslyn・Mono・参照アセンブリ）。$YOLUPAINTER_CORE_UNITY_DATA で場所を変えられる。
set -euo pipefail
here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
repo="$(cd "$here/../.." && pwd)"
source_dir="${YOLUPAINTER_UNITY_SOURCE:-/workspace}"
unity="${YOLUPAINTER_CORE_UNITY_DATA:-/opt/unity/Editor/Data}"
optimize="-optimize-"; flavor="debug"
mode="golden"; out=""; extra=()
while [[ $# -gt 0 ]]; do
  case "$1" in
    --source) source_dir="$2"; shift 2 ;;
    --out) out="$2"; shift 2 ;;
    --release) optimize="-optimize+"; flavor="release"; shift ;;
    golden|bench) mode="$1"; shift ;;
    -h|--help) sed -n '2,10p' "$0"; exit 0 ;;
    *) extra+=("$1"); shift ;;
  esac
done
cases="$repo/crates/yolu-core/tests/golden/cases.txt"
[[ -n "$out" ]] || out="$(dirname "$cases")"
core="$source_dir/Runtime/Core"
[[ -d "$core" ]] || { echo "Core のソースが無い: $core" >&2; exit 3; }
dotnet="$unity/NetCoreRuntime/dotnet"; csc="$unity/DotNetSdkRoslyn/csc.dll"; mono="$unity/MonoBleedingEdge/bin/mono"
api="$unity/UnityReferenceAssemblies/unity-4.8-api"
for f in "$dotnet" "$csc" "$mono"; do [[ -e "$f" ]] || { echo "Unity の同梱の道具が無い: $f" >&2; exit 3; }; done

build="$repo/target/csharp-golden/$flavor"
mkdir -p "$build"
rsp="$build/build.rsp"
{
  echo "-nologo"; echo "-target:exe"; echo "-langversion:9.0"; echo "$optimize"; echo "-nostdlib+"; echo "-nowarn:CS1591,CS0618"
  echo "-out:\"$build/golden.exe\""
  for f in "$api"/*.dll "$api"/Facades/*.dll; do echo "-r:\"$f\""; done
  find "$core" -name '*.cs' | LC_ALL=C sort | sed 's/.*/"&"/'
  echo "\"$here/Golden.cs\""
} > "$rsp"
"$dotnet" exec "$csc" /noconfig "@$rsp" > "$build/build.log" 2>&1 || { cat "$build/build.log" >&2; echo "組めなかった" >&2; exit 3; }

if [[ "$mode" == "bench" ]]; then
  exec "$mono" "$build/golden.exe" bench "${extra[@]}"
fi
# 出どころ: Unity 版のコミットと Runtime/Core の中身の指紋（手元の変更があれば印を付ける）
commit="$(git -C "$source_dir" rev-parse --short=12 HEAD 2>/dev/null || echo 不明)"
dirty="$(git -C "$source_dir" status --porcelain -- Runtime/Core 2>/dev/null | head -1)"
fingerprint="$(cd "$core" && find . -name '*.cs' | LC_ALL=C sort | xargs sha256sum | sha256sum | cut -c1-16)"
export GOLDEN_SOURCE="YoluPainter $commit${dirty:+（Runtime/Core に未コミットの変更あり）} Runtime/Core=$fingerprint $flavor"
"$mono" "$build/golden.exe" golden "$cases" "$out"
echo "書いた: $out（$(ls "$out"/*.rgba | wc -l) 個の .rgba、$(du -sh "$out" | cut -f1)）"
