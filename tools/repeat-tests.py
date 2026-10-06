#!/usr/bin/env python3
"""試験の束（`tests/<束>/main.rs`）を何度も続けて回し、落ちた回・シグナルで止まった回を調べる（lavapipe の中で落ちる揺れの確かめ用）。

例: 画面の束を 20 回続ける
    tools/repeat-tests.py --rounds 20 --out /tmp/repeat --test gui_canvas --test gui_shell --test gui_view3d
`--shuffle` を付けると、回ごとに違う種で試験の順を混ぜる（試験どうしが順序に頼っていないかの確かめ。libtest の不安定な機能なので、
`RUSTC_BOOTSTRAP` は試験の実行のときだけ付ける。組み直しにはならない）。
`--test` を省くと、yolu-app の `tests/` の全部（束と、直下の 1 ファイル 1 本の実行ファイル）。
各回は `cargo test -p <クレート> --test …` と同じ形で回す（`.cargo/config.toml` の環境と実行先の設定が本番の試験と揃う）。
"""
import argparse
import json
import os
import re
import stat
import subprocess
import sys
import time
from pathlib import Path


def targets(package: Path):
    """tests/ の試験の実行ファイルの名前（束のフォルダ `<名前>/main.rs` と、直下の `<名前>.rs`）。"""
    tests = package / 'tests'
    names = {p.stem for p in tests.glob('*.rs')}
    names |= {p.parent.name for p in tests.glob('*/main.rs')}
    return sorted(names)


def host_triple() -> str:
    out = subprocess.run(['rustc', '-vV'], capture_output=True, text=True, check=True).stdout
    return re.search(r'^host: (\S+)', out, re.M).group(1)


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument('--rounds', type=int, default=20)
    parser.add_argument('--out', type=Path, required=True)
    parser.add_argument('--package', default='yolu-app')
    parser.add_argument('--test', action='append', help='回す試験の実行ファイル（複数可。省略は全部）')
    parser.add_argument('--stop-file', type=Path)
    parser.add_argument('--test-threads', type=int, help='試験の並列の数（既定は cargo の環境のまま）')
    parser.add_argument('--shuffle', action='store_true', help='回ごとに違う種で試験の順を混ぜる')
    parser.add_argument('--keep-going', action='store_true', help='シグナルで落ちた巡回があっても、記録して続ける（既定は最初のシグナルで止める）')
    args = parser.parse_args()
    if args.rounds < 1:
        parser.error('--rounds は 1 以上')
    repo = Path(__file__).resolve().parents[1]
    package = repo / 'crates' / args.package
    available = targets(package)
    names = args.test or available
    unknown = [n for n in names if n not in available]
    if unknown:
        parser.error(f'{args.package} に無い試験の実行ファイル: {unknown}（ある物: {available}）')
    out = args.out.resolve()
    out.mkdir(parents=True, exist_ok=True)
    env = dict(os.environ, CARGO_INCREMENTAL='0')
    env.setdefault('XDG_RUNTIME_DIR', '/tmp/xdg-' + env.get('USER', 'node'))
    Path(env['XDG_RUNTIME_DIR']).mkdir(parents=True, exist_ok=True)
    if args.shuffle:
        # 実行のときだけ RUSTC_BOOTSTRAP を付ける実行先（cargo が rustc にも同じ環境を渡すので、環境変数で直に付けると依存まで組み直しになる）
        runner = out / 'shuffle-runner.sh'
        runner.write_text('#!/bin/sh\nexe="$1"; shift\nRUSTC_BOOTSTRAP=1 exec "$exe" "$@" -Zunstable-options --shuffle --shuffle-seed "$YOLU_SHUFFLE_SEED"\n')
        runner.chmod(runner.stat().st_mode | stat.S_IXUSR)
        env['CARGO_TARGET_' + host_triple().upper().replace('-', '_') + '_RUNNER'] = str(runner)
    command = ['cargo', 'test', '-p', args.package, '--no-fail-fast']
    for name in names:
        command += ['--test', name]
    run_command = command + (['--', f'--test-threads={args.test_threads}'] if args.test_threads else [])
    with (out / 'build.log').open('w') as log:
        build = subprocess.run(command + ['--no-run'], cwd=repo, env=env, stdout=log, stderr=subprocess.STDOUT)
    if build.returncode:
        return build.returncode
    manifest = {'tests': names, 'rounds': args.rounds, 'command': run_command, 'shuffle': args.shuffle,
                'environment': {k: env.get(k) for k in ['WGPU_BACKEND', 'VK_ICD_FILENAMES', 'NODEVICE_SELECT', 'RUST_TEST_THREADS']}}
    (out / 'manifest.json').write_text(json.dumps(manifest, ensure_ascii=False, indent=2) + '\n')
    print('実行: ' + ' → '.join(names), flush=True)
    failed = False
    with (out / 'results.jsonl').open('w') as results:
        for round_number in range(1, args.rounds + 1):
            if args.stop_file and args.stop_file.exists():
                print('停止ファイルを検出したため終了', flush=True)
                return 2
            log_path = out / f'{round_number:02}.log'
            if args.shuffle:
                env['YOLU_SHUFFLE_SEED'] = str(round_number)
            started = time.monotonic()
            with log_path.open('w') as log:
                run = subprocess.run(run_command, cwd=repo, env=env, stdout=log, stderr=subprocess.STDOUT)
            seconds = round(time.monotonic() - started)
            output = log_path.read_text(errors='replace')
            counts = [tuple(map(int, match)) for match in re.findall(
                r'(\d+) passed; (\d+) failed; (\d+) ignored', output)]
            record = {'round': round_number, 'returncode': run.returncode, 'seconds': seconds,
                      'passed': sum(c[0] for c in counts), 'failed': sum(c[1] for c in counts),
                      'ignored': sum(c[2] for c in counts)}
            if args.shuffle:
                record['seed'] = round_number
            results.write(json.dumps(record) + '\n')
            results.flush()
            print(f'{round_number}/{args.rounds}: {record}', flush=True)
            failed |= run.returncode != 0
            if run.returncode < 0 or re.search(r'signal: \d+', output):
                if not args.keep_going:
                    return 1
    return int(failed)


if __name__ == '__main__':
    sys.exit(main())
