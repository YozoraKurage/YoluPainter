#!/usr/bin/env python3
"""Unity 同梱の Roslyn / Mono で C# Core の正解データを作る。出力はこの作業木だけ。"""
import argparse
from pathlib import Path
import subprocess

parser = argparse.ArgumentParser(description=__doc__)
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
lines += [f'"{Path(__file__).with_name("Generate.cs")}"']
response = out / 'compile.rsp'
response.write_text('\n'.join(lines) + '\n')
subprocess.run([str(args.unity_data / 'NetCoreRuntime/dotnet'), 'exec',
                str(args.unity_data / 'DotNetSdkRoslyn/csc.dll'), '/noconfig', '@' + str(response)], check=True)
subprocess.run([str(args.unity_data / 'MonoBleedingEdge/bin/mono'), str(out / 'Generate.exe'),
                str(root / 'crates/yolu-io/tests/fixtures')], check=True)
