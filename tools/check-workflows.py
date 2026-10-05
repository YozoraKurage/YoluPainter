#!/usr/bin/env python3
"""ワークフローの YAML が読めること、配る対象の一覧と release.yml の入力が食い違わないこと、配る物の手順の決まりを確かめる。

`cargo xtask preflight` が呼ぶ（actionlint の代わりではない。文法と式の検査は actionlint に任せ、ここは「この repo の決まり」だけを見る）。
YAML の読み取りには PyYAML が要る。無いときは何も確かめず終了コード 3（preflight は「飛ばした」と表示する）。
終了コード: 0 = 全部通った／1 = 食い違いがある（1 行ずつ標準エラーへ）／3 = PyYAML が無い。
"""
import argparse
import json
from pathlib import Path
import re
import sys

try:
    import yaml
except ImportError:  # pragma: no cover - 環境による
    yaml = None

ROOT = Path(__file__).resolve().parents[1]
# release.yml の入力のうち、対象の一覧と関係なく持つもの。
FIXED_INPUTS = {'kind', 'dry-run', 'rebuild'}
PINNED = re.compile(r'^[A-Za-z0-9_.-]+/[A-Za-z0-9_./-]+@[0-9a-f]{40}$')
DIST_BUILD = './.github/workflows/dist-build.yml'


def triggers(document):
    """`on` は YAML 1.1 では真偽値の True に読まれる。"""
    value = document.get('on', document.get(True))
    return value if isinstance(value, dict) else {}


def steps_of(document):
    for job in (document.get('jobs') or {}).values():
        yield from (job.get('steps') or [])


def uses_of(document):
    for job in (document.get('jobs') or {}).values():
        if 'uses' in job:
            yield job['uses']
    for step in steps_of(document):
        if 'uses' in step:
            yield step['uses']


def load(root):
    problems, documents = [], {}
    directory = Path(root) / '.github/workflows'
    for path in sorted([*directory.glob('*.yml'), *directory.glob('*.yaml')]):
        try:
            document = yaml.safe_load(path.read_text(encoding='utf-8'))
        except (OSError, yaml.YAMLError) as exc:
            problems.append(f'{path.name}: YAML を読めません: {str(exc).splitlines()[0] if str(exc) else exc}')
            continue
        if not isinstance(document, dict) or not isinstance(document.get('jobs'), dict) or not triggers(document):
            problems.append(f'{path.name}: on・jobs が読めません')
            continue
        documents[path.name] = document
    return documents, problems


def check(root):
    documents, problems = load(root)
    targets = json.loads((Path(root) / 'tools/dist-targets.json').read_text(encoding='utf-8'))['targets']
    switches = {t['input'] for t in targets if t['input']}

    # 外の Actions はコミットの SHA で固定する（版の名前のタグは動かせるため）。
    for name, document in documents.items():
        for uses in uses_of(document):
            if uses.startswith('./') or uses.startswith('docker://'):
                continue
            if not PINNED.match(uses):
                problems.append(f'{name}: 外の Action がコミットの SHA で固定されていません: {uses}')

    ci = documents.get('ci.yml')
    if ci is None:
        problems.append('ci.yml がありません')
    else:
        on = triggers(ci)
        if 'push' in on:
            problems.append('ci.yml: push で動かさない（main は CI を通した PR からしか変わらない。同じ中身をもう一度組むだけになる）')
        for needed in ('pull_request', 'workflow_dispatch'):
            if needed not in on:
                problems.append(f'ci.yml: {needed} で動きません')
        conditions = ' '.join(str(job.get('if', '')) for job in ci['jobs'].values())
        for needle in ("github.base_ref == 'main'", 'github.event.pull_request.head.repo.full_name == github.repository'):
            if needle not in conditions:
                problems.append(f'ci.yml: 配る物の組みの条件に {needle} がありません（main 向けの、同じリポジトリの PR だけで組む）')

    release = documents.get('release.yml')
    if release is None:
        problems.append('release.yml がありません')
    else:
        inputs = (triggers(release).get('workflow_dispatch') or {}).get('inputs') or {}
        for name in sorted(switches | {'rebuild'}):
            item = inputs.get(name)
            if not isinstance(item, dict):
                problems.append(f'release.yml: 入力 {name} がありません（tools/dist-targets.json が参照している）')
            elif item.get('type') != 'boolean' or item.get('default') is not False:
                problems.append(f'release.yml: 入力 {name} は既定が切の真偽値にする')
        for name in sorted(set(inputs) - FIXED_INPUTS - switches):
            problems.append(f'release.yml: 入力 {name} を使う対象が tools/dist-targets.json にありません')

    # 配る物の組みは 1 つの手順（dist-build.yml）を ci.yml と release.yml の両方が呼ぶ。
    build = documents.get('dist-build.yml')
    if build is None:
        problems.append('dist-build.yml がありません')
    elif 'workflow_call' not in triggers(build):
        problems.append('dist-build.yml: workflow_call で呼べません')
    for name in ('ci.yml', 'release.yml'):
        if name in documents and DIST_BUILD not in list(uses_of(documents[name])):
            problems.append(f'{name}: 配る物の組み（{DIST_BUILD}）を呼んでいません')

    # 配る物を作る手順はキャッシュを使わない（汚染されたキャッシュが配る物に入る道を作らない）。
    for name in ('release.yml', 'dist-build.yml'):
        document = documents.get(name)
        if document is None:
            continue
        for step in steps_of(document):
            text = json.dumps(step, ensure_ascii=False).lower()
            used = str(step.get('uses', '')).lower()
            if 'cache' in used or 'cache' in (step.get('with') or {}) or 'sccache' in text or 'rustc_wrapper' in text:
                problems.append(f'{name}: 配る物の手順でキャッシュを使っています: {step.get("name") or step.get("uses")}')
    return documents, problems


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument('--root', default=str(ROOT), help='リポジトリの根（試験用）')
    args = parser.parse_args(argv)
    if yaml is None:
        print('PyYAML が無いので、ワークフローの YAML を確かめられません', file=sys.stderr)
        return 3
    documents, problems = check(args.root)
    for problem in problems:
        print(problem, file=sys.stderr)
    if problems:
        return 1
    print(f'ワークフロー {len(documents)} 本を確認しました')
    return 0


if __name__ == '__main__':
    for stream in (sys.stdout, sys.stderr):
        reconfigure = getattr(stream, 'reconfigure', None)
        if reconfigure:
            reconfigure(encoding='utf-8')
    sys.exit(main())
