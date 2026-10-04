#!/usr/bin/env python3
"""既存の CPU ベンチを同じ回数・並列数で実行する（Linux）。"""
import argparse
import datetime
import json
import hashlib
import os
from pathlib import Path
import re
import subprocess
import sys

ROOT = Path(__file__).resolve().parents[1]


def parse(log, surface=False):
    rows = {}
    for line in log.splitlines():
        if not surface:
            m = re.fullmatch(r'(.+): 最小 ([\d.]+) ms / 中央 ([\d.]+) ms（(\d+) 回）', line)
            if m:
                label = re.sub(r'（ワーカー \d+）', '', m[1])
                rows[label] = (float(m[3]), 'ms', f'最小 {m[2]} ms、{m[4]} 回')
        else:
            if '三角形: 組み立て' in line:
                m = re.search(r'球 (\d+) 三角形: 組み立て ([\d.]+) ms（隣り合わせ ([\d.]+) ms・BVH ([\d.]+) ms）',line)
                if not m: raise ValueError('面の組み立ての出力形式が変わりました')
                for i, name in enumerate(['組み立て','隣り合わせ','BVH'],2):
                    rows[f'面 {m[1]} 三角形 {name}']=(float(m[i]),'ms','中央値')
            elif 'レイ 1 本' in line:
                m=re.search(r'レイ 1 本 ([\d.]+) µs（(\d+) 本・当たり (\d+)',line)
                if not m: raise ValueError('レイの出力形式が変わりました')
                rows[f'レイ {m[2]} 本・当たり {m[3]}（1 本あたり）']=(float(m[1]),'µs','1 スレッド')
            elif 'ダブ 2048²' in line:
                m=re.search(r'(ダブ 2048² 半径 [\d.]+): ([\d.]+) ms（(\d+) 画素',line)
                if not m: raise ValueError('面のダブの出力形式が変わりました')
                rows[f'面 {m[1]}・{m[3]} 画素']=(float(m[2]),'ms','中央値')
    if not rows: raise ValueError('計測行がありません')
    return rows


