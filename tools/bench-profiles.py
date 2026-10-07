#!/usr/bin/env python3
"""配布のビルドの profile（release と dist。dist は lto = "fat"・codegen-units = 1 を --config で与える）を、ビルドの時間・exe の大きさ・重い操作の時間で比べる（Linux）。

  python3 tools/bench-profiles.py [--app] [--cold] [--runs 5] [--rounds 3] [--cpu N] [--clean]

ビルド: sccache なし・インクリメンタルなしで、各 profile の yolu-core の examples と yolu-io の io_bench（保存・PSD。tools/profile-bench/ から一時的に
yolu-io/examples/ へ写してビルドし、終わったら消す）をビルドする。`--app` は yolu-app（exe の大きさと、依存を含む全部のビルド時間）もビルドする。`--cold` はビルドの前に
target/<profile> を消す（ほかの profile・worktree には触れない）。
測る: 1 つの CPU に固定（taskset）・rayon 1 スレッドで、yolu-core の bench（合成・ブラシ）と export_bench（書き出し）、io_bench（保存・PSD）を、
profile を交互に、`--rounds` 回まわす。各行は、回ごとの中央値の中央値。profile ごとの比（release ÷ dist。1 より大きいほど dist が速い）を、操作の種類ごとの
幾何平均と行ごとに出す。機械・負荷・回数を一緒に出すので、数は「その機械・その負荷の範囲」の値として読む。Windows の runner の時間は測れない（比で見積もる）。
"""
import argparse
import math
import os
from pathlib import Path
import re
import shutil
import statistics
import subprocess
import sys
import time

ROOT = Path(__file__).resolve().parents[1]
INJECT = [(ROOT / 'tools/profile-bench/io_bench.rs', ROOT / 'crates/yolu-io/examples/io_bench.rs')]
BENCHES = ['bench', 'export_bench', 'io_bench']
LINE = re.compile(r'^\s*(.+?): 最小 ([\d.]+) ms / 中央 ([\d.]+) ms（(\d+) 回）')
GROUPS = [('合成', ('合成', 'Normal の出力')), ('ブラシ', ('ストローク', 'M2 ')), ('書き出し', ('unity-', 'liltoon/', '覆い', '塗り広げ')),
          ('保存', ('保存', '開く')), ('PSD', ('PSD',))]


def group_of(label):
    for name, prefixes in GROUPS:
        if label.startswith(prefixes):
            return name
    return None


def environment():
    env = os.environ.copy()
    env.update(RUSTC_WRAPPER='', CARGO_INCREMENTAL='0', LC_ALL='C.UTF-8', RAYON_NUM_THREADS='1', BENCH_THREADS='1')
    return env


