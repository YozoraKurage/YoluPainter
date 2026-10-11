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


def check_dist_build(build, targets):
    """配る物のビルドの手順（dist-build.yml）が、対象の一覧の指定（試作・時間・複数の Rust のターゲット）を読むこと。"""
    problems = []
    job = (build.get('jobs') or {}).get('build') or {}
    steps = job.get('steps') or []
    if any(t.get('experimental') for t in targets) and 'matrix.experimental' not in str(job.get('continue-on-error', '')):
        problems.append('dist-build.yml: build の continue-on-error が matrix.experimental を読んでいません'
                        '（試作の対象が落ちると、CI の結論が失敗になり、配布も止まる）')
    if any(t.get('timeout_minutes') for t in targets) and 'matrix.timeout' not in str(job.get('timeout-minutes', '')):
        problems.append('dist-build.yml: build の timeout-minutes が matrix.timeout を読んでいません（対象ごとの時間の上限が効かない）')
    if any(t.get('rust_targets') for t in targets):
        # 複数の Rust のターゲットで作る対象は、配る物の名前（matrix.target）を --target に渡さず、rust_targets を足す
        single = [st for st in steps if str(st.get('if', '')).replace(' ', '') == '${{!matrix.rust_targets}}']
        multiple = [st for st in steps if str(st.get('if', '')).replace(' ', '') == '${{matrix.rust_targets}}']
        if not any('--target ${{ matrix.target }}' in str(st.get('run', '')) for st in single):
            problems.append('dist-build.yml: rust_targets の無い対象の Rust の準備（--target ${{ matrix.target }}）が、`if: ${{ !matrix.rust_targets }}` の手順に無い')
        if not any('rustup target add' in str(st.get('run', '')) and 'matrix.rust_targets' in str(st.get('run', '')) for st in multiple):
            problems.append('dist-build.yml: rust_targets のある対象が、`if: ${{ matrix.rust_targets }}` の手順で `rustup target add ${{ matrix.rust_targets }}` を走らせていない')
    return problems


def check_experimental_flow(documents, targets):
    """試作の対象（experimental）が落ちても配布が止まらない形（release.yml の find・metadata と main-tested.yml）が崩れていないこと。"""
    problems = []
    if not any(t.get('experimental') for t in targets):
        return problems
    release = documents.get('release.yml')
    if release is not None:
        jobs = release.get('jobs') or {}
        outputs = (jobs.get('plan') or {}).get('outputs') or {}
        for name in ('experimental', 'pattern'):
            if name not in outputs:
                problems.append(f'release.yml: plan の出力 {name} がありません（試作の対象・成果物の絞り込みを渡せない）')
        for job_name, step_text in (('find', 'tools/dist.py find'), ('metadata', 'tools/dist.py install')):
            text = json.dumps((jobs.get(job_name) or {}).get('steps') or [], ensure_ascii=False)
            if step_text not in text or '--optional' not in text:
                problems.append(f'release.yml: {job_name} が {step_text} に --optional を渡していません（試作の対象が欠けると、配布が止まる）')
        metadata = jobs.get('metadata') or {}
        if 'needs.build.result' in str(metadata.get('if', '')):
            problems.append('release.yml: metadata の条件が build の結果を見ています（試作の対象だけが落ちたときに、下書きまで止まる。'
                            '試作でない対象の欠けは install が止める）')
        patterns = [str((st.get('with') or {}).get('pattern', '')) for st in metadata.get('steps') or [] if str(st.get('uses', '')).startswith('actions/download-artifact@')]
        if not patterns or any('needs.plan.outputs.pattern' not in p for p in patterns):
            problems.append('release.yml: metadata の成果物の取り込みが、plan の出力 pattern（対象の一覧で絞った物）を使っていません')
    tested = documents.get('main-tested.yml')
    if tested is not None and 'tools/dist.py ci-ok' not in json.dumps(list(steps_of(tested)), ensure_ascii=False):
        problems.append('main-tested.yml: CI の結論を tools/dist.py ci-ok で見ていません（試作の対象だけが落ちた CI を成功とみなせず、main の確かめが落ちる）')
    return problems


