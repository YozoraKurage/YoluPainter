#!/usr/bin/env python3
"""同じ入力の結果を比べ、全差分と関数別集計を target/ に残す。"""
import argparse
import collections
import json
from pathlib import Path
import struct


def ordered(bits, width):
    return (~bits & ((1 << width) - 1)) if bits >> (width-1) else bits | (1 << (width-1))


def compare(a, b, out):
    counts = collections.defaultdict(lambda: dict(total=0, different=0, max_ulp=0, max_xor_bits=0))
    with a.open() as left, b.open() as right, out.with_suffix('.tsv').open('w') as diff:
        diff.write('id\top\tx\ty\treference\tactual\tulp\txor_bits\n')
        from itertools import zip_longest
        for l, r in zip_longest(left, right):
            if l is None or r is None:
                raise ValueError('行数が違います')
            l, r = l.split(), r.split()
            if len(l) != 5 or len(r) != 5 or l[:4] != r[:4]:
                raise ValueError('入力が違います')
            key = l[0].split(':')[0] + '/' + l[1]
            c = counts[key]; c['total'] += 1
            if len(l[4]) != len(r[4]) or len(l[4]) not in (8,16):
                raise ValueError('結果の精度が違います')
            width = len(l[4])*4
            x, y = int(l[4], 16), int(r[4], 16)
            if x != y:
                ulp, xor = abs(ordered(x,width)-ordered(y,width)), (x ^ y).bit_count()
                c['different'] += 1
                c['max_ulp'] = max(c['max_ulp'], ulp)
                c['max_xor_bits'] = max(c['max_xor_bits'], xor)
                c.setdefault('first', dict(id=l[0], x=l[2], y=l[3], reference=l[4], actual=r[4], ulp=ulp, xor_bits=xor))
                diff.write('\t'.join(l + [r[4], str(ulp), str(xor)])+'\n')
    if not counts:
        raise ValueError('結果が空です')
    out.with_suffix('.json').write_text(json.dumps(counts, ensure_ascii=False, indent=2)+'\n')
    for k, c in counts.items():
        print(f"{k}: {c['different']}/{c['total']}、最大 {c['max_ulp']} ULP / XOR {c['max_xor_bits']} ビット")


def inputs(out):
    mask=(1<<64)-1
    state=3003
    def next64():
        nonlocal state
        state=(state+0x9e3779b97f4a7c15)&mask
        z=state
        z=((z^(z>>30))*0xbf58476d1ce4e5b9)&mask
        z=((z^(z>>27))*0x94d049bb133111eb)&mask
        return z^(z>>31)
    def unit(): return (next64()>>11)*(1.0/9007199254740992.0)
    def tilt(): return 0.0 if next64()%7==0 else (unit()*2-1)*1.5707963267948966
    def bits(x): return struct.pack('>d',x).hex()
    with out.open('w') as f:
        def row(label,op,x,y=0.0): f.write(f'{label} {op} {bits(x)} {bits(y)}\n')
        for i in range(65536):
            x,y=tilt(),tilt()
            for op in ['amount','azimuth']: row(f'pen:{i}',op,x,y)
            for axis,v in [('x',x),('y',y)]: row(f'tilt_{axis}:{i}','tan',v)
        # 以下は関数の代表領域の掃引。筆先の実際の全中間値の列挙ではない。
        for i in range(4096):
            x=(unit()*2-1)*6.283185307179586
            for op in ['sin','cos']: row(f'rotation:{i}',op,x)
            row(f'atan:{i}','atan',unit()*100)
            row(f'atan2:{i}','atan2',unit()*2-1,unit()*2-1)
            row(f'rim:{i}','exp',-unit()*151)
            row(f'rim_square:{i}','pow',unit()*24-12,2.0)
            row(f'square_tip:{i}','pow',unit(),6.0)
            row(f'square_root:{i}','pow',unit()*2,1.0/6.0)
            row(f'levels:{i}','pow',unit(),1.0/1.4)
            row(f'mip:{i}','log',1+unit()*1023)
        for i in range(256): row(f'srgb:{i}','pow',i/255.0,1.0/2.4)
        for i,x in enumerate([1.0,1.7,3.0,9.0,100.0,1e6]): row(f'mip_cases:{i}','log',x)


if __name__ == '__main__':
    p=argparse.ArgumentParser(description=__doc__)
    p.add_argument('--inputs',type=Path)
    p.add_argument('files',nargs='*',type=Path)
    a=p.parse_args()
    if a.inputs: inputs(a.inputs)
    elif len(a.files)==3: compare(*a.files)
    else: p.error('--inputs 出力先 または 基準 結果 集計の接頭辞 を指定')
