#!/usr/bin/env python3
"""yolu-gpu・yolu-app の試験（描画するもの）を、試験の実行ファイルごとの別のプロセスで並べて回す。

1 つのプロセスの中は `--test-threads=1`（窓・GPU の装置を作る試験は同じプロセスで同時に作ると lavapipe の中で落ちることがあったので、
貸し出しで 1 つずつにしている。描かない試験も混ざるので、プロセスの中は前と同じ 1 本ずつ）。プロセスどうしは別の装置なので並べてよい。
並べ方: 長い 3 つの束（gui_view3d・gui_canvas・gui_shell）がそれぞれ 1 本の列の先頭で、残りの試験の実行ファイル（`cargo metadata` から
数える。新しく足した物も漏れない）と単体試験・ドキュメントの試験は、列の決まった所に入る。列の中は順に回す。

先に `cargo test --no-run` でまとめてビルドし（ビルドが落ちたらそこで終わる）、それから列を回す。cargo は試験を走らせる間はビルドの
ロックを持たないので、列どうしは待たない。列のログは終わってから 1 本ずつ出す。どれかが落ちても全部を回し、最後に落ちた物を並べて
終了コード 1 にする。

    tools/render-tests.py --log-dir /tmp/render-logs              # CI と同じ（3 列）
    tools/render-tests.py --log-dir /tmp/render-logs --lanes 1    # 1 本ずつ（前の形）
"""
import argparse
import json
from pathlib import Path
import subprocess
import sys
import threading
import time

PACKAGES = ['yolu-gpu', 'yolu-app']
# 列の先頭。CI の 1 回の実行で gui_view3d 187 秒・gui_canvas 111 秒・gui_shell 103 秒、ほかは合わせて 80 秒ほど。4 コアに絞った台で
# 3 列が最も短かった（gui_view3d を絞り込みで 2 つのプロセスに分けた 4 列は、全部の列が CPU を取り合って長くなった）。
HEADS = ['gui_view3d', 'gui_canvas', 'gui_shell']
# 先頭の後ろに置く物（列の番号）。ここに無い物は最後の列へ。`--lib`・`--doc` は 2 つのクレートの分をまとめて回す。
AFTER = {'headless': 1, '--lib': 1}
TEST_ARGS = ['--', '--nocapture', '--test-threads=1']
PACKAGE_ARGS = [a for p in PACKAGES for a in ('-p', p)]


def test_targets(root):
    meta = json.loads(subprocess.check_output(
        ['cargo', 'metadata', '--locked', '--no-deps', '--format-version', '1'], cwd=root, text=True))
    found = []
    for package in meta['packages']:
        if package['name'] not in PACKAGES:
            continue
        for target in package['targets']:
            if 'test' in target['kind']:
                found.append(target['name'])
    if len(set(found)) != len(found):
        raise SystemExit(f'2 つのクレートに同じ名前の試験がある（--test で選び分けられない）: {sorted(found)}')
    return sorted(found)


def plan(root, lanes):
    """列ごとの「試験の実行ファイルの名前か --lib・--doc」の並び。"""
    items = test_targets(root) + ['--lib', '--doc']
    for name in HEADS:
        if name not in items:
            raise SystemExit(f'束が見つからない: {name}')
    out = [[] for _ in range(lanes)]
    for i, name in enumerate(HEADS):
        out[min(i, lanes - 1)].append(name)
    rest = [name for name in items if name not in HEADS and name not in AFTER]
    for name in items:
        if name in AFTER:
            out[min(AFTER[name], lanes - 1)].append(name)
    out[lanes - 1].extend(rest)
    return out


def command(name):
    # どの回も 2 つのクレートを選ぶ。1 つだけを選ぶと依存の機能の合わせ方が変わり、依存からビルドし直す
    selector = [name] if name.startswith('--') else ['--test', name]
    return ['cargo', 'test', *PACKAGE_ARGS, '--locked', *selector, *TEST_ARGS]


def run_lane(root, index, items, log_dir, results):
    log = Path(log_dir) / f'lane-{index}.log'
    with log.open('w', encoding='utf-8') as out:
        for item in items:
            cmd = command(item)
            out.write(f'\n===== {" ".join(cmd)}\n')
            out.flush()
            started = time.monotonic()
            code = subprocess.call(cmd, cwd=root, stdout=out, stderr=subprocess.STDOUT)
            results.append((index, item, code, time.monotonic() - started))


def main():
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument('--log-dir', required=True, help='列ごとのログの置き場')
    parser.add_argument('--lanes', type=int, default=3, help='並べるプロセスの数（1 で前と同じ 1 本ずつ）')
    parser.add_argument('--plan', action='store_true', help='並べ方だけを出して終わる')
    args = parser.parse_args()
    root = Path(__file__).resolve().parents[1]
    lanes = plan(root, max(1, args.lanes))
    for i, items in enumerate(lanes):
        print(f'列 {i}: ' + '、'.join(items))
    if args.plan:
        return 0
    sys.stdout.flush()
    build = subprocess.call(['cargo', 'test', *PACKAGE_ARGS, '--locked', '--no-run'], cwd=root)
    if build != 0:
        return build
    Path(args.log_dir).mkdir(parents=True, exist_ok=True)
    results = []
    started = time.monotonic()
    threads = [threading.Thread(target=run_lane, args=(root, i, items, args.log_dir, results))
               for i, items in enumerate(lanes)]
    for t in threads:
        t.start()
    for t in threads:
        t.join()
    wall = time.monotonic() - started
    for i in range(len(lanes)):
        print(f'::group::列 {i} のログ')
        sys.stdout.write((Path(args.log_dir) / f'lane-{i}.log').read_text(encoding='utf-8', errors='replace'))
        print('::endgroup::')
    print(f'\n実行の時間（並べた全体 {wall:.0f} 秒）:')
    for lane, name, code, seconds in sorted(results):
        print(f'  列 {lane}  {seconds:6.1f} 秒  {"ok" if code == 0 else f"失敗（{code}）"}  {name}')
    failed = [name for _, name, code, _ in results if code != 0]
    if failed:
        print('落ちた: ' + '、'.join(failed), file=sys.stderr)
        return 1
    return 0


if __name__ == '__main__':
    sys.exit(main())
