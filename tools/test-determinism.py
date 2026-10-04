#!/usr/bin/env python3
"""数値比較の行対応と、32/64 ビットの ULP 計算を検証する。"""
import contextlib
import importlib.util
import io
import json
from pathlib import Path
import sys
import tempfile
import unittest
sys.dont_write_bytecode=True
ROOT=Path(__file__).resolve().parents[1]
spec=importlib.util.spec_from_file_location('compare',ROOT/'tools/determinism/compare.py')
compare=importlib.util.module_from_spec(spec);spec.loader.exec_module(compare)

class CompareTests(unittest.TestCase):
    def test_adjacent_values_across_sign_and_precision(self):
        for a,b,width in [(0x3ff0000000000000,0x3ff0000000000001,64),(0xbff0000000000000,0xbff0000000000001,64),(0xbf800000,0xbf800001,32)]:
            self.assertEqual(abs(compare.ordered(a,width)-compare.ordered(b,width)),1)
    def check_files(self,left,right):
        (ROOT/'target').mkdir(exist_ok=True)
        with tempfile.TemporaryDirectory(dir=ROOT/'target') as tmp:
            p=Path(tmp);(p/'a').write_text(left);(p/'b').write_text(right)
            with contextlib.redirect_stdout(io.StringIO()):compare.compare(p/'a',p/'b',p/'result')
            return json.loads((p/'result.json').read_text())
    def test_bit_carry_is_not_ulp(self):
        row='sample:0 pow 3ff0000000000000 4000000000000000 '
        d=self.check_files(row+'3fffffff\n',row+'40000000\n')['sample/pow']
        self.assertEqual(d['max_ulp'],1)
        self.assertGreater(d['max_xor_bits'],1)
    def test_missing_reordered_or_precision_changed_fails(self):
        a='sample:0 sin 0000000000000000 0000000000000000 00000000\n'
        for b in ['',a.replace('sample:0','sample:1'),a.rstrip()+'00000000\n']:
            with self.assertRaises(ValueError): self.check_files(a,b)
    def test_empty_fails(self):
        with self.assertRaises(ValueError): self.check_files('','')

if __name__=='__main__':unittest.main()
