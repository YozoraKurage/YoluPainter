#!/usr/bin/env python3
"""画面の試験を並べて回すツール（render-tests.py）の確かめ。試験になる target が列のどこかに入ること、
上限の時間を越えた試験がプロセスのグループごと殺されて失敗になり、その列の残りは回さなかった物として残ることを確かめる。
cargo は呼ばない（`cargo metadata` の応答は偽、回す command は sh に置き換える）。"""
import contextlib
import importlib.util
import io
import json
from pathlib import Path
import sys
import tempfile
import time
import unittest
from unittest import mock
sys.dont_write_bytecode = True

ROOT = Path(__file__).resolve().parents[1]
spec = importlib.util.spec_from_file_location('render_tests', ROOT / 'tools/render-tests.py')
rt = importlib.util.module_from_spec(spec)
spec.loader.exec_module(rt)


def metadata(*targets):
    """targets: (パッケージ, target の名前, kind, test)。"""
    packages = {}
    for package, name, kind, test in targets:
        packages.setdefault(package, []).append({'name': name, 'kind': [kind], 'test': test})
    return json.dumps({'packages': [{'name': p, 'targets': t} for p, t in packages.items()]})


BASE = [
    ('yolu-gpu', 'yolu_gpu', 'lib', True),
    ('yolu-gpu', 'bake', 'test', True),
    ('yolu-gpu', 'measure', 'example', False),
    ('yolu-app', 'yolu_app', 'lib', True),
    ('yolu-app', 'yolupainter', 'bin', True),
    ('yolu-app', 'gui_view3d', 'test', True),
    ('yolu-app', 'gui_canvas', 'test', True),
    ('yolu-app', 'gui_shell', 'test', True),
    ('yolu-app', 'headless', 'test', True),
]


def plan_of(targets, lanes=3):
    with mock.patch.object(rt.subprocess, 'check_output', return_value=metadata(*targets)):
        return rt.plan(ROOT, lanes)


def alive(pid):
    """殺した直後は親を失ったプロセスが終了の回収を待つ間 Z（ゾンビ）で残るので、それは生きていると数えない。"""
    try:
        state = Path(f'/proc/{pid}/stat').read_text().rsplit(')', 1)[1].split()[0]
    except (FileNotFoundError, ProcessLookupError):
        return False
    return state != 'Z'


class Plan(unittest.TestCase):
    def test_every_runnable_target_is_in_exactly_one_lane(self):
        for lanes in (1, 2, 3, 4):
            flat = [i for lane in plan_of(BASE, lanes) for i in lane]
            self.assertEqual(sorted(flat), sorted(['bake', 'gui_view3d', 'gui_canvas', 'gui_shell', 'headless', '--lib', '--bins', '--doc']))

    def test_bin_tests_are_run(self):
        self.assertIn('--bins', [i for lane in plan_of(BASE) for i in lane])

    def test_a_new_integration_test_is_added(self):
        flat = [i for lane in plan_of(BASE + [('yolu-app', 'brand_new', 'test', True)]) for i in lane]
        self.assertIn('brand_new', flat)

    def test_a_test_target_that_no_round_runs_stops_the_plan(self):
        for kind in ('example', 'bench'):
            with self.assertRaises(SystemExit) as ctx:
                plan_of(BASE + [('yolu-app', 'stray', kind, True)])
            self.assertIn('stray', str(ctx.exception))

    def test_targets_that_are_not_tests_are_ignored(self):
        flat = [i for lane in plan_of(BASE + [('yolu-app', 'bench_x', 'example', False), ('yolu-app', 'b', 'bin', False)]) for i in lane]
        self.assertNotIn('bench_x', flat)

    def test_bins_command_selects_all_bins_of_both_packages(self):
        cmd = rt.command('--bins')
        self.assertEqual(cmd[:2], ['cargo', 'test'])
        self.assertIn('--bins', cmd)
        self.assertEqual([c for c in cmd if c == '-p'], ['-p', '-p'])


class Limit(unittest.TestCase):
    def run_lane(self, scripts, seconds, **kwargs):
        """scripts: 列の中で順に回す sh のスクリプト。戻り値は (results, 標準出力, ログ)。"""
        results = []
        out = io.StringIO()
        names = [f'item{i}' for i in range(len(scripts))]
        by_name = dict(zip(names, scripts))
        with tempfile.TemporaryDirectory() as tmp, \
                mock.patch.object(rt, 'command', lambda name: ['sh', '-c', by_name[name]]), \
                contextlib.redirect_stdout(out):
            rt.run_lane(ROOT, 0, names, tmp, results, time.monotonic() + seconds, int(seconds), **kwargs)
            log = (Path(tmp) / 'lane-0.log').read_text(encoding='utf-8')
        return results, out.getvalue(), log

    def test_exit_code_passes_through(self):
        results, _, log = self.run_lane(['echo one', 'exit 3'], 30)
        self.assertEqual([(r[1], r[2]) for r in results], [('item0', 0), ('item1', 3)])
        self.assertIn('one', log)

    def test_timeout_kills_the_whole_process_group_and_shows_the_log_tail(self):
        with tempfile.TemporaryDirectory() as tmp:
            pidfile = Path(tmp) / 'child.pid'
            # sh の子の sleep が cargo の起こした試験の実行ファイルの役。sh だけを殺すと sleep が残る
            script = f'echo "test slow::one ... "; sleep 300 & echo $! > {pidfile}; wait'
            started = time.monotonic()
            results, stdout, log = self.run_lane([script, 'echo MARK$((1+1))'], 1.5)
            elapsed = time.monotonic() - started
            self.assertLess(elapsed, 20, '上限の時間で止まる')
            child = int(pidfile.read_text())
            for _ in range(100):
                if not alive(child):
                    break
                time.sleep(0.05)
            self.assertFalse(alive(child), '孫のプロセスも殺された')
        self.assertEqual([(r[1], r[2]) for r in results], [('item0', rt.TIMED_OUT), ('item1', rt.NOT_RUN)])
        self.assertIn('::error::列 0 の item0', stdout)
        self.assertIn('test slow::one', stdout, '止まった試験の名前がすぐに出る')
        self.assertIn('test slow::one', log)
        self.assertIn('回さない', log)
        self.assertNotIn('MARK2', log, '回さなかった物は実行されない')

    def test_tail_is_limited(self):
        script = 'i=0; while [ $i -lt 100 ]; do echo line$i; i=$((i+1)); done; sleep 300'
        _, stdout, _ = self.run_lane([script], 1.5, tail_lines=5)
        self.assertIn('line99', stdout)
        self.assertNotIn('line90', stdout)


if __name__ == '__main__':
    unittest.main()
