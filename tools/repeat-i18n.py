#!/usr/bin/env python3
"""ライブラリ・実行形式・i18n までの結合試験（`--all` なら yolu-app の全部）を cargo と同じ順で反復する。"""
import argparse
import json
import os
import re
from pathlib import Path
import subprocess
import sys
import time


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--rounds', type=int, default=20)
    parser.add_argument('--out', type=Path, required=True)
    parser.add_argument('--stop-file', type=Path)
    parser.add_argument('--predecessor', action='append', help='前に走らせる結合試験を絞る（既定: i18n より前の全結合試験）')
    parser.add_argument('--all', action='store_true', help='yolu-app の全件（ライブラリ・実行形式・全結合試験・doc）を `cargo test -p yolu-app` と同じ形で反復する')
    parser.add_argument('--test-threads', type=int, help='試験の並列の数（既定は cargo の環境のまま。CI の `--test-threads=1` を外せるかを比べるときに 1 と既定を並べる）')
    parser.add_argument('--keep-going', action='store_true', help='シグナルで落ちた巡回があっても、記録して続ける（既定は最初のシグナルで止める）')
    args = parser.parse_args()
    if args.rounds < 1:
        parser.error('--rounds は 1 以上')
    repo = Path(__file__).resolve().parents[1]
    package = repo / 'crates/yolu-app'
    names = sorted(p.stem for p in (package / 'tests').glob('*.rs') if p.stem < 'i18n')
    if args.all and args.predecessor:
        parser.error('--all と --predecessor は一緒に使えない')
    if args.all:
        names = sorted(p.stem for p in (package / 'tests').glob('*.rs'))
    elif args.predecessor:
        if any(n not in names for n in args.predecessor):
            parser.error('--predecessor は i18n より前の既存の結合試験名')
        names = sorted(set(args.predecessor))
    if not args.all:
        names.append('i18n')
    out = args.out.resolve()
    out.mkdir(parents=True, exist_ok=True)
    env = dict(os.environ, CARGO_INCREMENTAL='0')
    env.setdefault('XDG_RUNTIME_DIR', '/tmp/xdg-' + env.get('USER', 'node'))
    Path(env['XDG_RUNTIME_DIR']).mkdir(parents=True, exist_ok=True)
    command = ['cargo', 'test', '-p', 'yolu-app', '--no-fail-fast']
    prelude = []
    if args.all:
        # 既定の `cargo test -p yolu-app`（ライブラリ・実行形式・全結合試験・doc）と同じ形のまま
        prelude = ['yolu_app', 'yolupainter']
    elif not args.predecessor:
        command += ['--lib', '--bins']
        prelude = ['yolu_app', 'yolupainter']
    if not args.all:
        for name in names:
            command += ['--test', name]
    run_command = command + (['--', f'--test-threads={args.test_threads}'] if args.test_threads else [])
    with (out / 'build.log').open('w') as log:
        build = subprocess.run(command + ['--no-run'], cwd=repo, env=env, stdout=log, stderr=subprocess.STDOUT)
    if build.returncode:
        return build.returncode
    names = prelude + names
    manifest = {'order': names, 'rounds': args.rounds,
                'command': run_command,
                'environment': {k: env.get(k) for k in ['WGPU_BACKEND', 'VK_ICD_FILENAMES', 'NODEVICE_SELECT', 'RUST_TEST_THREADS']}}
    (out / 'manifest.json').write_text(json.dumps(manifest, ensure_ascii=False, indent=2) + '\n')
    print('実行順: ' + ' → '.join(names), flush=True)
    failed = False
    with (out / 'results.jsonl').open('w') as results:
        for round_number in range(1, args.rounds + 1):
            if args.stop_file and args.stop_file.exists():
                print('停止ファイルを検出したため終了', flush=True)
                return 2
            # Cargo 経由にして、.cargo/config.toml の [env] と実行先の設定も本番の試験と揃える。
            log_path = out / f'{round_number:02}.log'
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
