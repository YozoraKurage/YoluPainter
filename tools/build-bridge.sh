#!/usr/bin/env bash
# Unity 版へ入れる Live Link のブリッジ（Linux の .so と Windows の .dll）を組み、C# の宣言と一緒に Unity 版のフォルダへ写す。
#
#   tools/build-bridge.sh [Unity 版のパッケージの根]     （根を省くと組むだけ。出来たものは target/bridge-out/）
#
# Windows 向けは x86_64-pc-windows-gnu（mingw-w64 が要る: sudo apt-get install -y mingw-w64）。手元のパス（cargo の置き場・この
# リポジトリの場所）がパニックの場所の文字列として DLL に残らないよう、--remap-path-prefix で置き換える。
set -euo pipefail
here="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cargo_home="${CARGO_HOME:-$HOME/.cargo}"
export PATH="$cargo_home/bin:$PATH"
export RUSTFLAGS="--remap-path-prefix=$cargo_home=/cargo --remap-path-prefix=$here=/yolupainter-rs ${RUSTFLAGS:-}"
cd "$here"
cargo build -p yolu-bridge --profile bridge
cargo build -p yolu-bridge --profile bridge --target x86_64-pc-windows-gnu
out="$here/target/bridge-out"
mkdir -p "$out"
cp target/bridge/libyolu_bridge.so "$out/"
cp target/x86_64-pc-windows-gnu/bridge/yolu_bridge.dll "$out/"
cp crates/yolu-bridge/generated/LiveLinkNative.g.cs "$out/"
if grep -a -q -e "$HOME" -e "$here" "$out/libyolu_bridge.so" "$out/yolu_bridge.dll"; then
  echo "手元のパスが DLL に残っている" >&2; exit 1
fi
if [[ $# -ge 1 ]]; then
  unity="$1"
  cp "$out/libyolu_bridge.so" "$out/yolu_bridge.dll" "$unity/Plugins/LiveLink/"
  cp "$out/LiveLinkNative.g.cs" "$unity/Editor/LiveLink/Native/"
  echo "写した: $unity/Plugins/LiveLink と Editor/LiveLink/Native"
fi
sha256sum "$out/libyolu_bridge.so" "$out/yolu_bridge.dll"