def timed(command, env):
    start = time.monotonic()
    subprocess.run(command, cwd=ROOT, env=env, check=True, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    return time.monotonic() - start


# 比べる側の `dist`（lto = "fat"・codegen-units = 1。panic は unwind のまま）は Cargo.toml に入れず、--config で与える
# （測った結果は release を超えず、採らなかった。docs/RELEASING.md）。
CANDIDATES = {'dist': ['--config', 'profile.dist.inherits="release"', '--config', 'profile.dist.lto="fat"',
                       '--config', 'profile.dist.codegen-units=1']}


def cargo_profile(profile):
    if profile == 'release':
        return ['--release']
    return ['--profile', profile, *CANDIDATES.get(profile, [])]


def build(profiles, app, cold, env):
    report = {}
    for profile in profiles:
        if cold:
            shutil.rmtree(ROOT / 'target' / profile, ignore_errors=True)
        row = {}
        if app:
            row['app'] = timed(['cargo', 'build', '--locked', '-p', 'yolu-app', *cargo_profile(profile)], env)
            exe = ROOT / 'target' / profile / 'yolupainter'
            row['exe'] = exe.stat().st_size
        row['examples'] = timed(['cargo', 'build', '--locked', '-p', 'yolu-core', '--examples', *cargo_profile(profile)], env)
        row['io'] = timed(['cargo', 'build', '--locked', '-p', 'yolu-io', '--example', 'io_bench', *cargo_profile(profile)], env)
        report[profile] = row
    return report


def run_bench(profile, name, runs, cpu, env):
    binary = ROOT / 'target' / profile / 'examples' / name
    result = subprocess.run(['taskset', '-c', str(cpu), str(binary), str(runs)], cwd=ROOT, env=env, check=True,
                            capture_output=True, text=True, timeout=1800)
    rows = {}
    for line in result.stdout.splitlines():
        match = LINE.match(line)
        if match:
            rows[match[1]] = float(match[3])
    if not rows:
        raise SystemExit(f'{name}: 計測行がありません')
    return rows


def geomean(values):
    return math.exp(sum(math.log(v) for v in values) / len(values))


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument('--profiles', default='release,dist')
    parser.add_argument('--runs', type=int, default=5)
    parser.add_argument('--rounds', type=int, default=3)
    parser.add_argument('--cpu', type=int, default=sorted(os.sched_getaffinity(0))[-1])
    parser.add_argument('--app', action='store_true', help='yolu-app もビルドする（exe の大きさ・全部のビルド時間）')
    parser.add_argument('--cold', action='store_true', help='ビルドの前に target/<profile> を消す')
    parser.add_argument('--no-build', action='store_true')
    parser.add_argument('--clean', action='store_true', help='終わったら target/<profile> を消す（ディスクのため）')
    args = parser.parse_args()
    profiles = args.profiles.split(',')
    if len(profiles) != 2 or min(args.runs, args.rounds) <= 0:
        parser.error('profile は 2 つ、回数は正の整数')
    env = environment()
    for source, destination in INJECT:
        shutil.copyfile(source, destination)
    try:
        built = {} if args.no_build else build(profiles, args.app, args.cold, env)
    finally:
        for _, destination in INJECT:
            destination.unlink(missing_ok=True)
    load_start = os.getloadavg()
    samples = {p: {} for p in profiles}
    for round_ in range(args.rounds):
        order = profiles if round_ % 2 == 0 else profiles[::-1]
        for name in BENCHES:
            for profile in order:
                for label, ms in run_bench(profile, name, args.runs, args.cpu, env).items():
                    samples[profile].setdefault(label, []).append(ms)
                print(f'{round_ + 1}/{args.rounds} {profile} {name}', file=sys.stderr, flush=True)
    base, other = profiles
    medians = {p: {label: statistics.median(v) for label, v in samples[p].items()} for p in profiles}
    labels = [label for label in medians[base] if label in medians[other]]
    cpu_model = next((l.split(':', 1)[1].strip() for l in Path('/proc/cpuinfo').read_text().splitlines() if l.startswith('model name')), '不明')
    rustc = subprocess.check_output(['rustc', '-V'], text=True).strip()
    print(f'# {base} と {other} の比較\n')
    print(f'機械: {cpu_model}（CPU {args.cpu} に固定・rayon 1 スレッド）、{rustc}、負荷平均 開始前 {load_start[0]:.1f} 終了 {os.getloadavg()[0]:.1f}。'
          f'各行は {args.runs} 回の中央値を {args.rounds} 回まわした中央値（profile を交互に）。比 = {base} ÷ {other}（1 より大きいほど {other} が速い）。\n')
    if built:
        print('## ビルド（sccache なし・インクリメンタルなし。この機械の cargo jobs と負荷つき）\n')
        print(f'| profile | yolu-app 全部 | yolu-core examples | io_bench | exe の大きさ |\n|---|---:|---:|---:|---:|')
        for profile in profiles:
            row = built[profile]
            app = f'{row["app"]:.0f} s' if 'app' in row else '—'
            exe = f'{row["exe"] / 1e6:.1f} MB' if 'exe' in row else '—'
            print(f'| {profile} | {app} | {row["examples"]:.0f} s | {row["io"]:.0f} s | {exe} |')
        if args.app:
            print(f'\nビルド時間の比（{other} ÷ {base}）: {built[other]["app"] / built[base]["app"]:.2f}、'
                  f'exe の大きさの比: {built[other]["exe"] / built[base]["exe"]:.3f}')
        print()
    print('## 重い操作の比（操作の種類ごとの幾何平均）\n')
    print('| 種類 | 行数 | 比 |\n|---|---:|---:|')
    for name, _ in GROUPS:
        ratios = [medians[base][l] / medians[other][l] for l in labels if group_of(l) == name]
        if ratios:
            print(f'| {name} | {len(ratios)} | {geomean(ratios):.3f} |')
    everything = [medians[base][l] / medians[other][l] for l in labels if group_of(l)]
    print(f'| 全部 | {len(everything)} | {geomean(everything):.3f} |\n')
    print(f'## 行ごと（ms）\n\n| 操作 | {base} | {other} | 比 |\n|---|---:|---:|---:|')
    for label in labels:
        print(f'| {label} | {medians[base][label]:.2f} | {medians[other][label]:.2f} | {medians[base][label] / medians[other][label]:.3f} |')
    ungrouped = [l for l in labels if not group_of(l)]
    if ungrouped:
        print(f'\n種類に入らない行（幾何平均には入れていない）: {len(ungrouped)} 行')
    if args.clean:
        for profile in profiles:
            shutil.rmtree(ROOT / 'target' / profile, ignore_errors=True)


if __name__ == '__main__':
    sys.exit(main())
