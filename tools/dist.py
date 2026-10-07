#!/usr/bin/env python3
"""配る物のビルドと、試験の通った成果物の昇格をつなぐ。標準ライブラリだけで動く（Windows の runner でも）。

  plan     対象の一覧（tools/dist-targets.json）から、ビルドの matrix と、いまの木（ファイルの木の SHA）を出す
  revision アプリに埋める診断用の ID を決め、あとの手順の環境（GITHUB_ENV）へ渡す
  catalog  ビルドした配る物の目録（catalog.json）を書き、成果物の名前（dist-<木>-<対象>）を出す
  find     配布の前に、同じ木で、全部のジョブが成功した PR の CI の成果物を探し、確かめる（受け取れるか・理由）
  install  受け取った成果物の目録を確かめ、配る物だけを 1 つのフォルダへ集める

成果物の名前に入れる「木」は `git rev-parse HEAD^{tree}`（PR の CI は merge の commit の木）。コミットの SHA ではなく木で引くので、
squash で main に入った commit も、PR の最後の CI と同じ木なら同じ成果物を指す。安全の筋と保証しないことは docs/RELEASING.md。
"""
import argparse
import hashlib
import io
import json
import os
from pathlib import Path, PurePosixPath
import re
import shutil
import stat
import subprocess
import sys
import urllib.parse
import zipfile

ROOT = Path(__file__).resolve().parents[1]
TARGETS_FILE = ROOT / 'tools/dist-targets.json'
CATALOG = 'catalog.json'
CATALOG_SCHEMA = 1
# 成果物を受け取ってよい CI のワークフロー（実行の path と照らす）。
CI_WORKFLOW = '.github/workflows/ci.yml'
# 1 つの成果物（zip）の展開後の合計の上限。配る物（zip とインストーラー）は 100 MB 前後なので、大きく外れた物は受け取らない。
MAX_EXTRACTED = 1 << 30


class DistError(Exception):
    pass


# ───────── 対象の一覧 ─────────

def load_targets(path=TARGETS_FILE):
    config = json.loads(Path(path).read_text(encoding='utf-8'))
    if config.get('schema') != 1:
        raise DistError('dist-targets.json の版が違います')
    seen = set()
    for item in config['targets']:
        for key in ('target', 'os', 'installer', 'input'):
            if key not in item:
                raise DistError(f'dist-targets.json の対象に {key} がありません: {item}')
        if item['target'] in seen:
            raise DistError(f"dist-targets.json の対象が重複しています: {item['target']}")
        seen.add(item['target'])
    return config


def select_targets(config, enabled_inputs):
    """入力が要らない対象と、入りになった入力の対象を、一覧の順に返す。一覧に無い入力が入りなら断る（名前の食い違いを黙って通さない）。"""
    known = {item['input'] for item in config['targets'] if item['input']}
    unknown = set(enabled_inputs) - known
    if unknown:
        raise DistError('対象の一覧に無い入力です: ' + '、'.join(sorted(unknown)))
    return [item for item in config['targets'] if not item['input'] or item['input'] in enabled_inputs]


def artifact_name(tree, target):
    return f'dist-{tree}-{target}'


# ───────── ツール ─────────

def git(*args, cwd=ROOT):
    try:
        return subprocess.check_output(['git', *args], cwd=cwd, text=True, encoding='utf-8').strip()
    except (OSError, subprocess.CalledProcessError) as exc:
        raise DistError(f'git {" ".join(args)} を実行できません: {exc}') from exc


def tree_of(rev='HEAD', cwd=ROOT):
    return git('rev-parse', f'{rev}^{{tree}}', cwd=cwd)


def sha256_bytes(data):
    return hashlib.sha256(data).hexdigest()


def sha256_file(path):
    digest = hashlib.sha256()
    with open(path, 'rb') as stream:
        for block in iter(lambda: stream.read(1 << 20), b''):
            digest.update(block)
    return digest.hexdigest()


def write_output(name, value):
    """GitHub Actions の出力（1 行の値だけ）。環境が無い（手元）ときは標準出力へ。"""
    value = str(value)
    if '\n' in value:
        raise DistError(f'出力 {name} に改行は入れられません')
    path = os.environ.get('GITHUB_OUTPUT')
    if path:
        with open(path, 'a', encoding='utf-8') as stream:
            stream.write(f'{name}={value}\n')
    else:
        print(f'{name}={value}')


