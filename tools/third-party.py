#!/usr/bin/env python3
"""Windows 配布物の依存を照合し、許諾一覧と検証済み全文の束を作る。"""
import argparse
from collections import Counter
import hashlib
import json
import os
from pathlib import Path
import re
import subprocess
import sys
import urllib.request

ROOT = Path(__file__).resolve().parents[1]
OUT = ROOT / 'target/third-party'
CONFIG = ROOT / 'tools/licenses-reviewed.json'
TARGET = 'x86_64-pc-windows-gnu'
# Ubuntu は egui 標準書体について承認された例外。
# BSL-1.0 と Hack 書体の Bitstream Vera は 2026-10-04 にユーザーが追加承認。
ALLOWED = {'MIT', 'Apache-2.0', '0BSD', 'BSD-2-Clause', 'BSD-3-Clause', 'Zlib',
           'ISC', 'Unicode-DFS-2016', 'Unicode-3.0', 'OFL-1.1', 'Ubuntu-font-1.0',
           'BSL-1.0', 'Bitstream-Vera'}


def cargo(*args):
    command = os.environ.get('CARGO', str(Path.home() / '.cargo/bin/cargo'))
    return subprocess.check_output([command, *args], cwd=ROOT, text=True)


def digest(data):
    return hashlib.sha256(data).hexdigest()


def dependency_keys(package, edges, offline):
    text = cargo('tree', '--locked', *(['--offline'] if offline else []), '--target', TARGET,
                 '-p', package, '-e', edges, '--prefix', 'none', '--format', '{p}')
    keys = set()
    for line in text.splitlines():
        match = re.match(r'^(\S+) v(\S+)', line)
        if not match:
            raise ValueError(f'依存の行を解釈できません: {line}')
        keys.add('@'.join(match.groups()))
    return keys


def read_source(package, spec, offline):
    if 'path' in spec:
        base = Path(package['manifest_path']).parent.resolve()
        path = (base / spec['path']).resolve()
        if not path.is_relative_to(base):
            raise ValueError('クレート外のパスは使えません')
        data = path.read_bytes()
        origin = 'crate:' + spec['path']
    else:
        origin = spec['url']
        if not re.fullmatch(r'https://raw\.githubusercontent\.com/[^/]+/[^/]+/[0-9a-f]{40}/.+', origin):
            raise ValueError('取得元は上流のコミット固定 URL が必要です')
        path = OUT / 'cache' / spec['sha256']
        if path.exists():
            data = path.read_bytes()
        elif offline:
            raise ValueError('未取得の原文です。初回は --offline を外してください')
        else:
            with urllib.request.urlopen(origin, timeout=30) as response:
                data = response.read()
            if digest(data) != spec['sha256']:
                raise ValueError('取得した原文の SHA-256 が違います')
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_bytes(data)
    if digest(data) != spec['sha256']:
        raise ValueError('原文の SHA-256 が違います')
    return origin, data.decode('utf-8-sig')


def inventory(package, metadata, config, offline):
    keys = dependency_keys(package, 'normal,build', offline)
    normal = dependency_keys(package, 'normal,no-proc-macro', offline)
    packages = {}
    for item in metadata['packages']:
        key = item['name'] + '@' + item['version']
        if key in keys and key in packages:
            raise ValueError('同じ名前・版で取得元が異なる依存は要確認: ' + key)
        packages[key] = item
    records, texts, errors = [], [], []
    for key in sorted(keys):
        p = packages[key]
        if p['id'] in metadata['workspace_members']:
            continue
        record = {'crate': p['name'], 'version': p['version'], 'declared': p['license'],
                  'role': '実行時' if key in normal else 'ビルド・マクロ用',
                  'repository': p['repository'], 'selected': [], 'sources': [], 'issues': []}
        review = config['crates'].get(key)
        if not p['source'] or not p['source'].startswith('registry+'):
            record['issues'].append('未確認の取得元（path/git 依存）')
        if not review or review['declared'] != p['license']:
            record['issues'].append('版または宣言した許諾が未確認')
        else:
            record['selected'] = review['selected']
            record['issues'].extend(review['blocked'])
            if not review['selected']:
                record['issues'].append('選択する許諾がありません')
            for license_id in review['selected']:
                if license_id not in ALLOWED:
                    record['issues'].append('許容外: ' + license_id)
            if not review['files']:
                record['issues'].append('検証済み全文なし')
            for spec in review['files']:
                try:
                    origin, text = read_source(p, spec, offline)
                    record['sources'].append({'origin': origin, 'sha256': spec['sha256']})
                    texts.append(f'\n{"=" * 72}\n{key}\n{origin}\nSHA-256: {spec["sha256"]}\n\n{text}\n')
                except (OSError, ValueError) as exc:
                    record['issues'].append(str(exc))
        errors.extend(key + ': ' + issue for issue in record['issues'])
        records.append(record)
    return records, texts, errors


