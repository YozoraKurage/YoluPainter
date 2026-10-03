#!/usr/bin/env python3
"""Unity の純 C# PSD 試験が渡す人工データと読み書き結果を採取する。元ソースは読み取り専用。"""
import argparse
import gzip
import hashlib
from pathlib import Path
import shutil
import subprocess

p = argparse.ArgumentParser(description=__doc__)
p.add_argument('--source', type=Path, required=True)
p.add_argument('--unity-data', type=Path, default=Path('/opt/unity/Editor/Data'))
p.add_argument('--nunit', type=Path, default=Path.home() / 'unity-testproject/Library/PackageCache/com.unity.ext.nunit@1.0.6/net35/unity-custom/nunit.framework.dll')
a = p.parse_args()
root = Path(__file__).resolve().parents[2]
build = root / 'target/psd-fixtures'
build.mkdir(parents=True, exist_ok=True)
codec_path = a.source / 'Runtime/Core/Psd/PsdCodec.cs'
codec = codec_path.read_text()
signature = 'public static PsdReadResult Read(byte[] bytes, PsdLimits limits = null)'
assert codec.count(signature) == 1
codec = codec.replace(signature, '''public static PsdReadResult Read(byte[] bytes, PsdLimits limits = null)
        {
            var result = ReadUncaptured(bytes, limits);
            Capture.Save(bytes, limits ?? new PsdLimits(), result);
            return result;
        }
        private static PsdReadResult ReadUncaptured(byte[] bytes, PsdLimits limits = null)''')
(build / 'PsdCodec.cs').write_text(codec)
api = a.unity_data / 'UnityReferenceAssemblies/unity-4.8-api'
lines = ['-nologo', '-target:exe', '-langversion:9.0', '-optimize+', '-nostdlib+', f'-out:"{build / "Capture.exe"}"']
lines += [f'-r:"{x}"' for x in sorted(api.rglob('*.dll'))] + [f'-r:"{a.nunit}"']
sources = sorted((a.source / 'Runtime/Core').rglob('*.cs')) + sorted((a.source / 'Tests/Editor/Psd').glob('*.cs'))
# 試験のGUIDを固定して、同じC#から同じ正解を再生成できるようにする。
for x in sources:
    if x == codec_path:
        continue
    text = x.read_text()
    if 'Guid.NewGuid()' in text:
        copy = build / x.relative_to(a.source)
        copy.parent.mkdir(parents=True, exist_ok=True)
        copy.write_text(text.replace('Guid.NewGuid()', 'global::Capture.NextGuid()'))
        lines.append(f'"{copy}"')
    else:
        lines.append(f'"{x}"')
lines += [f'"{build / "PsdCodec.cs"}"', f'"{Path(__file__).with_name("Capture.cs")}"']
rsp = build / 'compile.rsp'
rsp.write_text('\n'.join(lines) + '\n')
shutil.copyfile(a.nunit, build / 'nunit.framework.dll')
subprocess.run([str(a.unity_data / 'NetCoreRuntime/dotnet'), 'exec', str(a.unity_data / 'DotNetSdkRoslyn/csc.dll'), '/noconfig', '@' + str(rsp)], check=True)
subprocess.run([str(a.unity_data / 'MonoBleedingEdge/bin/mono'), str(build / 'Capture.exe'), str(build / 'corpus.bin')], check=True)
out = root / 'crates/yolu-io/tests/fixtures/psd'
out.mkdir(exist_ok=True)
(out / 'csharp.bin.gz').write_bytes(gzip.compress((build / 'corpus.bin').read_bytes(), mtime=0))
fingerprint = hashlib.sha256(b''.join(x.read_bytes() for x in sources)).hexdigest()
commit = subprocess.check_output(['git', '-C', str(a.source), 'rev-parse', 'HEAD'], text=True).strip()
(out / 'source.txt').write_text(f'Unity コミット: {commit}\nCore・PSD 試験の連結 SHA-256: {fingerprint}\n' + (build / 'corpus.bin.summary').read_text())
print((out / 'source.txt').read_text())