def check(root):
    documents, problems = load(root)
    targets = json.loads((Path(root) / 'tools/dist-targets.json').read_text(encoding='utf-8'))['targets']
    switches = {t['input'] for t in targets if t['input']}
    # 入力の既定: 対象の default が true なら入（PR の CI でもビルドする）、無ければ切
    defaults = {t['input']: bool(t.get('default', False)) for t in targets if t['input']}

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
            problems.append('ci.yml: push で動かさない（main は CI を通した PR からしか変わらない。同じ中身をもう一度ビルドするだけになる）')
        for needed in ('pull_request', 'workflow_dispatch'):
            if needed not in on:
                problems.append(f'ci.yml: {needed} で動きません')
        conditions = ' '.join(str(job.get('if', '')) for job in ci['jobs'].values())
        for needle in ("github.base_ref == 'main'", 'github.event.pull_request.head.repo.full_name == github.repository'):
            if needle not in conditions:
                problems.append(f'ci.yml: 配る物のビルドの条件に {needle} がありません（main 向けの、同じリポジトリの PR だけでビルドする）')

    release = documents.get('release.yml')
    if release is None:
        problems.append('release.yml がありません')
    else:
        inputs = (triggers(release).get('workflow_dispatch') or {}).get('inputs') or {}
        for name in sorted(switches | {'rebuild'}):
            item = inputs.get(name)
            expected = defaults.get(name, False)
            if not isinstance(item, dict):
                problems.append(f'release.yml: 入力 {name} がありません（tools/dist-targets.json が参照している）')
            elif item.get('type') != 'boolean' or item.get('default') is not expected:
                problems.append(f'release.yml: 入力 {name} は既定が{"入" if expected else "切"}の真偽値にする'
                                f'（tools/dist-targets.json の default {"あり" if expected else "なし"}と同じ）')
        # 対象を選ぶ段（plan）が、切り替えられる入力を全部渡すこと（渡し忘れると、入力を切っても対象が変わらない）
        plan_text = json.dumps((release.get('jobs') or {}).get('plan') or {}, ensure_ascii=False)
        for name in sorted(switches):
            if f'--input {name}=${{{{ inputs.{name} }}}}' not in plan_text:
                problems.append(f'release.yml: plan が入力 {name} を渡していません（--input {name}=${{{{ inputs.{name} }}}}）')
        for name in sorted(set(inputs) - FIXED_INPUTS - switches):
            problems.append(f'release.yml: 入力 {name} を使う対象が tools/dist-targets.json にありません')
        # 下書きは、成果物を受け取って build が skipped の回にも作る。状態の関数が無いと暗に success() が付き、skipped の build につられて飛ばされる。
        draft = (release.get('jobs') or {}).get('draft') or {}
        condition = str(draft.get('if', ''))
        for needle in ('!cancelled()', "needs.metadata.result == 'success'", '!inputs.dry-run'):
            if needle not in condition:
                problems.append(f'release.yml: draft の条件に {needle} がありません（build が skipped の回にも下書きを作る）')

    # 配る物のビルドは 1 つの手順（dist-build.yml）を ci.yml と release.yml の両方が呼ぶ。
    build = documents.get('dist-build.yml')
    if build is None:
        problems.append('dist-build.yml がありません')
    elif 'workflow_call' not in triggers(build):
        problems.append('dist-build.yml: workflow_call で呼べません')
    else:
        problems.extend(check_dist_build(build, targets))
    problems.extend(check_experimental_flow(documents, targets))
    for name in ('ci.yml', 'release.yml'):
        if name in documents and DIST_BUILD not in list(uses_of(documents[name])):
            problems.append(f'{name}: 配る物のビルド（{DIST_BUILD}）を呼んでいません')

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
