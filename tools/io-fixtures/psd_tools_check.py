#!/usr/bin/env python3
"""Rust が書いた PSD の調整レイヤー（YOLU_PSD_DUMP で出した adjust0〜5.psd）を psd-tools で読み直して値を確かめる（開発用）。

  YOLU_PSD_DUMP=<ディレクトリ> cargo test -p yolu-io --test psd_adjust dump_psd_for_psd_tools
  python3 tools/io-fixtures/psd_tools_check.py <ディレクトリ>

psd-tools（MIT）は開発用の scratch の venv に入れる（`python3 -m venv <scratch>/venv && <scratch>/venv/bin/pip install psd-tools`）。
読めた値を試験の期待値（tests/psd_adjust.rs の exact()）と同じかを人が見る。Photoshop・CLIP STUDIO の実機の読みではない。
"""
import sys
from pathlib import Path
from psd_tools import PSDImage
from psd_tools.constants import Tag

def block(layer, tag):
    return layer._record.tagged_blocks.get_data(tag)

root = Path(sys.argv[1])
expect = {
    0: ('grdm', Tag.GRADIENT_MAP),
    1: ('curv', Tag.CURVES),
    2: ('blnc', Tag.COLOR_BALANCE),
    3: ('CgEd', Tag.CONTENT_GENERATOR_EXTRA_DATA),
    4: ('thrs', Tag.THRESHOLD),
    5: ('post', Tag.POSTERIZE),
}
for i, (name, tag) in expect.items():
    psd = PSDImage.open(root / f'adjust{i}.psd')
    layer = [l for l in psd if l.kind != 'pixel'][0]
    data = block(layer, tag)
    print(f'--- adjust{i}.psd  layer kind={layer.kind}  key={name}')
    if name == 'grdm':
        print('reversed', data.is_reversed, 'dithered', data.is_dithered, 'name', repr(data.name))
        for s in data.color_stops:
            print('  color stop location', s.location, 'midpoint', s.midpoint, 'mode', s.mode, 'color', s.color)
        for s in data.transparency_stops:
            print('  opacity stop location', s.location, 'midpoint', s.midpoint, 'opacity', s.opacity)
        print('expansion', data.expansion, 'interpolation', data.interpolation, 'length', data.length, 'mode', data.mode)
    elif name == 'curv':
        print('is_map', data.is_map, 'version', data.version, 'channels', bin(data.count_map))
        for c in data.data:
            print('  points (output, input):', [tuple(p) for p in c])
    elif name == 'blnc':
        print('shadows', data.shadows, 'midtones', data.midtones, 'highlights', data.highlights, 'luminosity', data.luminosity)
    elif name == 'CgEd':
        print('brightness', layer.brightness, 'contrast', layer.contrast, 'mean', layer.mean, 'lab', layer.lab,
              'use_legacy', layer.use_legacy, 'vrsn', layer.vrsn, 'auto', layer.automatic)
        raw = block(layer, Tag.BRIGHTNESS_AND_CONTRAST)
        print('brit (旧式の記録):', raw)
    else:
        print('value', getattr(data, 'value', data))
