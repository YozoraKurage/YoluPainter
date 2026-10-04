#!/usr/bin/env bash
# Unity 版の C# の Core から yolu-core の正解のファイルを作る・C# の速さを測る。
#   run.sh [golden]            台本（crates/yolu-core/tests/golden/cases.txt）を走らせ、同じフォルダへ index.txt と .rgba を書く
#   run.sh bench [回数]         4096² の合成と半径 40・200 のストローク、M2 のブラシ（ゆらぎ・筆先・質感・デュアル・色・全部・
#                               ぼかし・指先）の時間（Mono。BENCH_ONLY=種類 で M2 の 1 種類だけ）
#   run.sh surface             面の計算の台本（crates/yolu-core/tests/golden/surface/cases.txt）を Unity 版の SurfaceGeometry に通し、
#                              同じフォルダへ index.txt を書く（Editor/Preview の原文を、本物の UnityEngine.CoreModule.dll と組む。
#                              ネイティブの Bounds.SqrDistance の呼び出しだけを SurfaceGolden.cs の写しに置き換える）
#   run.sh surface-bench [回数]  7 万三角形の球で組み立て・レイ・ダブの時間（Mono）
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
    golden|bench|surface|surface-bench) mode="$1"; shift ;;
    -h|--help) sed -n '2,10p' "$0"; exit 0 ;;
    *) extra+=("$1"); shift ;;
  esac
done
cases="$repo/crates/yolu-core/tests/golden/cases.txt"
[[ "$mode" == surface* ]] && cases="$repo/crates/yolu-core/tests/golden/surface/cases.txt"
[[ -n "$out" ]] || out="$(dirname "$cases")"
core="$source_dir/Runtime/Core"
[[ -d "$core" ]] || { echo "Core のソースが無い: $core" >&2; exit 3; }
dotnet="$unity/NetCoreRuntime/dotnet"; csc="$unity/DotNetSdkRoslyn/csc.dll"; mono="$unity/MonoBleedingEdge/bin/mono"
api="$unity/UnityReferenceAssemblies/unity-4.8-api"
for f in "$dotnet" "$csc" "$mono"; do [[ -e "$f" ]] || { echo "Unity の同梱の道具が無い: $f" >&2; exit 3; }; done

if [[ "$mode" == surface* ]]; then
  preview="$source_dir/Editor/Preview"; managed="$unity/Managed/UnityEngine"
  build="$repo/target/csharp-golden/surface-$flavor"
  mkdir -p "$build/src"; rm -f "$build"/src/*.cs
  for f in SurfaceGeometry.cs SurfaceGeometry.Sampling.cs SurfaceRegions.cs; do
    [[ -f "$preview/$f" ]] || { echo "Unity 版の面の計算のソースが無い: $preview/$f" >&2; exit 3; }
    sed -E 's/([A-Za-z_][A-Za-z0-9_.]*(\[[^]]*\])?(\.[A-Za-z_][A-Za-z0-9_]*)*)\.SqrDistance\(/NativeBounds.SqrDistance(\1, /g' "$preview/$f" > "$build/src/$f"
  done
  # 置き換えたのは Bounds.SqrDistance の 5 か所だけ（原文が変わって数が合わなければ、置き換えを見直す）
  replaced=$(grep -o 'NativeBounds\.SqrDistance(' "$build"/src/*.cs | wc -l)
  [[ "$replaced" == 5 ]] || { echo "SqrDistance の呼び出しが 5 か所でない（$replaced）。原文が変わったので run.sh の置き換えを見直す" >&2; exit 3; }
  rsp="$build/build.rsp"
  {
    echo "-nologo"; echo "-target:exe"; echo "-langversion:9.0"; echo "$optimize"; echo "-nostdlib+"; echo "-nowarn:CS1591,CS0618"
    echo "-out:\"$build/surface.exe\""
    for f in "$api"/*.dll "$api"/Facades/*.dll; do echo "-r:\"$f\""; done
    echo "-r:\"$managed/UnityEngine.CoreModule.dll\""
    find "$core" -name '*.cs' | LC_ALL=C sort | sed 's/.*/"&"/'
    for f in "$build"/src/*.cs; do echo "\"$f\""; done
    echo "\"$here/SurfaceGolden.cs\""
  } > "$rsp"
  "$dotnet" exec "$csc" /noconfig "@$rsp" > "$build/build.log" 2>&1 || { cat "$build/build.log" >&2; echo "組めなかった" >&2; exit 3; }
  # Unity の外の Mono で UnityEngine.CoreModule の C# の部分（Vector3・Mathf・Bounds・Ray）を使う。版の違いの警告は捨てる
  run_mono() { MONO_PATH="$managed" "$mono" "$build/surface.exe" "$@" 2> >(grep -v -e "out of sync" -e "update one from git" -e "compile and install" -e "the other too" -e "Do not report this as a bug" -e "you probably have a broken" -e "If you see other errors" -e "and you need to fix your mono" -e "The out of sync library" -e '^$' >&2); }
  if [[ "$mode" == "surface-bench" ]]; then run_mono bench "${extra[@]}"; exit 0; fi
  commit="$(git -C "$source_dir" rev-parse --short=12 HEAD 2>/dev/null || echo 不明)"
  dirty="$(git -C "$source_dir" status --porcelain -- Editor/Preview Runtime/Core 2>/dev/null | head -1)"
  fingerprint="$(cd "$preview" && sha256sum SurfaceGeometry.cs SurfaceGeometry.Sampling.cs SurfaceRegions.cs | sha256sum | cut -c1-16)"
  export GOLDEN_SOURCE="YoluPainter $commit${dirty:+（未コミットの変更あり）} Editor/Preview/Surface*=$fingerprint UnityEngine.CoreModule $flavor"
  run_mono golden "$cases" "$out"
  echo "書いた: $out/index.txt（$(grep -c '^case ' "$out/index.txt") 事例）"
  exit 0
fi

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