def write_summary(lines):
    text = '\n'.join(lines) + '\n'
    path = os.environ.get('GITHUB_STEP_SUMMARY')
    if path:
        with open(path, 'a', encoding='utf-8') as stream:
            stream.write(text)
    print(text, end='')


def workspace_version():
    try:
        text = subprocess.check_output(['cargo', 'metadata', '--locked', '--no-deps', '--format-version', '1'],
                                       cwd=ROOT, text=True, encoding='utf-8')
    except (OSError, subprocess.CalledProcessError) as exc:
        raise DistError(f'版を取得できません: {exc}') from exc
    return next(p['version'] for p in json.loads(text)['packages'] if p['name'] == 'yolu-app')


# ───────── plan ─────────

def plan(config, enabled_inputs):
    chosen = select_targets(config, enabled_inputs)
    return {'include': [{'os': t['os'], 'target': t['target'], 'installer': bool(t['installer'])} for t in chosen]}


def cmd_plan(args):
    enabled = set()
    for item in args.input:
        name, _, value = item.partition('=')
        if value not in ('true', 'false'):
            raise DistError(f'--input は NAME=true|false の形です: {item}')
        if value == 'true':
            enabled.add(name)
        elif name not in {t['input'] for t in load_targets()['targets']}:
            raise DistError(f'対象の一覧に無い入力です: {name}')
    matrix = plan(load_targets(), enabled)
    write_output('matrix', json.dumps(matrix, separators=(',', ':')))
    write_output('targets', ','.join(item['target'] for item in matrix['include']))
    write_output('tree', tree_of())


# ───────── revision ─────────

FULL_SHA = re.compile(r'^[0-9a-f]{40}$')
SHORT_ID = 7  # アプリの版の表示（「0.4.0 · a1b2c3d」）とクラッシュ報告の Git: の長さ（build.rs の `git rev-parse --short` と同じ）


def embedded_revision(environment, head):
    """アプリに埋める診断用の ID（フルの SHA, 短い ID）。

    PR の CI が checkout するのは merge の commit（refs/pull/N/merge）で、main にもタグにも入らず、PR を閉じたあとに参照できる保証も無い。
    昇格した配る物にその ID が入ると、どの commit にも対応しないので、PR の先頭の commit（PR_HEAD_SHA。昇格の条件で、その木は成果物の名前の木と同じ）を使う。
    PR でない（配布のビルド・手元）ときは HEAD。PR_HEAD_SHA が空でなく形も違うなら、黙って merge の commit に戻さず断る。
    """
    given = (environment.get('PR_HEAD_SHA') or '').strip().lower()
    if given and not FULL_SHA.match(given):
        raise DistError(f'PR_HEAD_SHA が commit の SHA（16 進 40 文字）ではありません: {given[:60]!r}')
    full = given or head
    if not FULL_SHA.match(full):
        raise DistError(f'HEAD が commit の SHA ではありません: {full[:60]!r}')
    return full, full[:SHORT_ID]


def cmd_revision(args):
    full, short = embedded_revision(os.environ, git('rev-parse', 'HEAD'))
    lines = [f'YOLU_GIT_REV={short}', f'YOLU_REVISION={full}']
    path = os.environ.get('GITHUB_ENV')
    if path:
        with open(path, 'a', encoding='utf-8') as stream:
            stream.write('\n'.join(lines) + '\n')
    write_summary(['### アプリに埋める診断用の ID', '', f'- `{short}`（{full}）'])


# ───────── catalog ─────────

