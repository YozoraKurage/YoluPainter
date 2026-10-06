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
# 試験版の更新情報を固定のタグの Release へ置くワークフロー。アプリが取る場所の定数は更新クレートが持つ。
BETA_CHANNEL = 'beta-channel.yml'
UPDATE_LIBRARY = 'crates/yolu-update/src/lib.rs'


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


def library_constant(root, name):
    """更新クレートの `pub const NAME: &str = "…";` の値。読めなければ None。"""
    try:
        text = (Path(root) / UPDATE_LIBRARY).read_text(encoding='utf-8')
    except OSError:
        return None
    found = re.search(rf'pub const {name}: &str = "([^"]*)";', text)
    return found.group(1) if found else None


def check_beta_channel(root, document):
    """試験版の置き場へ更新情報を置くワークフローの決まり。"""
    problems = []
    on = triggers(document)
    if set(on) != {'release'} or (on['release'] or {}).get('types') != ['published']:
        problems.append(f'{BETA_CHANNEL}: release の published だけで動かす'
                        '（Draft のうちは置き場を動かさない。prereleased は Draft からの公開では起こらない）')
    if (document.get('permissions') or {}) != {'contents': 'read'}:
        problems.append(f'{BETA_CHANNEL}: 全体の権限は contents: read だけにする（書き込みはジョブに限る）')
    conditions = ' '.join(str(job.get('if', '')) for job in document['jobs'].values())
    for needle in ('github.event.release.prerelease', "startsWith(github.event.release.tag_name, 'v')"):
        if needle not in conditions:
            problems.append(f'{BETA_CHANNEL}: 条件に {needle} がありません（試験版の Release だけで動かす。置き場の Release 自身では動かさない）')
    tag = library_constant(root, 'BETA_CHANNEL_TAG')
    name = library_constant(root, 'UPDATER_FILE')
    if tag is None or name is None:
        problems.append(f'{UPDATE_LIBRARY}: BETA_CHANNEL_TAG・UPDATER_FILE が読めません')
        return problems
    if (document.get('env') or {}).get('CHANNEL_TAG') != tag:
        problems.append(f'{BETA_CHANNEL}: env の CHANNEL_TAG が {UPDATE_LIBRARY} の BETA_CHANNEL_TAG（{tag}）と違います')
    names = set(re.findall(r'updater-v[0-9]+\.json', json.dumps(document, ensure_ascii=False)))
    if names != {name}:
        problems.append(f'{BETA_CHANNEL}: 更新情報のファイル名が UPDATER_FILE（{name}）だけではありません: {sorted(names)}')
    return problems


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
        # 下書きは、成果物を受け取って build が skipped の回にも作る。状態の関数が無いと暗に success() が付き、skipped の build につられて飛ばされる。
        draft = (release.get('jobs') or {}).get('draft') or {}
        condition = str(draft.get('if', ''))
        for needle in ('!cancelled()', "needs.metadata.result == 'success'", '!inputs.dry-run'):
            if needle not in condition:
                problems.append(f'release.yml: draft の条件に {needle} がありません（build が skipped の回にも下書きを作る）')

    # 配る物の組みは 1 つの手順（dist-build.yml）を ci.yml と release.yml の両方が呼ぶ。
    build = documents.get('dist-build.yml')
    if build is None:
        problems.append('dist-build.yml がありません')
    elif 'workflow_call' not in triggers(build):
        problems.append('dist-build.yml: workflow_call で呼べません')
    for name in ('ci.yml', 'release.yml'):
        if name in documents and DIST_BUILD not in list(uses_of(documents[name])):
            problems.append(f'{name}: 配る物の組み（{DIST_BUILD}）を呼んでいません')

    # 試験版の更新情報の置き場（公開した試験版の署名を確かめ直して、固定のタグの Release へ置く）。
    beta = documents.get(BETA_CHANNEL)
    if beta is None:
        problems.append(f'{BETA_CHANNEL} がありません')
    else:
        problems.extend(check_beta_channel(root, beta))

    # 配る物を作る手順はキャッシュを使わない（汚染されたキャッシュが配る物に入る道を作らない）。
    for name in ('release.yml', 'dist-build.yml', BETA_CHANNEL):
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
