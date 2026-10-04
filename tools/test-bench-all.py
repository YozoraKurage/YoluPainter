#!/usr/bin/env python3
"""表が違う負荷を同じ行にまとめないことを確認する。"""
import importlib.util
from pathlib import Path
import sys
import unittest
sys.dont_write_bytecode=True
spec=importlib.util.spec_from_file_location('bench_all',Path(__file__).with_name('bench-all.py'))
bench=importlib.util.module_from_spec(spec);spec.loader.exec_module(bench)

class ParseTests(unittest.TestCase):
    def test_worker_annotation_only_is_removed(self):
        r=bench.parse('M2 tip 半径 40・123 ダブ（ワーカー 12）: 最小 1.00 ms / 中央 2.00 ms（3 回）')
        c=bench.parse('M2 tip 半径 40・123 ダブ: 最小 3.00 ms / 中央 4.00 ms（3 回）')
        self.assertEqual(r.keys(),c.keys())
        self.assertEqual(next(iter(r.values()))[0],2.0)
    def test_different_dab_counts_are_distinct(self):
        r=bench.parse('M2 tip 半径 40・123 ダブ: 最小 1.00 ms / 中央 2.00 ms（3 回）')
        c=bench.parse('M2 tip 半径 40・124 ダブ: 最小 1.00 ms / 中央 2.00 ms（3 回）')
        self.assertNotEqual(r.keys(),c.keys())
    def test_surface_units_and_work(self):
        r=bench.parse('Rust 球 69312 三角形: 組み立て 20.00 ms（隣り合わせ 4.00 ms・BVH 5.00 ms）、3 回の中央値\nRust レイ 1 本 2.500 µs（20000 本・当たり 900。1 本のスレッド）\nRust ダブ 2048² 半径 0.05: 3.00 ms（100 画素、4 スレッド）',True)
        self.assertEqual(len(r),5)
        self.assertEqual(r['レイ 20000 本・当たり 900（1 本あたり）'][:2],(2.5,'µs'))
        self.assertIn('面 ダブ 2048² 半径 0.05・100 画素',r)
    def test_empty_or_encoding_loss_fails(self):
        with self.assertRaises(ValueError):bench.parse('??: ?? 1.00 ms / ?? 2.00 ms?3 ??')
    def test_bad_surface_format_fails(self):
        with self.assertRaises(ValueError):bench.parse('球 69312 三角形: 組み立て ??',True)

if __name__=='__main__':unittest.main()