def build_catalog(assets, target, tree, commit, version, environment):
    files = {}
    for path in sorted(Path(assets).iterdir()):
        if path.name == CATALOG:
            continue
        if not path.is_file() or path.is_symlink():
            raise DistError(f'配る物のフォルダに通常のファイルでない物があります: {path.name}')
        files[path.name] = {'sha256': sha256_file(path), 'size': path.stat().st_size}
    if not files:
        raise DistError('配る物がありません')
    return {
        'schema': CATALOG_SCHEMA,
        'tree': tree,
        'commit': commit,
        # アプリに埋めた診断用の ID の元の commit（PR の CI では PR の先頭。commit は merge の commit で、履歴に入らない）。
        'revision': environment.get('YOLU_REVISION') or commit,
        'version': version,
        'target': target,
        'repository': environment.get('GITHUB_REPOSITORY', ''),
        'run_id': environment.get('GITHUB_RUN_ID', ''),
        'run_attempt': environment.get('GITHUB_RUN_ATTEMPT', ''),
        'workflow_ref': environment.get('GITHUB_WORKFLOW_REF', ''),
        'event': environment.get('GITHUB_EVENT_NAME', ''),
        'runner': ' '.join(filter(None, [environment.get('RUNNER_OS', ''), environment.get('ImageOS', ''),
                                         environment.get('ImageVersion', '')])),
        'rustc': environment.get('RUSTC_VERSION', ''),
        # 公開鍵は秘密ではない。昇格のとき、いまの変数と同じ鍵を組み込んだ物だけを受け取る。
        'update_public_key': environment.get('YOLUPAINTER_UPDATE_PUBLIC_KEY', '').strip(),
        'files': files,
    }


def cmd_catalog(args):
    assets = Path(args.assets)
    environment = dict(os.environ)
    if 'RUSTC_VERSION' not in environment:
        try:
            environment['RUSTC_VERSION'] = subprocess.check_output(['rustc', '-V'], text=True).strip()
        except (OSError, subprocess.CalledProcessError):
            environment['RUSTC_VERSION'] = ''
    tree = args.tree or tree_of()
    commit = args.commit or git('rev-parse', 'HEAD')
    version = args.version or workspace_version()
    catalog = build_catalog(assets, args.target, tree, commit, version, environment)
    (assets / CATALOG).write_text(json.dumps(catalog, ensure_ascii=False, indent=2) + '\n', encoding='utf-8')
    write_output('name', artifact_name(tree, args.target))
    write_summary([f'### 配る物の目録（{args.target}）', '',
                   f'- 木: `{tree}`', f'- commit: `{commit}`', f'- アプリに埋めた ID の元: `{catalog["revision"]}`', f'- 版: {version}',
                   f'- {catalog["rustc"]}', '',
                   '| ファイル | SHA-256 | 大きさ |', '|---|---|---:|',
                   *[f'| {name} | `{item["sha256"]}` | {item["size"]} |' for name, item in catalog['files'].items()]])


# ───────── 目録の確かめ ─────────

def verify_catalog(directory, tree, target, version=None, update_public_key=None):
    """受け取った成果物のフォルダを、目録と照らす。食い違いの一覧（空なら一致）を返す。"""
    directory = Path(directory)
    where = directory.name
    path = directory / CATALOG
    if not path.is_file():
        return [f'{where}: 目録（{CATALOG}）がありません']
    try:
        catalog = json.loads(path.read_text(encoding='utf-8'))
    except (OSError, ValueError) as exc:
        return [f'{where}: 目録を読めません: {exc}']
    problems = []
    if not isinstance(catalog, dict) or not isinstance(catalog.get('files'), dict) or not catalog['files']:
        return [f'{where}: 目録の形が違います']
    if catalog.get('schema') != CATALOG_SCHEMA:
        problems.append(f'{where}: 目録の版が違います（{catalog.get("schema")}）')
    if catalog.get('tree') != tree:
        problems.append(f'{where}: 目録の木が違います（{catalog.get("tree")}。いまは {tree}）')
    if catalog.get('target') != target:
        problems.append(f'{where}: 目録の対象が違います（{catalog.get("target")}。要るのは {target}）')
    if version is not None and catalog.get('version') != version:
        problems.append(f'{where}: 目録の版が違います（{catalog.get("version")}。いまは {version}）')
    if update_public_key is not None and catalog.get('update_public_key', '') != update_public_key.strip():
        problems.append(f'{where}: 組み込んだ更新用の公開鍵が、いまの変数と違います')
    present = {p.name for p in directory.iterdir()} - {CATALOG}
    for name in sorted(present - set(catalog['files'])):
        problems.append(f'{where}: 目録に無いファイルがあります: {name}')
    for name, item in catalog['files'].items():
        file = directory / name
        if PurePosixPath(name).name != name or name in ('', '.', '..'):
            problems.append(f'{where}: 目録のファイル名が不正です: {name}')
        elif not file.is_file() or file.is_symlink():
            problems.append(f'{where}: 目録にあるファイルがありません: {name}')
        elif file.stat().st_size != item.get('size') or sha256_file(file) != item.get('sha256'):
            problems.append(f'{where}: {name} が目録の SHA-256・大きさと違います')
    return problems


