#!/usr/bin/env python3
"""対象ごとの配布物の依存を照合し、許諾一覧と検証済み全文の束を作る。"""
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
# 1 つの配布物を、複数の Rust のターゲットで作った実行ファイルをまとめて作る対象。universal-apple-darwin は Apple Silicon と Intel の
# 実行ファイルを 1 つにした macOS の .app なので、依存は 2 つのターゲットの木の和を数える。
COMPONENTS = {'universal-apple-darwin': ('aarch64-apple-darwin', 'x86_64-apple-darwin')}
# Ubuntu は egui 標準フォントについて承認された例外。
# BSL-1.0 と Hack フォントの Bitstream Vera は 2026-10-04 にユーザーが追加承認。
ALLOWED = {'MIT', 'Apache-2.0', '0BSD', 'BSD-2-Clause', 'BSD-3-Clause', 'Zlib',
           'ISC', 'Unicode-DFS-2016', 'Unicode-3.0', 'OFL-1.1', 'Ubuntu-font-1.0',
           'BSL-1.0', 'Bitstream-Vera'}


def cargo(*args):
    command = os.environ.get('CARGO', str(Path.home() / '.cargo/bin/cargo'))
    return subprocess.check_output([command, *args], cwd=ROOT, text=True, encoding='utf-8')


def digest(data):
    return hashlib.sha256(data).hexdigest()


def triples(target):
    """配布の対象を作る Rust のターゲット（1 つの対象は自分自身、まとめて作る対象は構成するターゲット）。"""
    return COMPONENTS.get(target, (target,))


def dependency_keys(package, edges, offline, built_with=None):
    """`package` が入れる依存の集合。`built_with` を指すと、その製品と同じ cargo のビルド（機能が合わさる）で作った物の、`package` の部分木。
    複数のターゲットで作る対象（`COMPONENTS`）は、ターゲットごとの木の和。"""
    keys = set()
    for triple in triples(TARGET):
        keys |= target_keys(package, edges, offline, built_with, triple)
    return keys


def target_keys(package, edges, offline, built_with, triple):
    if built_with:
        text = cargo('tree', '--locked', *(['--offline'] if offline else []), '--target', triple,
                     '-p', built_with, '-p', package, '-e', edges, '--prefix', 'depth', '--no-dedupe', '--format', '{p}')
        return subtree_keys(text, package)
    text = cargo('tree', '--locked', *(['--offline'] if offline else []), '--target', triple,
                 '-p', package, '-e', edges, '--prefix', 'none', '--format', '{p}')
    keys = set()
    for line in text.splitlines():
        match = re.match(r'^(\S+) v(\S+)', line)
        if not match:
            raise ValueError(f'依存の行を解釈できません: {line}')
        keys.add('@'.join(match.groups()))
    return keys


def subtree_keys(text, package):
    """`cargo tree --prefix depth --no-dedupe` の出力（根ごとの木が空行で区切られる）から、根 `package` の木に載るクレートを取る。"""
    keys = set()
    inside = False
    found = False
    for line in text.splitlines():
        if not line.strip():
            continue
        match = re.match(r'^(\d+)(\S+) v(\S+)', line)
        if not match:
            raise ValueError(f'依存の行を解釈できません: {line}')
        depth, name, version = match.groups()
        if depth == '0':
            inside = name == package
            found = found or inside
        if inside:
            keys.add(f'{name}@{version}')
    if not found:
        raise ValueError(f'{package} の木が出力にありません')
    return keys


# 上流が許諾の本文を持たないクレートのために、上流の記載（Cargo.toml の license と authors）から組み立てた文を置くフォルダ（リポジトリの中）。
# 文の頭に、上流の原文ではないことを書く。`licenses-reviewed.json` の `repo` の指定は、このフォルダの下のファイルだけ。
LICENSE_TEXTS = 'tools/license-texts'


def read_source(package, spec, offline):
    if 'repo' in spec:
        folder = (ROOT / LICENSE_TEXTS).resolve()
        path = (ROOT / spec['repo']).resolve()
        if not path.is_relative_to(folder):
            raise ValueError(f'リポジトリの文は {LICENSE_TEXTS}/ の下だけです')
        data = path.read_bytes()
        origin = 'repo:' + spec['repo']
    elif 'path' in spec:
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


