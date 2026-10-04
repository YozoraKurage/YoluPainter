#!/usr/bin/env python3
"""Linux C# / Linux Rust / Wine Rust を同じ入力で比較。生成物は target/determinism/。"""
import json
import math
import os
from pathlib import Path
import subprocess
import sys
import struct
sys.dont_write_bytecode = True
import compare

ROOT=Path(__file__).resolve().parents[2]
OUT=ROOT/'target/determinism'
UNITY=Path(os.environ.get('YOLUPAINTER_CORE_UNITY_DATA','/opt/unity/Editor/Data'))

def main():
    os.chdir(ROOT); OUT.mkdir(parents=True,exist_ok=True)
    env=os.environ.copy()
    env['PATH']=str(Path.home()/'.cargo/bin')+os.pathsep+env['PATH']
    def run(cmd,inp=None,out=None,extra=None):
        with (OUT/(out or 'build.log')).open('w') as dst:
            if inp:
                with (OUT/inp).open() as src:
                    subprocess.run(cmd,env=env| (extra or {}),stdin=src,stdout=dst,check=True)
            else: subprocess.run(cmd,env=env,stdout=dst,stderr=subprocess.STDOUT,check=True)
    compare.inputs(OUT/'inputs.txt')
    run(['rustc','-O','tools/determinism/probe.rs','-o',str(OUT/'probe')])
    run(['rustc','-O','--target','x86_64-pc-windows-gnu','tools/determinism/probe.rs','-o',str(OUT/'probe.exe')],out='build-windows.log')
    refs=sorted((UNITY/'UnityReferenceAssemblies/unity-4.8-api').glob('*.dll'))
    (OUT/'build.rsp').write_text('\n'.join(['-nologo','-target:exe','-optimize+','-nostdlib+',f'-out:"{OUT}/probe-csharp.exe"']+[f'-r:"{p}"' for p in refs]+['tools/determinism/Probe.cs']))
    run([str(UNITY/'NetCoreRuntime/dotnet'),'exec',str(UNITY/'DotNetSdkRoslyn/csc.dll'),'/noconfig','@'+str(OUT/'build.rsp')],out='build-csharp.log')
    wine=dict(WINEPREFIX=str(ROOT/'target/wine-tests/prefix'),WINEDEBUG='-all',WINEDLLOVERRIDES='bcryptprimitives=n,b',WINEPATH='Z:'+str(ROOT/'target/wine-tests/compat').replace('/','\\'))
    if not (ROOT/'target/wine-tests/compat/bcryptprimitives.dll').exists():
        raise RuntimeError('先に tools/wine-tests.sh --compat-bcrypt を実行してください')
    commands=[('csharp-linux',[str(UNITY/'MonoBleedingEdge/bin/mono'),str(OUT/'probe-csharp.exe')],{}),('rust-linux',[str(OUT/'probe')],{}),('rust-wine',['wine',str(OUT/'probe.exe')],wine)]
    for name,cmd,extra in commands: run(cmd,'inputs.txt',name+'.txt',extra)
    # 同じ tan の出力から、sqrt と atan に渡る引数を固定して分離する。
    def value(h): return struct.unpack('>d',bytes.fromhex(h))[0]
    def bits(x): return struct.pack('>d',x).hex()
    pairs={}
    for line in (OUT/'csharp-linux.txt').read_text().splitlines():
        t=line.split(); label,i=t[0].split(':')
        if label in ('tilt_x','tilt_y'): pairs.setdefault(int(i),{})[label]=value(t[4])
    with (OUT/'stages-input.txt').open('w') as f:
        for i,pair in pairs.items():
            x,y=pair['tilt_x'],pair['tilt_y']; square=x*x+y*y
            for op,v,w in [('sqrt',square,0.0),('atan',math.sqrt(square),0.0),('atan2',y,x)]:
                f.write(f'stage:{i} {op} {bits(v)} {bits(w)}\n')
    for name,cmd,extra in commands: run(cmd,'stages-input.txt',name+'-stages.txt',extra)
    for name in ['rust-linux','rust-wine']:
        compare.compare(OUT/'csharp-linux.txt',OUT/(name+'.txt'),OUT/name)
        compare.compare(OUT/'csharp-linux-stages.txt',OUT/(name+'-stages.txt'),OUT/(name+'-stages'))
    versions={name:subprocess.check_output(cmd,env=env,text=True,stderr=subprocess.STDOUT).strip() for name,cmd in [('rust',['rustc','-Vv']),('wine',['wine','--version']),('mono',[str(UNITY/'MonoBleedingEdge/bin/mono'),'--version'])]}
    versions['revision']=subprocess.check_output(['git','rev-parse','HEAD'],text=True).strip()
    versions['notes']='Math プローブ。Windows 実機の Mono/.NET/MathF は未測定。'
    (OUT/'environment.json').write_text(json.dumps(versions,ensure_ascii=False,indent=2)+'\n')

if __name__=='__main__': main()