# ───────── 展開 ─────────

def safe_extract(data, destination):
    """artifact の zip を、通常のファイルだけ、destination の直下へ展開する（外へ出る名前・リンク・大きすぎる物は断る）。"""
    destination = Path(destination)
    destination.mkdir(parents=True, exist_ok=True)
    total = 0
    try:
        archive = zipfile.ZipFile(io.BytesIO(data))
    except zipfile.BadZipFile as exc:
        raise DistError(f'成果物の zip を開けません: {exc}') from exc
    with archive:
        for info in archive.infolist():
            if info.is_dir():
                continue
            name = info.filename
            parts = PurePosixPath(name).parts
            if (not parts or name.startswith('/') or '\\' in name or ':' in name or '..' in parts
                    or len(parts) != 1):
                raise DistError(f'成果物に不正な名前があります: {name!r}')
            if stat.S_ISLNK(info.external_attr >> 16):
                raise DistError(f'成果物にリンクがあります: {name}')
            total += info.file_size
            if total > MAX_EXTRACTED:
                raise DistError('成果物が大きすぎます')
            with archive.open(info) as source, open(destination / name, 'wb') as out:
                shutil.copyfileobj(source, out)


# ───────── find ─────────

class Gh:
    """`gh api`（読み取りだけ）。GH_TOKEN を環境から読む。"""

    def api(self, path):
        return json.loads(self._run(path))

    def api_bytes(self, path):
        return self._run(path, text=False)

    @staticmethod
    def _run(path, text=True):
        try:
            result = subprocess.run(['gh', 'api', '-H', 'Accept: application/vnd.github+json', path],
                                    capture_output=True, text=text, encoding='utf-8' if text else None, check=False, timeout=600)
        except (OSError, subprocess.TimeoutExpired) as exc:
            raise DistError(f'gh api {path}: {exc}') from exc
        if result.returncode != 0:
            err = result.stderr if text else result.stderr.decode('utf-8', 'replace')
            raise DistError(f'gh api {path}: {err.strip()[:300]}')
        return result.stdout


class Decision:
    def __init__(self, promote, run_id=None, reasons=None):
        self.promote = promote
        self.run_id = run_id
        self.reasons = reasons or []


def judge_run(gh, repo, run_id, tree):
    """この CI の実行の成果物を受け取ってよいか。(よい, 理由)。"""
    run = gh.api(f'repos/{repo}/actions/runs/{run_id}')
    if run.get('path', '').split('@')[0] != CI_WORKFLOW:
        return False, f'実行 {run_id} は CI のワークフロー（{CI_WORKFLOW}）の実行ではありません'
    if run.get('event') != 'pull_request':
        return False, f'実行 {run_id} は pull_request の実行ではありません（{run.get("event")}）'
    if run.get('status') != 'completed' or run.get('conclusion') != 'success':
        return False, f'実行 {run_id} は成功していません（{run.get("status")}・{run.get("conclusion")}）'
    head_repo = (run.get('head_repository') or {}).get('full_name')
    if (run.get('repository') or {}).get('full_name') != repo or head_repo != repo:
        return False, f'実行 {run_id} は同じリポジトリの枝からの実行ではありません'
    jobs = gh.api(f'repos/{repo}/actions/runs/{run_id}/jobs?filter=latest&per_page=100')
    listed = jobs.get('jobs', [])
    if not listed or len(listed) != jobs.get('total_count'):
        return False, f'実行 {run_id} のジョブの一覧を読み切れません'
    bad = [f'{j["name"]}（{j.get("conclusion")}）' for j in listed if j.get('conclusion') != 'success']
    if bad:
        return False, f'実行 {run_id} に成功でないジョブがあります: ' + '、'.join(bad)
    # PR の CI の head_sha は PR の先頭の commit（merge の commit ではない）。その木が、成果物の名前の木と同じなら、
    # 「木が同じならワークフローの台本も同じ」が成り立つ（名前を偽った PR の成果物をここで落とす）。
    commit = gh.api(f'repos/{repo}/git/commits/{run.get("head_sha")}')
    head_tree = (commit.get('tree') or {}).get('sha')
    if head_tree != tree:
        return False, (f'実行 {run_id} の PR の先頭の木が違います（{head_tree}）。PR の枝が main に追いついていないと、'
                       'merge の木と先頭の木がずれます')
    return True, ''