def bundled_assets(package, config):
    """クレートでない同梱物と表記を SHA-256 で照合し、全文束に加える。"""
    texts, errors = [], []
    for item in config.get('bundled', {}).get(package, []):
        name = f"{item['name']} {item['version']}"
        if not item['selected'] or not item['files']:
            errors.append(f'{name}: 許諾または同梱ファイルの登録がありません')
        for pattern in item.get('file_globs', []):
            actual = {p.relative_to(ROOT).as_posix() for p in ROOT.glob(pattern) if p.is_file()}
            reviewed = {spec['path'] for spec in item['files']}
            for path in sorted(actual - reviewed):
                errors.append(f'{name}: 未登録の同梱ファイル: {path}')
        verified = {}
        for license_id in item['selected']:
            # CC0 は既存の Krita 筆先で使用。クレートの許容一覧には加えない。
            if license_id not in ALLOWED | {'CC0-1.0'}:
                errors.append(f'{name}: 許容外: {license_id}')
        for spec in [*item['files'], item['license_file']]:
            path = (ROOT / spec['path']).resolve()
            if not path.is_relative_to(ROOT):
                errors.append(f"{name}: リポジトリ外のパスは使えません: {spec['path']}")
                continue
            try:
                data = path.read_bytes()
            except OSError as exc:
                errors.append(f"{name}: {spec['path']} を読めません: {exc}")
                continue
            if digest(data) != spec['sha256']:
                errors.append(f"{name}: {spec['path']} の SHA-256 が違います（確認済みの内容から変わっています）")
            else:
                verified[spec['path']] = data
        license_file = item['license_file']
        if license_file['path'] in verified:
            try:
                text = verified[license_file['path']].decode('utf-8-sig')
            except UnicodeDecodeError:
                errors.append(f'{name}: 許諾の文を UTF-8 として読めません')
                continue
            origin = item.get('origin') or f"{item['repository']}/tree/{item['commit']}"
            texts.append(f'\n{"=" * 72}\n{name}（同梱物。{", ".join(item["selected"])}）\n{origin}\n'
                         f'SHA-256: {license_file["sha256"]}\n\n{text}\n')
    return texts, errors


def inventory(package, metadata, config, offline, include_update=False, include_cli=False, built_with=None):
    keys = dependency_keys(package, 'normal,build', offline, built_with)
    normal = dependency_keys(package, 'normal,no-proc-macro', offline, built_with)
    # 同じ配布物に入る別の製品（自動更新の yolu-update・コマンドラインと MCP サーバーの yolu-cli）の依存も、同じ照合・同じ全文束に入れる
    for included, wanted in (('yolu-update', include_update), ('yolu-cli', include_cli)):
        if wanted:
            keys |= dependency_keys(included, 'normal,build', offline)
            normal |= dependency_keys(included, 'normal,no-proc-macro', offline)
    records, texts, errors = review_packages(keys, normal, metadata, config, offline)
    bundled_texts, bundled_errors = bundled_assets(package, config)
    return records, texts + bundled_texts, errors + bundled_errors


def review_packages(keys, normal, metadata, config, offline):
    """指定した集合の宣言・選択条件・原文を同じ規則で照合する。"""
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


