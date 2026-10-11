#!/usr/bin/env python3
"""画面のフォント BIZ UDPGothic から、細い字の抜粋のフォント（`*-Lines.ttf`）を作り直す。

BIZ UDPGothic の下線（_）・ダッシュ（– — ―）・マイナス（−）・上線（‾ ¯）などは、輪郭が 0.3〜0.6 画素より細い棒で、フォントに入っている
ヒンティングの命令が棒の上下の縁を別々に画素の格子へ丸める。細い棒は両方の縁が同じ格子の線に落ち、厚みが 0 になって描かれなくなる
（egui は箱の高さが 0 の字を描かない）。命令を持たないフォントは egui の読み出し（skrifa）が自動のヒンティングを使い、棒が潰れない。
そこで、消えるグリフだけを命令なしで取り出した小さなフォントを、画面のフォントの先頭に置く（`crates/yolu-app/src/ui/fonts.rs`）。

派生のフォントは SIL Open Font License 1.1 のまま。原本の著作権表記と許諾の文は name 表に残し、フォントの名前だけ変える
（原本の `BIZ UDPGothic` は Morisawa Inc. の商標）。縦の寸法（hhea・OS/2・head）は原本のままで、egui が先頭のフォントの寸法で行の高さを決めても、
行の高さと文字の縦の位置は変わらない。

使い方（リポジトリの根から）:
    python3 tools/make-ui-font-lines.py          # crates/yolu-app/assets/fonts/ の 2 つを書き直す
    python3 tools/make-ui-font-lines.py --check  # 書き直した結果が、置いてある 2 つとバイト単位で同じかを確かめる（変えない）

fontTools（`pip install fonttools==4.65.0`）が要る。結果は fontTools の版で変わることがあり、SHA-256 は `tools/licenses-reviewed.json` に固定してある。
"""
import argparse
import hashlib
import io
from pathlib import Path
import sys

import fontTools
from fontTools import subset
from fontTools.ttLib import TTFont

FONTTOOLS_VERSION = '4.65.0'
if fontTools.version != FONTTOOLS_VERSION:
    sys.exit(
        f'fontTools {FONTTOOLS_VERSION} が要ります（今は {fontTools.version}）。版が違うと書き出すバイトが変わり、'
        f'tools/licenses-reviewed.json の SHA-256 と合わなくなります: pip install fonttools=={FONTTOOLS_VERSION}'
    )

ROOT = Path(__file__).resolve().parents[1]
FONTS = ROOT / 'crates/yolu-app/assets/fonts'

# 命令つきの原本で、ある大きさの画素にヒンティングすると厚みが 0 になるグリフ（BIZ UDPGothic 1.051 の全グリフを、
# 8〜48 px を 0.25 刻みで測った。消えたのはこの 9 つだけ。ハイフン（-）は 8〜8.25 px だけで、アプリの大きさの外）。
UNICODES = [0x005F, 0x00AD, 0x00AF, 0x0304, 0x2013, 0x2014, 0x2015, 0x203E, 0x2212]

FACES = ['Regular', 'Bold']


def make(source: Path) -> bytes:
    options = subset.Options()
    options.hinting = False            # fpgm・prep・cvt・グリフごとの命令を外す
    options.layout_features = []       # 字形の置き換え（GSUB）・位置（GPOS）は要らない
    options.layout_closure = False
    options.notdef_outline = True
    options.name_IDs = ['*']
    options.name_languages = ['*']
    options.drop_tables += ['GSUB', 'GPOS', 'gasp', 'DSIG', 'meta', 'vmtx', 'vhea']
    font = subset.load_font(str(source), options)
    subsetter = subset.Subsetter(options)
    subsetter.populate(unicodes=UNICODES)
    subsetter.subset(font)
    rename(font)
    out = io.BytesIO()
    font.save(out)
    return out.getvalue()


def rename(font: TTFont) -> None:
    """名前の表: 著作権表記（0）・許諾（13・14）・商標の注記（7）・版（5）は原本のまま、フォントの名前だけ変え、作り手（8）と URL（11・12）は外す。"""
    name = font['name']
    style = None
    for record in name.names:
        if record.nameID == 2:
            style = str(record)
            break
    style = style or 'Regular'
    family = 'YoluPainter UI Lines'
    full = f'{family} {style}'
    ps = f'YoluPainterUILines-{style}'
    # 日本語などの別の言語の名前（原本の名前を持つ）は捨てる
    name.names = [r for r in name.names if not (r.nameID in (1, 3, 4, 6, 16, 17) and r.langID != 0x409)]
    # 作り手（8）と、その URL（11・12）は外す（派生のフォントをモリサワが作ったように読めないように）
    name.names = [r for r in name.names if r.nameID not in (8, 11, 12)]
    for record in name.names:
        if record.nameID == 1:
            record.string = family
        elif record.nameID == 3:
            record.string = f'{ps};derived-from-BIZ-UDPGothic-1.051'
        elif record.nameID == 4:
            record.string = full
        elif record.nameID == 6:
            record.string = ps
        elif record.nameID in (16, 17):
            record.string = family if record.nameID == 16 else style
    name.setName(
        'Subset of BIZ UDPGothic 1.051: the thin bars (underscore, dashes, minus, overlines) without hinting instructions.',
        10, 3, 1, 0x409,
    )


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument('--check', action='store_true', help='置いてあるフォントと同じかを確かめるだけ')
    args = parser.parse_args()
    status = 0
    for face in FACES:
        source = FONTS / f'BIZUDPGothic-{face}.ttf'
        target = FONTS / f'BIZUDPGothic-{face}-Lines.ttf'
        data = make(source)
        digest = hashlib.sha256(data).hexdigest()
        if args.check:
            same = target.is_file() and target.read_bytes() == data
            print(f"{'同じ' if same else '違う'}: {target.relative_to(ROOT).as_posix()} {digest}")
            status |= 0 if same else 1
        else:
            target.write_bytes(data)
            print(f'{target.relative_to(ROOT).as_posix()} {len(data)} バイト {digest}')
    return status


if __name__ == '__main__':
    sys.exit(main())