def fetch_artifact(gh, repo, artifact, work, tree, target, update_public_key):
    """成果物を落として展開し、digest と目録を確かめる。問題の一覧（空なら使える）を返す。"""
    # digest は GitHub が成果物の zip に付けた SHA-256。目録は zip の中の自己申告なので、zip の差し替えを見つける手は digest だけになる。
    # 記録が無い成果物は確かめようが無いので受け取らず、ビルドし直しに倒す（確かめを黙って外さない）。
    digest = artifact.get('digest')
    if not digest:
        return [f'{artifact["name"]}: GitHub が記録した digest が無いので、zip を確かめられません']
    data = gh.api_bytes(f'repos/{repo}/actions/artifacts/{artifact["id"]}/zip')
    if digest != 'sha256:' + sha256_bytes(data):
        return [f'{artifact["name"]}: GitHub が記録した digest と、落とした zip が違います']
    directory = Path(work) / artifact['name']
    if directory.exists():
        shutil.rmtree(directory)
    try:
        safe_extract(data, directory)
    except DistError as exc:
        return [f'{artifact["name"]}: {exc}']
    return verify_catalog(directory, tree, target, None, update_public_key)


def find(gh, repo, tree, targets, work, update_public_key, rebuild=False):
    if rebuild:
        return Decision(False, None, ['入力 rebuild が入なので、必ずビルドする'])
    reasons = []
    candidates = {}  # 対象 -> {実行 id: 成果物}
    verdicts = {}
    for target in targets:
        name = artifact_name(tree, target)
        listing = gh.api(f'repos/{repo}/actions/artifacts?name={urllib.parse.quote(name)}&per_page=100')
        found = [a for a in listing.get('artifacts', []) if a.get('name') == name]
        usable = {}
        if not found:
            reasons.append(f'{target}: CI の成果物 {name} が見つからない（この木の PR の CI が無いか、保存の期限が切れた）')
        for artifact in sorted(found, key=lambda a: a.get('created_at', ''), reverse=True):
            if artifact.get('expired'):
                reasons.append(f'{target}: 成果物 {artifact["id"]} は期限切れ')
                continue
            run_id = (artifact.get('workflow_run') or {}).get('id')
            if run_id is None:
                continue
            if run_id not in verdicts:
                verdicts[run_id] = judge_run(gh, repo, run_id, tree)
            ok, why = verdicts[run_id]
            if ok:
                usable.setdefault(run_id, artifact)
            elif why not in reasons:
                reasons.append(why)
        candidates[target] = usable
    if any(not usable for usable in candidates.values()):
        return Decision(False, None, reasons)
    common = set.intersection(*(set(usable) for usable in candidates.values()))
    if not common:
        return Decision(False, None, reasons + ['全部の対象がそろった CI の実行がありません'])
    for run_id in sorted(common, reverse=True):
        problems = []
        for target in targets:
            problems += fetch_artifact(gh, repo, candidates[target][run_id], work, tree, target, update_public_key)
        if not problems:
            return Decision(True, run_id, [])
        reasons += problems
    return Decision(False, None, reasons)


