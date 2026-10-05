#!/usr/bin/env python3
"""C# の宣言を関数ポインターの形へ書き換える道具（gen-livelink-native.py）が、C の口の全部の関数を落とさず結ぶことを確かめる。"""
import importlib.util
from pathlib import Path
import re
import sys
import unittest
sys.dont_write_bytecode = True

ROOT = Path(__file__).resolve().parents[1]
spec = importlib.util.spec_from_file_location('gen', ROOT / 'tools/gen-livelink-native.py')
gen = importlib.util.module_from_spec(spec)
spec.loader.exec_module(gen)
RAW = ROOT / 'crates/yolu-bridge/generated/LiveLinkNative.g.cs'


def exported_functions():
    names = []
    for name in ['ffi.rs', 'testserver.rs']:
        text = (ROOT / 'crates/yolu-bridge/src' / name).read_text(encoding='utf-8')
        names += re.findall(r'pub (?:unsafe )?extern "C" fn (ylb_\w+)', text)
    return names


class Generator(unittest.TestCase):
    def test_every_exported_function_is_bound_and_no_dllimport_is_left(self):
        out, entries = gen.transform(RAW.read_text(encoding='utf-8'))
        self.assertEqual(sorted(entries), sorted(exported_functions()))
        self.assertNotIn('DllImport', out)
        self.assertNotIn('__DllName', out)
        for name in entries:
            self.assertIn(f'p_{name} = Resolve(symbol, "{name}");', out)
            self.assertIn(f'static IntPtr p_{name};', out)
        # 構造体の宣言はそのまま残る
        for struct in ['YlbEvent', 'YlbRequest', 'YlbSetInfo', 'YlbTestServerStats']:
            self.assertIn(f'partial struct {struct}', out)

    def test_a_call_keeps_the_names_and_the_order_of_the_arguments(self):
        out, _ = gen.transform(RAW.read_text(encoding='utf-8'))
        self.assertIn('internal static ulong ylb_connect(byte* name, int name_len, byte* agent, int agent_len) => '
                      '((delegate* unmanaged[Cdecl]<byte*, int, byte*, int, ulong>)Pointer(p_ylb_connect))(name, name_len, agent, agent_len);', out)
        # C# の予約語の引数名（@event）もそのまま
        self.assertRegex(out, r'ylb_next_event\(ulong handle, YlbEvent\* @event, byte\* buf, int cap\) =>')

    def test_a_declaration_it_cannot_read_stops_the_generation(self):
        bad = ('class C {\n        [DllImport(__DllName, EntryPoint = "other", CallingConvention = CallingConvention.Cdecl, ExactSpelling = true)]\n'
               '        internal static extern int ylb_x(int a);\n    }\n')
        with self.assertRaises(ValueError):
            gen.transform(bad)
        with self.assertRaises(ValueError):
            gen.transform('class C {\n    }\n')
        with self.assertRaises(ValueError):
            gen.split_params('int')


if __name__ == '__main__':
    unittest.main()
