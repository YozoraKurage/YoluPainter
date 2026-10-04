#!/usr/bin/env python3
"""Cargo が今回返した実行ファイルだけを Wine で試し、件数を記録する。"""
import argparse
import json
import os
from pathlib import Path
import re
import shutil
import signal
import subprocess
import sys

ROOT = Path(__file__).resolve().parents[1]
OUT = ROOT / 'target/wine-tests'
TARGET = 'x86_64-pc-windows-gnu'


def run(cmd, log, env, timeout):
    with log.open('w') as stream:
        proc = subprocess.Popen(cmd, cwd=ROOT, env=env, stdout=stream,
                                stderr=subprocess.STDOUT, start_new_session=True)
        try:
            return proc.wait(timeout=timeout)
        except subprocess.TimeoutExpired:
            os.killpg(proc.pid, signal.SIGKILL)
            proc.wait()
            stream.write('\n時間切れ\n')
            return 124


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--compat-bcrypt', action='store_true',
                        help='古い Wine 用の乱数 API 互換 DLL を試験専用に作る')
    parser.add_argument('--timeout', type=int, default=180, help='実行ごとの上限秒数')
    parser.add_argument('--package', action='append', metavar='NAME',
                        help='試すクレートを絞る（何度でも指定できる。既定は全部。例: --package yolu-protocol --package yolu-bridge）')
    args = parser.parse_args()
    if args.timeout <= 0:
        parser.error('--timeout は正の整数が必要')
    for tool in ['cargo', 'wine', 'x86_64-w64-mingw32-gcc']:
        if not shutil.which(tool):
            parser.error(f'{tool} が見つかりません')
    OUT.mkdir(parents=True, exist_ok=True)
    env = os.environ.copy()
    env.update(CARGO_TARGET_DIR=str(ROOT / 'target'), WINEPREFIX=str(OUT / 'prefix'),
               WINEARCH='win64', WINEDEBUG='-all')
    env.pop('DISPLAY', None)
    result = {'target': TARGET, 'compat_bcrypt': args.compat_bcrypt,
              'passed': 0, 'failed': 0, 'ignored': 0, 'excluded': [], 'runs': [],
              'status': '準備中', 'revision': subprocess.check_output(
                  ['git', 'rev-parse', 'HEAD'], cwd=ROOT, text=True).strip()}
    summary = OUT / 'summary.json'

    def save():
        summary.write_text(json.dumps(result, ensure_ascii=False, indent=2) + '\n')

    save()  # 前回の成功結果を今回の結果と取り違えない。
    if args.compat_bcrypt:
        dll_dir = OUT / 'compat'
        dll_dir.mkdir(exist_ok=True)
        subprocess.run(['x86_64-w64-mingw32-gcc', '-shared', '-O2', '-o',
                        str(dll_dir / 'bcryptprimitives.dll'),
                        str(ROOT / 'tools/wine-bcryptprimitives.c'), '-ladvapi32'], check=True)
        env['WINEDLLOVERRIDES'] = 'bcryptprimitives=n,b;' + env.get('WINEDLLOVERRIDES', '')
        env['WINEPATH'] = 'Z:' + str(dll_dir).replace('/', '\\') + ';' + env.get('WINEPATH', '')
    else:
        # 同じ prefix で前回互換モードを使っていても、DLL 自体は prefix に置かない。
        env.pop('WINEPATH', None)
    packages = args.package or ['yolu-core', 'yolu-io', 'yolu-protocol', 'yolu-bridge', 'yolu-app']
    cmd = ['cargo', 'test', '--locked', '--target', TARGET, '--no-run',
           '--lib', '--tests', '--message-format=json']
    for package in packages:
        cmd += ['-p', package]
    print('Windows 向けに試験を組みます。ログ: target/wine-tests/build.log', flush=True)
    # ビルドの診断と Cargo の構造化出力を分離する。
    with (OUT / 'build.jsonl').open('w') as stdout, (OUT / 'build.log').open('w') as stderr:
        code = subprocess.run(cmd, cwd=ROOT, env=env, stdout=stdout, stderr=stderr).returncode
    if code:
        result['status'] = 'ビルド失敗'
        save()
        print((OUT / 'build.log').read_text()[-5000:])
        return 1
    artifacts = {}
    for line in (OUT / 'build.jsonl').read_text().splitlines():
        item = json.loads(line)
        if item.get('reason') == 'compiler-artifact' and item.get('executable') and item['profile']['test']:
            artifacts[item['executable']] = item
    if not artifacts:
        raise RuntimeError('試験の実行ファイルがありません')
    errors = 0
    for executable, item in sorted(artifacts.items()):
        name = item['target']['name']
        is_app = Path(item['manifest_path']).parent.name == 'yolu-app'
        if 'bin' in item['target']['kind']:
            result['excluded'].append({'target': name, 'reason': 'アプリ起動用バイナリの試験ハーネス'})
            continue
        filters = [None]
        if is_app and 'test' in item['target']['kind']:
            log = OUT / f'{name}-list.log'
            code = run(['wine', executable, '--list', '--format=terse'], log, env, args.timeout)
            if code:
                errors += 1
                result['runs'].append({'target': name, 'phase': '列挙', 'exit_code': code})
                save()
                continue
            names = [line[:-6] for line in log.read_text(errors='replace').splitlines() if line.endswith(': test')]
            filters = [n for n in names if n.split('::')[-1].startswith('headless_')]
            result['excluded'].extend({'target': name, 'test': n, 'reason':
                'wgpu/egui_kittest の描画試験。Wine の描画バックエンドでは検証しない'}
                for n in names if n not in filters)
        for index, test in enumerate(filters):
            log = OUT / f'{name}-{index}.log'
            command = ['wine', executable, '--test-threads=1', '--color=never']
            if test:
                command += ['--exact', test]
            code = run(command, log, env, args.timeout)
            content = log.read_text(errors='replace')
            totals = re.findall(r'^test result: (?:ok|FAILED)\. (\d+) passed; (\d+) failed; (\d+) ignored;', content, re.M)
            counts = tuple(map(int, totals[-1])) if totals else (0, 0, 0)
            valid = bool(totals) and (not test or sum(counts) == 1)
            if code or not valid or counts[1]:
                errors += 1
            for key, count in zip(['passed', 'failed', 'ignored'], counts):
                result[key] += count
            result['runs'].append({'target': name, 'test': test, 'exit_code': code,
                                   'summary_found': valid, 'passed': counts[0],
                                   'failed': counts[1], 'ignored': counts[2], 'log': log.name})
            print(f'{name} {test or "全件"}: 成功 {counts[0]} / 失敗 {counts[1]} / 無視 {counts[2]} (終了 {code})', flush=True)
            save()
    result['status'] = '成功' if not errors and result['passed'] else '失敗'
    result['errors'] = errors
    save()
    print(f"合計: 成功 {result['passed']} / 失敗 {result['failed']} / 無視 {result['ignored']}、実行エラー {errors}")
    print('詳細: target/wine-tests/summary.json')
    return 0 if result['status'] == '成功' else 1


if __name__ == '__main__':
    try:
        sys.exit(main())
    except (OSError, ValueError, RuntimeError, subprocess.CalledProcessError) as exc:
        print(f'試験を完了できません: {exc}', file=sys.stderr)
        sys.exit(1)