def audit_lock(metadata, config, offline):
    """lock 全件を、対象の通常・ビルド依存、試験依存、対象外に分類する。"""
    targets = config['targets']
    runtime, development = set(), set()
    membership = {}
    for target in targets:
        membership[target] = {}
        for role, edges in [('通常・ビルド', 'normal,build'), ('試験込み', 'normal,build,dev')]:
            output = cargo('tree', '--locked', *(['--offline'] if offline else []),
                           '--workspace', '--target', target, '-e', edges,
                           '--prefix', 'none', '--format', '{p}')
            keys = set()
            for line in output.splitlines():
                if not line.strip():
                    continue
                match = re.match(r'^(\S+) v(\S+)', line)
                if not match:
                    raise ValueError(f'依存の行を解釈できません: {line}')
                keys.add('@'.join(match.groups()))
            membership[target][role] = keys
            (runtime if role == '通常・ビルド' else development).update(keys)
    records, _, errors = review_packages(development, runtime, metadata, config, offline)
    reviewed = {r['crate'] + '@' + r['version']: r for r in records}
    all_keys = set()
    for p in metadata['packages']:
        if p['id'] in metadata['workspace_members']:
            continue
        key = p['name'] + '@' + p['version']
        if key in all_keys:
            raise ValueError('同じ名前・版で取得元が異なる依存は要確認: ' + key)
        all_keys.add(key)
        if key not in reviewed:
            reviewed[key] = {'crate': p['name'], 'version': p['version'], 'declared': p['license'],
                             'selected': [], 'issues': [], 'role': '対象外（未承認）'}
        r = reviewed[key]
        r['scope'] = '通常・ビルド' if key in runtime else '試験のみ' if key in development else '対象外'
        r['targets'] = [t for t in targets if key in membership[t]['試験込み']]
    # metadata の対象外が黙って欠落したときも、lock 全件を確認した扱いにしない。
    lock_keys = set()
    for block in (ROOT / 'Cargo.lock').read_text(encoding='utf-8').split('[[package]]')[1:]:
        if re.search(r'^source = ', block, re.M):
            name = re.search(r'^name = "([^"]+)"', block, re.M)[1]
            version = re.search(r'^version = "([^"]+)"', block, re.M)[1]
            lock_keys.add(name + '@' + version)
    if lock_keys != all_keys:
        errors.append('Cargo.lock と metadata の外部クレート集合が一致しません')
    stale = sorted(config['crates'].keys() - all_keys)
    errors.extend('Cargo.lock に無い古い登録: ' + key for key in stale)
    report = {'lock_sha256': digest((ROOT / 'Cargo.lock').read_bytes()), 'targets': targets,
              'records': [reviewed[key] for key in sorted(reviewed)], 'stale': stale, 'issues': errors}
    OUT.mkdir(parents=True, exist_ok=True)
    (OUT / 'lock-inventory.json').write_text(json.dumps(report, ensure_ascii=False, indent=2) + '\n', encoding='utf-8')
    print('lock 全体: ' + str(len(all_keys)) + ' 件、' + str(dict(Counter(r['scope'] for r in reviewed.values()))))
    for error in errors:
        print('  ' + error, file=sys.stderr)
    return 1 if errors else 0


def merged_metadata(offline):
    """対象を作る Rust のターゲットごとの `cargo metadata`（そのターゲットの依存だけに絞った物）を 1 つにまとめる。"""
    merged = None
    for triple in triples(TARGET):
        metadata = json.loads(cargo('metadata', '--locked', '--format-version', '1',
                                    '--filter-platform', triple, *(['--offline'] if offline else [])))
        if merged is None:
            merged = metadata
            continue
        known = {item['id'] for item in merged['packages']}
        merged['packages'] += [item for item in metadata['packages'] if item['id'] not in known]
    return merged


def markdown(package, records, errors, lock_hash, built_with=None):
    counts = Counter(' AND '.join(r['selected']) or '未確認' for r in records)
    lines = [f'## {package} の依存一覧', '', f'対象: `{TARGET}`、通常の機能。Cargo.lock SHA-256: `{lock_hash}`。', '',
             f'外部クレート {len(records)} 件（同名の別版は別件）。実行時 {sum(r["role"] == "実行時" for r in records)} 件。', '',
             'ビルド用・手続きマクロ用も取りこぼしを避けて全文束に含める。試験用の依存は除く。', '',
             *([f'`{built_with}` と同じ cargo のビルド（`-p {built_with} -p {package}`）で機能が合わさった、`{package}` の部分木。', ''] if built_with else []),
             '| 選択した許諾（追加条件を含む） | 件数 |', '|---|---:|']
    lines += [f'| {license_id} | {count} |' for license_id, count in sorted(counts.items())]
    lines += ['', '状態: ' + ('要確認。配布用全文束は生成しない。' if errors else 'クレートの許諾照合は成功。'), '',
              '| クレート | 版 | 用途 | 宣言された許諾 | 選択・追加条件 | 確認 |', '|---|---|---|---|---|---|']
    for r in records:
        lines.append(f'| {r["crate"]} | {r["version"]} | {r["role"]} | {r["declared"]} | {" AND ".join(r["selected"])} | {" / ".join(r["issues"]) or "確認済み"} |')
    return '\n'.join(lines) + '\n'


def use_utf8_output():
    """Windows の runner では標準出力がパイプで、既定の符号化（ANSI）では日本語を書けず落ちる。"""
    for stream in (sys.stdout, sys.stderr):
        reconfigure = getattr(stream, 'reconfigure', None)
        if reconfigure:
            reconfigure(encoding='utf-8')


