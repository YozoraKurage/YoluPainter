#!/usr/bin/env python3
"""Unity 同梱の Roslyn / Mono で C# Core の正解データを作る。出力はこの作業木だけ。"""
import argparse
from pathlib import Path
import subprocess
import hashlib
import shutil

parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument('--verify', action='store_true', help='既存の27件の.ylpをC#とRustの例で合成しPNG全バイトを比較')
parser.add_argument('--m1', action='store_true', help='形式7とM1合成の正解データを生成')
parser.add_argument('--m2', action='store_true', help='M2の層の正本5件と全チャンネルの合成を生成')
parser.add_argument('--effects', action='store_true', help='効果（フィルター・Generator・Anchor・塗りつぶしの画像と投影・グラデーション・パス）の正本5件と、版9〜20の旧い正本12件、合成・層の出力・入力を生成')
parser.add_argument('--rust-written-effects', action='store_true',
                    help='Rustが編集APIで作って書いた効果入りの版21の正本（rust-written-effects-v21.utpaint）をUnity版の読み手に読ませ、書き直しの一致・全チャンネルの合成・層ごとの出力を記録')
parser.add_argument('--user-channels', action='store_true',
                    help='Rustが書いた版22の正本（user-channels-v22.utpaint）をUnity版の読み手に読ませた結果を記録')
parser.add_argument('--rust-written', action='store_true',
                    help='Rustが書いた版21の正本（rust-written-v21.utpaint）をUnity版の読み手に読ませ、書き直しの一致と合成を記録')
parser.add_argument('--source', type=Path, required=True, help='Unity 版ソース（読むだけ）')
parser.add_argument('--unity-data', type=Path, default=Path('/opt/unity/Editor/Data'))
args = parser.parse_args()
root = Path(__file__).resolve().parents[2]
out = root / 'target/io-fixtures'
out.mkdir(parents=True, exist_ok=True)
api = args.unity_data / 'UnityReferenceAssemblies/unity-4.8-api'
lines = ['-nologo', '-target:exe', '-langversion:9.0', '-optimize+', '-nostdlib+',
         '-define:UNITY_EDITOR,UNITY_EDITOR_LINUX,UNITY_2022_3_OR_NEWER',
         f'-out:"{out / "Generate.exe"}"']
lines += [f'-r:"{p}"' for p in sorted(api.rglob('*.dll'))]
lines += [f'"{p}"' for p in sorted((args.source / 'Runtime/Core').rglob('*.cs'))]
lines += [f'"{p}"' for p in sorted(Path(__file__).parent.glob('*.cs'))]
response = out / 'compile.rsp'
response.write_text('\n'.join(lines) + '\n')
subprocess.run([str(args.unity_data / 'NetCoreRuntime/dotnet'), 'exec',
                str(args.unity_data / 'DotNetSdkRoslyn/csc.dll'), '/noconfig', '@' + str(response)], check=True)
mono = [str(args.unity_data / 'MonoBleedingEdge/bin/mono'), str(out / 'Generate.exe')]
fixtures = root / 'crates/yolu-io/tests/fixtures'
if args.verify:
    cargo = shutil.which('cargo') or str(Path.home() / '.cargo/bin/cargo')
    subprocess.run([cargo, 'build', '-p', 'yolu-io', '--example', 'composite_png'], cwd=root, check=True)
    comparison = out / 'comparison'
    comparison.mkdir(exist_ok=True)
    for name in [f'm1-mode-{mode:02}' for mode in range(26)] + ['m1-pattern']:
        source = fixtures / (name + '.ylp')
        csharp = comparison / (name + '-csharp.png')
        rust = comparison / (name + '-rust.png')
        rust.unlink(missing_ok=True)  # この道具が作った出力だけを再生成する。
        subprocess.run(mono + ['--composite', str(source), str(csharp)], check=True)
        subprocess.run([str(root / 'target/debug/examples/composite_png'), str(source), str(rust)], cwd=root, check=True)
        actual, expected = rust.read_bytes(), csharp.read_bytes()
        if actual != expected:
            raise SystemExit(f'PNG全バイトが不一致: {name}')
        if expected != (fixtures / (name + '.png')).read_bytes():
            raise SystemExit(f'C#の出力が既存の正解データと不一致: {name}')
        print(f'{name}: {len(actual)} bytes / SHA-256 {hashlib.sha256(actual).hexdigest()}', flush=True)
    print('同じ.ylp 27件のC#・Rust合成PNGは、全ファイルで全バイト一致しました')
elif args.m2:
    subprocess.run(mono + ['--m2', str(fixtures)], check=True)
elif args.effects:
    subprocess.run(mono + ['--effects', str(fixtures)], check=True)
elif args.rust_written_effects:
    subprocess.run(mono + ['--rust-written-effects', str(fixtures / 'rust-written-effects-v21.utpaint'),
                           str(fixtures / 'rust-written-effects-v21.unity.txt'),
                           str(fixtures / 'rust-written-effects-v21.composite'),
                           str(fixtures / 'rust-written-effects-v21.layers')], check=True)
elif args.user_channels:
    subprocess.run(mono + ['--unity-reads', str(fixtures / 'user-channels-v22.utpaint'),
                           str(fixtures / 'user-channels-v22.unity.txt')], check=True)
elif args.rust_written:
    subprocess.run(mono + ['--rust-written', str(fixtures / 'rust-written-v21.utpaint'),
                           str(fixtures / 'rust-written-v21.unity.txt'),
                           str(fixtures / 'rust-written-v21.composite')], check=True)
else:
    subprocess.run(mono + (['--m1'] if args.m1 else []) + [str(fixtures)], check=True)