def main():
    p=argparse.ArgumentParser(description=__doc__)
    p.add_argument('--runs',type=int,default=5)
    p.add_argument('--threads',type=int,default=1)
    p.add_argument('--source',default=os.environ.get('YOLUPAINTER_UNITY_SOURCE','/workspace'))
    p.add_argument('--out',type=Path,default=ROOT/'target/bench-all')
    p.add_argument('--timeout',type=int,default=1800,help='各コマンドの上限秒')
    p.add_argument('--only',choices=['round','jitter','tip','texture','dual','color','all','blur','smudge'],help='M2 ブラシだけを絞る（合成・通常ブラシ・面は常に測る）')
    a=p.parse_args()
    if min(a.runs,a.threads,a.timeout)<=0: p.error('回数・並列数・制限時間は正の整数が必要')
    out=a.out.resolve()
    if not out.is_relative_to(ROOT/'target'): p.error('出力先はこの worktree の target/ 内にしてください')
    if not hasattr(os,'sched_getaffinity'): p.error('CPU の割当を揃えるため Linux が必要です')
    cpus=sorted(os.sched_getaffinity(0))[:a.threads]
    if len(cpus)!=a.threads: p.error('利用可能 CPU 数を超えています')
    out.mkdir(parents=True,exist_ok=True)
    # 古い成功の表を今回の結果と取り違えない。
    (out/'summary.md').write_text('計測中。結果はまだ確定していません。\n')
    env=os.environ.copy()
    env.update(RAYON_NUM_THREADS=str(a.threads),BENCH_THREADS=str(a.threads),LC_ALL='C.UTF-8',CARGO_TARGET_DIR=str(ROOT/'target'))
    env.pop('BENCH_ONLY',None)
    if a.only: env['BENCH_ONLY']=a.only
    meta=dict(start=datetime.datetime.now(datetime.timezone.utc).isoformat(),runs=a.runs,threads=a.threads,cpus=cpus,
              load_start=os.getloadavg(),revision=subprocess.check_output(['git','rev-parse','HEAD'],cwd=ROOT,text=True).strip(),
              rust=subprocess.check_output(['rustc','-Vv'],text=True).strip(),platform=os.uname().sysname,
              unity_revision=subprocess.check_output(['git','-C',a.source,'rev-parse','HEAD'],text=True).strip(),commands={})
    def source_hashes():
        source=Path(a.source)
        paths=sorted((source/'Runtime/Core').rglob('*.cs'))
        paths += [source/'Editor/Preview'/name for name in ['SurfaceGeometry.cs','SurfaceGeometry.Sampling.cs','SurfaceRegions.cs']]
        return {str(path.relative_to(source)):hashlib.sha256(path.read_bytes()).hexdigest() for path in paths}
    meta['source_hashes']=source_hashes()
    meta['cpu_model']=next((line.split(':',1)[1].strip() for line in Path('/proc/cpuinfo').read_text().splitlines() if line.startswith('model name')),'不明')
    meta['rustflags']={k:os.environ.get(k,'') for k in ['RUSTFLAGS','CARGO_ENCODED_RUSTFLAGS']}
    unity=Path(env.get('YOLUPAINTER_CORE_UNITY_DATA','/opt/unity/Editor/Data'))
    meta['mono']=subprocess.check_output([str(unity/'MonoBleedingEdge/bin/mono'),'--version'],text=True).strip()
    def run(name,cmd,measure=True):
        command=(['taskset','-c',','.join(map(str,cpus))]+cmd) if measure else cmd
        meta['commands'][name]=command
        print(f'実行: {name}',file=sys.stderr,flush=True)
        with (out/f'{name}.log').open('w') as stream:
            subprocess.run(command,cwd=ROOT,env=env,stdout=stream,stderr=subprocess.STDOUT,check=True,timeout=a.timeout)
        return (out/f'{name}.log').read_text()
    try:
        run('build',['cargo','build','--locked','--release','-p','yolu-core','--examples'],False)
        core=['tools/csharp-golden/run.sh','--source',a.source,'--release']
        rust_log=run('rust-core',['target/release/examples/bench',str(a.runs)]+([a.only] if a.only else []))
        csharp_log=run('csharp-core',core+['bench',str(a.runs)])
        if f'rayon のスレッド {a.threads}' not in rust_log or f'CoreParallelism {a.threads}' not in csharp_log:
            raise ValueError('実行時の並列数を確認できません')
        r,c=parse(rust_log),parse(csharp_log)
        if len(r)!=(11 if a.only else 27) or len(c)!=(10 if a.only else 26):
            raise ValueError('計測行が欠けています。ベンチの変更も確認してください')
        rs=parse(run('rust-surface',['target/release/examples/surface_bench',str(a.runs)]),True)
        cs=parse(run('csharp-surface',core+['surface-bench',str(a.runs)]),True)
        if len(rs)!=6 or len(cs)!=6: raise ValueError('面の計測行が欠けています')
        r.update(rs);c.update(cs)
        if source_hashes()!=meta['source_hashes']: raise ValueError('計測中に C# のソースが変更されました')
        # composite_into は C# に対となる実装がない。ほかの行は負荷の個数も一致が必要。
        unmatched=set(r)^set(c)
        if any('composite_into' not in key for key in unmatched): raise ValueError(f'負荷や出力が一致しません: {sorted(unmatched)}')
        lines=['# CPU の速さの比較','',f'採用 {a.runs} 回、並列上限 {a.threads}、CPU 割当 {cpus}。Rust release / C# optimize+・Unity 同梱 Linux Mono。',
               '合成・ブラシは予熱 2 回を除いた中央値。面は予熱なしの中央値。時間にビルドは含まない。',
               '合成は 4096²、ブラシは 101 点。面は合成球 69,312 三角形、レイ 20,000 本、ダブ 2048²。',
               'フィルターは既存ベンチのレベル補正・ぼかし・指先を含む。独立したフィルター全種のベンチと GPU は対象外。',
               'スレッド数は上限。レイは逐次。GC・メモリ確保・言語ごとの実装差を含み、Windows の性能を示すものではない。','',
               '| 負荷 | Rust 中央 | C# 中央 | C#/Rust | 注記（Rust / C#） |','|---|---:|---:|---:|---|']
        for key in dict.fromkeys([*r,*c]):
            rv,cv=r.get(key),c.get(key)
            if rv and cv and rv[1]!=cv[1]: raise ValueError('時間単位が違います')
            def cell(v): return f'{v[0]:.3f} {v[1]}' if v else '対象なし'
            ratio=f'{cv[0]/rv[0]:.2f}' if rv and cv and rv[0]>0 else '—'
            note=' / '.join(v[2] if v else '対になる計測なし' for v in [rv,cv])
            lines.append(f'| {key} | {cell(rv)} | {cell(cv)} | {ratio} | {note} |')
        meta['load_end']=os.getloadavg();meta['status']='成功'
        lines+=['',f'負荷平均（1/5/15 分）: 開始 {meta["load_start"]}、終了 {meta["load_end"]}。共有ホストの他の仕事は止めていない。',
                'コマンド・版・CPU 割当は metadata.json、各実行の原文は同じディレクトリの .log を参照。']
        result='\n'.join(lines)+'\n'
        (out/'summary.md').write_text(result)
        print(result)
    except Exception as e:
        meta['status']='失敗';meta['error']=str(e)
        (out/'summary.md').write_text('計測失敗。metadata.json とログを確認してください。\n')
        raise
    finally:
        (out/'metadata.json').write_text(json.dumps(meta,ensure_ascii=False,indent=2)+'\n')

if __name__=='__main__':
    main()
