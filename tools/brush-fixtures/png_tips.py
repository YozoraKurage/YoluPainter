#!/usr/bin/env python3
"""同梱の PNG の筆先の被覆率を、Rust とは別の復号器（Pillow）で求めて指紋にする。

Unity 版の PNG の筆先（Editor/Brushes/BrushImport.cs の TipFromPng）は Texture2D.LoadImage で RGBA32 にしてから
  輝度 = (r×299 + g×587 + b×114 + 500) / 1000、被覆率 = (255 − 輝度) × a / 255（整数の割り算）
とする。Core の外（Unity のエンジン）なので C# 側の照合には入らないため、同じ式を Pillow の RGBA 変換の上に書いて、
`crates/yolu-io/tests/fixtures/brushes/png-tips.txt` に「ファイル名 幅x高さ 指紋」の行で書く（行は下から。指紋は
 幅・高さ（i32 のリトルエンディアン）と被覆率の SHA-256 の先頭 8 バイト）。

使い方: python3 tools/brush-fixtures/png_tips.py   （要 Pillow）
"""
import hashlib
import struct
from pathlib import Path

from PIL import Image

root = Path(__file__).resolve().parents[2]
source = root / 'crates/yolu-brush-sets/data/krita4/brushes'
lines = []
for path in sorted(source.glob('*.png')):
    image = Image.open(path)
    assert image.mode in ('1', 'L', 'LA', 'P', 'RGB', 'RGBA'), (path.name, image.mode)
    rgba = image.convert('RGBA')
    width, height = rgba.size
    pixels = rgba.tobytes()
    coverage = bytearray(width * height)
    for y in range(height):
        for x in range(width):
            r, g, b, a = pixels[(y * width + x) * 4:(y * width + x) * 4 + 4]
            luminance = (r * 299 + g * 587 + b * 114 + 500) // 1000
            coverage[(height - 1 - y) * width + x] = (255 - luminance) * a // 255
    digest = hashlib.sha256(struct.pack('<ii', width, height) + bytes(coverage)).hexdigest()[:16]
    lines.append(f'{path.name} {width}x{height} {digest}')
out = root / 'crates/yolu-io/tests/fixtures/brushes/png-tips.txt'
out.write_text('\n'.join(lines) + '\n', encoding='utf-8')
print(f'書いた: {out}（{len(lines)} 個）')