def main():
    global TARGET
    use_utf8_output()
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--package', choices=['yolu-app', 'yolu-update', 'yolu-cli', 'xtask', 'all'], default='all')
    parser.add_argument('--audit-lock', action='store_true', help='登録した全ターゲット（Windows・Linux・macOS）の試験依存も照合し、lock 全件の分類を lock-inventory.json に記録する（配布用全文束は作らない）')
    parser.add_argument('--bundle', action='store_true', help='照合成功時だけ配布用 THIRD_PARTY_LICENSES.txt を作る')
    parser.add_argument('--offline', action='store_true', help='取得済みの原文だけを使う')
    parser.add_argument('--target', choices=['x86_64-pc-windows-gnu', 'x86_64-pc-windows-msvc', 'x86_64-unknown-linux-gnu',
                                             'aarch64-apple-darwin', 'x86_64-apple-darwin', 'universal-apple-darwin'])
    parser.add_argument('--include-update', action='store_true', help='将来組み込む更新クレートも全文束に含める')
    parser.add_argument('--include-cli', action='store_true', help='同じ配布物に入るコマンドライン（yolu-cli）の依存も全文束に含める')
    parser.add_argument('--built-with', choices=['yolu-app', 'yolu-update', 'yolu-cli'],
                        help='指した製品と同じ cargo の命令でビルドする物として、--package の依存を数える（機能が合わさって、単独の木に無い依存が入る。'
                             '`cargo build -p yolu-app -p yolu-cli` でビルドする yolupainter-cli の .mcpb 用）')
    args = parser.parse_args()
    if args.built_with and (args.package == 'all' or args.built_with == args.package):
        parser.error('--built-with は、別の製品を指した --package と一緒に使います')
    TARGET = args.target or 'x86_64-pc-windows-gnu'
    output = OUT / TARGET if args.target else OUT
    output.mkdir(parents=True, exist_ok=True)
    selected = ['yolu-app', 'yolu-cli'] if args.package == 'all' else [args.package]
    if args.audit_lock:
        if args.bundle or args.target or args.package != 'all' or args.include_update or args.include_cli or args.built_with:
            parser.error('--audit-lock は --offline 以外と併用できません')
        config = json.loads(CONFIG.read_text(encoding='utf-8'))
        if config['schema'] != 1:
            raise ValueError('設定の版が違います')
        metadata = json.loads(cargo('metadata', '--locked', '--format-version', '1',
                                   *(['--offline'] if args.offline else [])))
        return audit_lock(metadata, config, args.offline)
    # 失敗した今回の結果と、以前の成功した全文束を取り違えない。
    for package in selected:
        (output / package / 'THIRD_PARTY_LICENSES.txt').unlink(missing_ok=True)
    config = json.loads(CONFIG.read_text(encoding='utf-8'))
    registered = config.get('targets', [config.get('target')])
    if config['schema'] != 1 or any(triple not in registered for triple in triples(TARGET)):
        raise ValueError('設定の版または対象が違います')
    metadata = merged_metadata(args.offline)
    lock_hash = digest((ROOT / 'Cargo.lock').read_bytes())
    failures = 0
    for package in selected:
        records, texts, errors = inventory(package, metadata, config, args.offline, args.include_update, args.include_cli, args.built_with)
        directory = output / package
        directory.mkdir(exist_ok=True)
        (directory / 'inventory.json').write_text(json.dumps({'target': TARGET, 'package': package,
            'built_with': args.built_with, 'lock_sha256': lock_hash, 'records': records, 'issues': errors}, ensure_ascii=False, indent=2) + '\n', encoding='utf-8')
        (directory / 'THIRD_PARTY.md').write_text(markdown(package, records, errors, lock_hash, args.built_with), encoding='utf-8')
        if errors:
            failures += 1
            print(f'{package}: {len(records)} クレート、要確認', file=sys.stderr)
            for error in errors:
                print('  ' + error, file=sys.stderr)
        else:
            if args.bundle:
                temporary = directory / 'THIRD_PARTY_LICENSES.txt.tmp'
                temporary.write_text(
                    f'{package} 第三者の許諾全文\n対象: {TARGET}\nCargo.lock SHA-256: {lock_hash}\n'
                    + '\n'.join(texts), encoding='utf-8')
                temporary.replace(directory / 'THIRD_PARTY_LICENSES.txt')
            print(f'{package}: {len(records)} クレート、照合成功')
    print('結果: ' + str(output.relative_to(ROOT)) + '/<クレート>/')
    return 1 if failures else 0


if __name__ == '__main__':
    try:
        sys.exit(main())
    except (OSError, ValueError, subprocess.CalledProcessError) as exc:
        print(f'許諾の照合を完了できません: {exc}', file=sys.stderr)
        sys.exit(1)