def cmd_find(args):
    targets = [t for t in args.targets.split(',') if t]
    if not targets:
        raise DistError('--targets が空です')
    rebuild = {'true': True, 'false': False}[args.rebuild]
    try:
        decision = find(Gh(), args.repo, args.tree, targets, args.work,
                        os.environ.get('YOLUPAINTER_UPDATE_PUBLIC_KEY', ''), rebuild)
    except (DistError, OSError, ValueError, KeyError) as exc:
        # 探す段の失敗で配布を止めない（ビルドし直せば足りる）。理由は Summary に残る。
        decision = Decision(False, None, [f'探す段で失敗したのでビルドする: {exc}'])
    lines = ['## 配る物の出どころ', '', f'- 木: `{args.tree}`', f'- 対象: {", ".join(targets)}']
    if decision.promote:
        lines += [f'- 結果: 試験の通った CI の実行 {decision.run_id} の成果物を受け取る（ビルドを飛ばす）']
    else:
        lines += ['- 結果: ビルドする', *[f'  - {reason}' for reason in decision.reasons]]
    write_summary(lines)
    write_output('promote', 'true' if decision.promote else 'false')
    write_output('run-id', decision.run_id or '')


# ───────── install ─────────

def install(received, tree, targets, out, version, update_public_key):
    problems = []
    for target in targets:
        directory = Path(received) / artifact_name(tree, target)
        if not directory.is_dir():
            problems.append(f'{directory.name}: 成果物がありません')
            continue
        problems += verify_catalog(directory, tree, target, version, update_public_key)
    if problems:
        raise DistError('\n'.join(problems))
    out = Path(out)
    out.mkdir(parents=True, exist_ok=True)
    copied = []
    for target in targets:
        directory = Path(received) / artifact_name(tree, target)
        for path in sorted(directory.iterdir()):
            if path.name == CATALOG:
                continue
            destination = out / path.name
            if destination.exists():
                raise DistError(f'配る物の名前が重なっています: {path.name}')
            shutil.copyfile(path, destination)
            copied.append(path.name)
    return copied


def cmd_install(args):
    targets = [t for t in args.targets.split(',') if t]
    key = args.update_public_key if args.update_public_key is not None else os.environ.get('YOLUPAINTER_UPDATE_PUBLIC_KEY', '')
    copied = install(args.received, args.tree, targets, args.out, args.version, key)
    write_summary(['### 受け取った配る物（目録の SHA-256 を確認済み）', '', *[f'- {name}' for name in copied]])


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    sub = parser.add_subparsers(dest='command', required=True)
    p = sub.add_parser('plan', help='ビルドの matrix・対象・木を出す')
    p.add_argument('--input', action='append', default=[], help='NAME=true|false（release.yml の入力）')
    p.set_defaults(run=cmd_plan)
    p = sub.add_parser('revision', help='アプリに埋める診断用の ID を決めて GITHUB_ENV へ渡す')
    p.set_defaults(run=cmd_revision)
    p = sub.add_parser('catalog', help='目録を書く')
    p.add_argument('--target', required=True)
    p.add_argument('--assets', required=True)
    p.add_argument('--tree')
    p.add_argument('--commit')
    p.add_argument('--version')
    p.set_defaults(run=cmd_catalog)
    p = sub.add_parser('find', help='昇格できる CI の成果物を探して確かめる')
    p.add_argument('--repo', required=True)
    p.add_argument('--tree', required=True)
    p.add_argument('--targets', required=True, help='カンマ区切り')
    p.add_argument('--work', required=True, help='確かめるために展開する場所')
    p.add_argument('--rebuild', choices=['true', 'false'], default='false')
    p.set_defaults(run=cmd_find)
    p = sub.add_parser('install', help='目録を確かめて配る物を集める')
    p.add_argument('--received', required=True)
    p.add_argument('--tree', required=True)
    p.add_argument('--targets', required=True)
    p.add_argument('--out', required=True)
    p.add_argument('--version')
    p.add_argument('--update-public-key', default=None)
    p.set_defaults(run=cmd_install)
    args = parser.parse_args(argv)
    try:
        args.run(args)
    except DistError as exc:
        print(f'配る物の処理を完了できません: {exc}', file=sys.stderr)
        return 1
    return 0


if __name__ == '__main__':
    for stream in (sys.stdout, sys.stderr):
        reconfigure = getattr(stream, 'reconfigure', None)
        if reconfigure:
            reconfigure(encoding='utf-8')
    sys.exit(main())