def markdown(package, records, errors, lock_hash):
    counts = Counter(' AND '.join(r['selected']) or '未確認' for r in records)
    lines = [f'## {package} の依存一覧', '', f'対象: `{TARGET}`、通常の機能。Cargo.lock SHA-256: `{lock_hash}`。', '',
             f'外部クレート {len(records)} 件（同名の別版は別件）。実行時 {sum(r["role"] == "実行時" for r in records)} 件。', '',
             'ビルド用・手続きマクロ用も取りこぼしを避けて全文束に含める。試験用の依存は除く。', '',
             '| 選択した許諾（追加条件を含む） | 件数 |', '|---|---:|']
    lines += [f'| {license_id} | {count} |' for license_id, count in sorted(counts.items())]
    lines += ['', '状態: ' + ('要確認。配布用全文束は生成しない。' if errors else 'クレートの許諾照合は成功。'), '',
              '| クレート | 版 | 用途 | 宣言された許諾 | 選択・追加条件 | 確認 |', '|---|---|---|---|---|---|']
    for r in records:
        lines.append(f'| {r["crate"]} | {r["version"]} | {r["role"]} | {r["declared"]} | {" AND ".join(r["selected"])} | {" / ".join(r["issues"]) or "確認済み"} |')
    return '\n'.join(lines) + '\n'


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--package', choices=['yolu-app', 'yolu-bridge', 'all'], default='all')
    parser.add_argument('--bundle', action='store_true', help='照合成功時だけ配布用 THIRD_PARTY_LICENSES.txt を作る')
    parser.add_argument('--offline', action='store_true', help='取得済みの原文だけを使う')
    args = parser.parse_args()
    OUT.mkdir(parents=True, exist_ok=True)
    selected = ['yolu-app', 'yolu-bridge'] if args.package == 'all' else [args.package]
    # 失敗した今回の結果と、以前の成功した全文束を取り違えない。
    for package in selected:
        (OUT / package / 'THIRD_PARTY_LICENSES.txt').unlink(missing_ok=True)
    config = json.loads(CONFIG.read_text())
    if config['schema'] != 1 or config['target'] != TARGET:
        raise ValueError('設定の版または対象が違います')
    metadata = json.loads(cargo('metadata', '--locked', '--format-version', '1',
                               '--filter-platform', TARGET, *(['--offline'] if args.offline else [])))
    lock_hash = digest((ROOT / 'Cargo.lock').read_bytes())
    failures = 0
    for package in selected:
        records, texts, errors = inventory(package, metadata, config, args.offline)
        directory = OUT / package
        directory.mkdir(exist_ok=True)
        (directory / 'inventory.json').write_text(json.dumps({'target': TARGET, 'package': package,
            'lock_sha256': lock_hash, 'records': records, 'issues': errors}, ensure_ascii=False, indent=2) + '\n')
        (directory / 'THIRD_PARTY.md').write_text(markdown(package, records, errors, lock_hash))
        if errors:
            failures += 1
            print(f'{package}: {len(records)} クレート、要確認', file=sys.stderr)
            for error in errors:
                print('  ' + error, file=sys.stderr)
        else:
            if args.bundle:
                if package == 'yolu-app':
                    texts.append('\n同梱アイコン\n' + (ROOT / 'crates/yolu-app/assets/icons/THIRD-PARTY-NOTICES.md').read_text())
                temporary = directory / 'THIRD_PARTY_LICENSES.txt.tmp'
                temporary.write_text(
                    f'{package} 第三者の許諾全文\n対象: {TARGET}\nCargo.lock SHA-256: {lock_hash}\n'
                    + '\n'.join(texts))
                temporary.replace(directory / 'THIRD_PARTY_LICENSES.txt')
            print(f'{package}: {len(records)} クレート、照合成功')
    print('結果: target/third-party/<クレート>/')
    return 1 if failures else 0


if __name__ == '__main__':
    try:
        sys.exit(main())
    except (OSError, ValueError, subprocess.CalledProcessError) as exc:
        print(f'許諾の照合を完了できません: {exc}', file=sys.stderr)
        sys.exit(1)
