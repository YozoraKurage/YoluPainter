#!/usr/bin/env bash
# Unity 版へ入れる Live Link のブリッジ（Linux の .so と Windows の .dll）を組み、C# の宣言と一緒に Unity 版のフォルダへ写す。
#
#   tools/build-bridge.sh [Unity 版のパッケージの根]     （根を省くと組むだけ。出来たものは target/bridge-out/）
#
# Linux の .so は glibc 2.31（Ubuntu 20.04）でも読めるよう、cargo-zigbuild で x86_64-unknown-linux-gnu.2.31 に向けて組む（組む機械の glibc が
# 新しいと、その版の記号を要る .so になり、古い Linux の Unity が読めない）。道具: python3 -m pip install --user ziglang・cargo install cargo-zigbuild。
# Windows 向けは x86_64-pc-windows-gnu（mingw-w64 が要る: sudo apt-get install -y mingw-w64）。手元のパス（cargo の置き場・この
# リポジトリの場所）がパニックの場所の文字列として DLL に残らないよう、--remap-path-prefix で置き換える。
set -euo pipefail
here="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cargo_home="${CARGO_HOME:-$HOME/.cargo}"
export PATH="$cargo_home/bin:$PATH"
export RUSTFLAGS="--remap-path-prefix=$cargo_home=/cargo --remap-path-prefix=$here=/yolupainter-rs ${RUSTFLAGS:-}"
cd "$here"
glibc=2.31
if ! command -v cargo-zigbuild >/dev/null 2>&1; then
  echo "cargo-zigbuild が要る（上の道具）。古い glibc に向けて組めない" >&2; exit 1
fi
cargo zigbuild -p yolu-bridge --profile bridge --target "x86_64-unknown-linux-gnu.$glibc"
cargo build -p yolu-bridge --profile bridge --target x86_64-pc-windows-gnu
out="$here/target/bridge-out"
mkdir -p "$out"
cp target/x86_64-unknown-linux-gnu/bridge/libyolu_bridge.so "$out/"
# 要る glibc の版が上を超えていないか（記号の版の一番新しいもの）
need=$(objdump -T "$out/libyolu_bridge.so" | grep -o 'GLIBC_[0-9.]*' | sed 's/GLIBC_//' | sort -V | tail -1)
if [[ "$(printf '%s\n%s\n' "$need" "$glibc" | sort -V | tail -1)" != "$glibc" ]]; then
  echo "libyolu_bridge.so が glibc $need を要る（$glibc までに収める）" >&2; exit 1
fi
cp target/x86_64-pc-windows-gnu/bridge/yolu_bridge.dll "$out/"
# csbindgen の宣言（[DllImport]）を、C# がライブラリをコピーから読む形（関数ポインター）に書き換える
python3 tools/gen-livelink-native.py crates/yolu-bridge/generated/LiveLinkNative.g.cs "$out/LiveLinkNative.g.cs"
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
