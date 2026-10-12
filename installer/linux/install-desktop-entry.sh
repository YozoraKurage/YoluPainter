#!/bin/sh
# YoluPainter をデスクトップのアプリのメニューに登録する（この利用者だけ。管理者の権限は要らない）。
# 展開したフォルダの中で実行する。フォルダを動かしたら、もう一度実行する。--remove で登録を外す。
# Registers YoluPainter in the desktop's application menu for the current user (no root needed).
# Run it inside the extracted folder; run it again after moving the folder. --remove unregisters it.
set -eu

here=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
data=${XDG_DATA_HOME:-}
# XDG Base Directory: 空と相対の値は無いものとして扱う
case "$data" in
/*) ;;
*) data="$HOME/.local/share" ;;
esac
entry="$data/applications/yolupainter.desktop"
icon="$data/icons/hicolor/256x256/apps/yolupainter.png"

refresh() {
    if command -v update-desktop-database >/dev/null 2>&1; then
        update-desktop-database "$data/applications" >/dev/null 2>&1 || true
    fi
}

case "${1:-}" in
--remove)
    rm -f "$entry" "$icon"
    refresh
    echo "removed: $entry"
    exit 0
    ;;
"") ;;
*)
    echo "usage: $0 [--remove]" >&2
    exit 2
    ;;
esac

if [ ! -x "$here/yolupainter" ]; then
    echo "yolupainter is not next to this script: $here" >&2
    exit 1
fi
# Exec の値で逃がしが要る文字（" \ ` $ %）と、値に入れられない制御文字（改行・タブなど）を含む道は、簡単のため断る
if [ "$(printf '%s' "$here" | LC_ALL=C tr -d '[:cntrl:]')" != "$here" ]; then
    echo "the folder path has a control character (such as a newline); move the folder and run this again: $here" >&2
    exit 1
fi
case "$here" in
*'"'* | *'\'* | *'`'* | *'$'* | *'%'*)
    echo "the folder path has a character this script does not escape (\" \\ \` \$ %); move the folder and run this again: $here" >&2
    exit 1
    ;;
esac

mkdir -p "$data/applications" "$(dirname "$icon")"
cp "$here/yolupainter.png" "$icon"
tmp="$entry.tmp"
cat >"$tmp" <<DESKTOP
[Desktop Entry]
Type=Application
Version=1.5
Name=YoluPainter
Comment=Paint textures on a 2D canvas and on 3D models
Comment[ja]=2D のキャンバスと 3D のモデルにテクスチャを描く
Exec="$here/yolupainter" %f
Icon=yolupainter
Terminal=false
Categories=Graphics;2DGraphics;3DGraphics;RasterGraphics;
StartupWMClass=YoluPainter
DESKTOP
mv "$tmp" "$entry"
refresh
echo "registered: $entry"
