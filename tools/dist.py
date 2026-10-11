#!/usr/bin/env python3
"""配る物のビルドと、試験の通った成果物の昇格をつなぐ。標準ライブラリだけで動く（Windows の runner でも）。

  plan     対象の一覧（tools/dist-targets.json）から、ビルドの matrix・試作の対象・成果物の絞り込み・いまの木（ファイルの木の SHA）を出す
  revision アプリに埋める診断用の ID を決め、あとの手順の環境（GITHUB_ENV）へ渡す
  catalog  ビルドした配る物の目録（catalog.json）を書き、成果物の名前（dist-<木>-<対象>）を出す
  find     配布の前に、同じ木で、全部のジョブが成功した PR の CI の成果物を探し、確かめる（受け取れるか・理由）
  install  受け取った成果物の目録を確かめ、配る物だけを 1 つのフォルダへ集める
  ci-ok    PR の先頭の CI の実行が成功か（main-tested.yml が使う）。試作の対象の失敗だけなら成功とみなす

試作の対象（dist-targets.json の `experimental: true`）は、落ちても配布を止めない: CI の結論を失敗にせず（dist-build.yml の
continue-on-error）、その失敗は `judge_run`・`ci-ok` が数えず、成果物が無ければ `find`・`install` は載せずに先へ進む（理由を要約に出す）。
試作でない対象（Windows）は、今までどおり 1 つでも欠けたら止まる。

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
# 1 つの成果物（zip）の展開後の合計の上限。配る物（Windows の zip とインストーラーは 100 MB 前後、macOS の zip は約 80 MB）なので、大きく外れた物は受け取らない。
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
        if not isinstance(item.get('default', False), bool):
            raise DistError(f"dist-targets.json の対象の default は真偽値です: {item['target']}")
        if item.get('default') and not item['input']:
            raise DistError(f"dist-targets.json の default は入力のある対象だけに付けます: {item['target']}")
        if not isinstance(item.get('rust_targets', ''), str):
            raise DistError(f"dist-targets.json の対象の rust_targets は空白で区切った文字列です: {item['target']}")
        if not isinstance(item.get('experimental', False), bool):
            raise DistError(f"dist-targets.json の対象の experimental は真偽値です: {item['target']}")
        timeout = item.get('timeout_minutes', 1)
        if isinstance(timeout, bool) or not isinstance(timeout, int) or timeout < 1:
            raise DistError(f"dist-targets.json の対象の timeout_minutes は 1 以上の整数です: {item['target']}")
    return config


def experimental_targets(config, targets=None):
    """試作の対象（落ちても配布を止めない）。`targets` を渡すと、その中の試作の対象だけ。"""
    chosen = {item['target'] for item in config['targets'] if item.get('experimental')}
    return chosen if targets is None else chosen & set(targets)


def default_inputs(config):
    """入力の既定が入の対象の入力の名前（PR の CI のように、入力が無いときに入として扱う）。"""
    return {item['input'] for item in config['targets'] if item['input'] and item.get('default')}


def select_targets(config, enabled_inputs):
    """入力が要らない対象と、入りになった入力の対象を、一覧の順に返す。一覧に無い入力が入りなら断る（名前の食い違いを黙って通さない）。"""
    known = {item['input'] for item in config['targets'] if item['input']}
    unknown = set(enabled_inputs) - known
    if unknown:
        raise DistError('対象の一覧に無い入力です: ' + '、'.join(sorted(unknown)))
    return [item for item in config['targets'] if not item['input'] or item['input'] in enabled_inputs]


def resolve_inputs(config, items):
    """`NAME=true|false` の並びから、入になっている入力の集合。言わなかった入力は既定のまま（既定が入の対象の入力は入、それ以外は切）。"""
    known = {item['input'] for item in config['targets'] if item['input']}
    enabled = default_inputs(config)
    for item in items:
        name, _, value = item.partition('=')
        if value not in ('true', 'false'):
            raise DistError(f'--input は NAME=true|false の形です: {item}')
        if name not in known:
            raise DistError(f'対象の一覧に無い入力です: {name}')
        if value == 'true':
            enabled.add(name)
        else:
            enabled.discard(name)
    return enabled


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
    include = []
    for t in chosen:
        entry = {'os': t['os'], 'target': t['target'], 'installer': bool(t['installer'])}
        # 1 つの配る物を作るために runner へ入れる Rust のターゲット（macOS の universal は 2 つ）。無ければ target そのもの
        if t.get('rust_targets'):
            entry['rust_targets'] = t['rust_targets']
        # 試作の対象: 落ちても CI の結論を失敗にしない（dist-build.yml の continue-on-error が読む）
        if t.get('experimental'):
            entry['experimental'] = True
        # ジョブの時間の上限（分）。無ければ dist-build.yml の既定
        if t.get('timeout_minutes'):
            entry['timeout'] = t['timeout_minutes']
        include.append(entry)
    return {'include': include}


def artifact_pattern(tree, targets):
    """成果物の取り込みを、配る対象だけに絞る glob（download-artifact の pattern）。複数の対象は `{a,b}` の形。"""
    if len(targets) == 1:
        return artifact_name(tree, targets[0])
    return f"dist-{tree}-{{{','.join(targets)}}}"


def cmd_plan(args):
    config = load_targets()
    matrix = plan(config, resolve_inputs(config, args.input))
    write_output('matrix', json.dumps(matrix, separators=(',', ':')))
    targets = [item['target'] for item in matrix['include']]
    tree = tree_of()
    write_output('targets', ','.join(targets))
    write_output('experimental', ','.join(t for t in targets if t in experimental_targets(config)))
    write_output('pattern', artifact_pattern(tree, targets))
    write_output('tree', tree)


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
    def __init__(self, promote, run_id=None, reasons=None, notes=None):
        self.promote = promote
        self.run_id = run_id
        self.reasons = reasons or []
        # 受け取れた（promote）とき、載らない試作の対象とその理由
        self.notes = notes or []


def failed_jobs(gh, repo, run_id, experimental=()):
    """この実行の、成功していないジョブ。(試作でないジョブの名前の一覧, 試作のジョブの名前の一覧)。一覧を読み切れなければ DistError。
    試作の対象のジョブは、配る物のビルドのジョブ名 `配る物のビルド（<対象>）`（dist-build.yml。呼び出し元の名前が前に付く）で見分ける。"""
    jobs = gh.api(f'repos/{repo}/actions/runs/{run_id}/jobs?filter=latest&per_page=100')
    listed = jobs.get('jobs', [])
    if not listed or len(listed) != jobs.get('total_count'):
        raise DistError(f'実行 {run_id} のジョブの一覧を読み切れません')
    bad, soft = [], []
    for job in listed:
        if job.get('conclusion') == 'success':
            continue
        label = f'{job["name"]}（{job.get("conclusion")}）'
        (soft if any(f'（{target}）' in job['name'] for target in experimental) else bad).append(label)
    return bad, soft


def run_conclusion_ok(gh, repo, run_id, run, experimental=()):
    """実行の結論が成功か。試作の対象のジョブだけが落ちて結論が失敗になった実行も、成功とみなす。(よい, 理由, 載らない試作のジョブ)。"""
    if run.get('status') != 'completed' or run.get('conclusion') not in ('success', 'failure'):
        return False, f'実行 {run_id} は成功していません（{run.get("status")}・{run.get("conclusion")}）', []
    bad, soft = failed_jobs(gh, repo, run_id, experimental)
    if bad:
        return False, f'実行 {run_id} に成功でないジョブがあります: ' + '、'.join(bad), soft
    if run.get('conclusion') == 'failure' and not soft:
        return False, f'実行 {run_id} は成功していません（{run.get("status")}・{run.get("conclusion")}。失敗したジョブを見つけられません）', soft
    return True, '', soft


def judge_run(gh, repo, run_id, tree, experimental=()):
    """この CI の実行の成果物を受け取ってよいか。(よい, 理由)。

    全部のジョブが成功していることを求める。ただし試作の対象（`experimental`）のジョブの失敗は数えない（その成果物が無いだけで、
    ほかの対象の成果物は受け取れる。載らない理由は `find` の要約に出る）。試作でない対象は、1 つでも失敗していれば受け取らない。"""
    run = gh.api(f'repos/{repo}/actions/runs/{run_id}')
    if run.get('path', '').split('@')[0] != CI_WORKFLOW:
        return False, f'実行 {run_id} は CI のワークフロー（{CI_WORKFLOW}）の実行ではありません'
    if run.get('event') != 'pull_request':
        return False, f'実行 {run_id} は pull_request の実行ではありません（{run.get("event")}）'
    head_repo = (run.get('head_repository') or {}).get('full_name')
    if (run.get('repository') or {}).get('full_name') != repo or head_repo != repo:
        return False, f'実行 {run_id} は同じリポジトリの枝からの実行ではありません'
    try:
        ok, why, _ = run_conclusion_ok(gh, repo, run_id, run, experimental)
    except DistError as exc:
        return False, str(exc)
    if not ok:
        return False, why
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


def find(gh, repo, tree, targets, work, update_public_key, rebuild=False, optional=()):
    """昇格できる CI の実行を探す。`optional` は試作の対象: その成果物が無くても（CI で落ちた・期限切れ・目録が合わない）、
    残りの対象が全部そろっていれば受け取り、載らない理由を `Decision.notes` に出す。試作でない対象は、1 つでも欠ければ受け取らない。"""
    if rebuild:
        return Decision(False, None, ['入力 rebuild が入なので、必ずビルドする'])
    required = [t for t in targets if t not in optional]
    extra = [t for t in targets if t in optional]
    reasons = []
    notes = {}  # 試作の対象 -> 載らない理由
    candidates = {}  # 対象 -> {実行 id: 成果物}
    verdicts = {}

    def why_not(target, text):
        if target in optional:
            notes.setdefault(target, [])
            if text not in notes[target]:
                notes[target].append(text)
        elif text not in reasons:
            reasons.append(text)

    for target in targets:
        name = artifact_name(tree, target)
        listing = gh.api(f'repos/{repo}/actions/artifacts?name={urllib.parse.quote(name)}&per_page=100')
        found = [a for a in listing.get('artifacts', []) if a.get('name') == name]
        usable = {}
        if not found:
            why_not(target, f'{target}: CI の成果物 {name} が見つからない（この木の PR の CI が無いか、そのビルドが落ちたか、保存の期限が切れた）')
        for artifact in sorted(found, key=lambda a: a.get('created_at', ''), reverse=True):
            if artifact.get('expired'):
                why_not(target, f'{target}: 成果物 {artifact["id"]} は期限切れ')
                continue
            run_id = (artifact.get('workflow_run') or {}).get('id')
            if run_id is None:
                continue
            if run_id not in verdicts:
                verdicts[run_id] = judge_run(gh, repo, run_id, tree, optional)
            ok, why = verdicts[run_id]
            if ok:
                usable.setdefault(run_id, artifact)
            else:
                why_not(target, why)
        candidates[target] = usable
    if any(not candidates[t] for t in required):
        return Decision(False, None, reasons)
    common = set.intersection(*(set(candidates[t]) for t in required)) if required else set()
    if not common:
        return Decision(False, None, reasons + ['全部の対象がそろった CI の実行がありません'])
    for run_id in sorted(common, reverse=True):
        problems = []
        for target in required:
            problems += fetch_artifact(gh, repo, candidates[target][run_id], work, tree, target, update_public_key)
        omitted = []
        for target in extra:
            artifact = candidates[target].get(run_id)
            if artifact is None:
                omitted.append(f'{target}（試作）: この実行に使える成果物が無い（ビルドが落ちた・期限切れ・形が合わない）')
                continue
            bad = fetch_artifact(gh, repo, artifact, work, tree, target, update_public_key)
            omitted += [f'{target}（試作）: {item}' for item in bad]
        if not problems:
            return Decision(True, run_id, [], omitted)
        reasons += problems
    return Decision(False, None, reasons)


def split_list(text):
    return [item for item in (text or '').split(',') if item]


def cmd_find(args):
    targets = split_list(args.targets)
    if not targets:
        raise DistError('--targets が空です')
    optional = set(split_list(args.optional))
    rebuild = {'true': True, 'false': False}[args.rebuild]
    try:
        decision = find(Gh(), args.repo, args.tree, targets, args.work,
                        os.environ.get('YOLUPAINTER_UPDATE_PUBLIC_KEY', ''), rebuild, optional)
    except (DistError, OSError, ValueError, KeyError) as exc:
        # 探す段の失敗で配布を止めない（ビルドし直せば足りる）。理由は Summary に残る。
        decision = Decision(False, None, [f'探す段で失敗したのでビルドする: {exc}'])
    lines = ['## 配る物の出どころ', '', f'- 木: `{args.tree}`', f'- 対象: {", ".join(targets)}']
    if optional:
        lines += [f'- 試作の対象（載らなくても配布は止めない）: {", ".join(sorted(optional))}']
    if decision.promote:
        lines += [f'- 結果: 試験の通った CI の実行 {decision.run_id} の成果物を受け取る（ビルドを飛ばす）']
        lines += [f'  - 載らない: {note}' for note in decision.notes]
    else:
        lines += ['- 結果: ビルドする', *[f'  - {reason}' for reason in decision.reasons]]
    write_summary(lines)
    write_output('promote', 'true' if decision.promote else 'false')
    write_output('run-id', decision.run_id or '')


# ───────── install ─────────

def install(received, tree, targets, out, version, update_public_key, optional=(), omitted=None):
    """目録を確かめて、配る物を `out` へ集める。コピーしたファイル名を返す。

    試作の対象（`optional`）は、成果物が無い・目録が合わないときに、載せずに先へ進む（理由を `omitted` の {対象: [理由]} に入れる）。
    試作でない対象は、1 つでも問題があれば DistError（何も集めない）。"""
    problems = []
    usable = []
    for target in targets:
        directory = Path(received) / artifact_name(tree, target)
        if not directory.is_dir():
            found = [f'{directory.name}: 成果物がありません']
        else:
            found = verify_catalog(directory, tree, target, version, update_public_key)
        if not found:
            usable.append(target)
        elif target in optional:
            if omitted is not None:
                omitted[target] = found
        else:
            problems += found
    if problems:
        raise DistError('\n'.join(problems))
    out = Path(out)
    out.mkdir(parents=True, exist_ok=True)
    copied = []
    for target in usable:
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
    targets = split_list(args.targets)
    key = args.update_public_key if args.update_public_key is not None else os.environ.get('YOLUPAINTER_UPDATE_PUBLIC_KEY', '')
    omitted = {}
    copied = install(args.received, args.tree, targets, args.out, args.version, key, set(split_list(args.optional)), omitted)
    lines = ['### 受け取った配る物（目録の SHA-256 を確認済み）', '', *[f'- {name}' for name in copied]]
    for target, problems in omitted.items():
        lines += ['', f'### {target}（試作）は載らなかった', '',
                  '試作の対象なので、載せずに先へ進みます（Windows など、ほかの対象の配る物だけで下書きを作ります）。理由:', '',
                  *[f'- {problem}' for problem in problems]]
    write_summary(lines)


def cmd_ci_ok(args):
    """main-tested.yml: PR の先頭の commit の CI（ci.yml・pull_request）の最新の実行が成功か。試作の対象のジョブだけが落ちた実行も成功とみなす。"""
    gh = Gh()
    experimental = experimental_targets(load_targets())
    runs = gh.api(f'repos/{args.repo}/actions/workflows/ci.yml/runs?head_sha={args.head}&event=pull_request&per_page=1').get('workflow_runs', [])
    if not runs:
        print('CI の実行が見つかりません', file=sys.stderr)
        return 1
    run = runs[0]
    ok, why, soft = run_conclusion_ok(gh, args.repo, run['id'], run, experimental)
    if not ok:
        print(why, file=sys.stderr)
        return 1
    note = f'（試作の対象の失敗は数えていない: {"、".join(soft)}）' if soft else ''
    print(f'CI の実行 {run["id"]} は成功しています{note}')
    return 0


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
    p.add_argument('--optional', default='', help='試作の対象（カンマ区切り。成果物が無くても、残りがそろえば受け取る）')
    p.set_defaults(run=cmd_find)
    p = sub.add_parser('install', help='目録を確かめて配る物を集める')
    p.add_argument('--received', required=True)
    p.add_argument('--tree', required=True)
    p.add_argument('--targets', required=True)
    p.add_argument('--out', required=True)
    p.add_argument('--version')
    p.add_argument('--update-public-key', default=None)
    p.add_argument('--optional', default='', help='試作の対象（カンマ区切り。成果物が無い・目録が合わないときは載せずに進む）')
    p.set_defaults(run=cmd_install)
    p = sub.add_parser('ci-ok', help='PR の先頭の CI の実行が成功か（試作の対象の失敗は数えない）')
    p.add_argument('--repo', required=True)
    p.add_argument('--head', required=True, help='PR の先頭の commit の SHA')
    p.set_defaults(run=cmd_ci_ok)
    args = parser.parse_args(argv)
    try:
        code = args.run(args)
    except DistError as exc:
        print(f'配る物の処理を完了できません: {exc}', file=sys.stderr)
        return 1
    return code or 0


if __name__ == '__main__':
    for stream in (sys.stdout, sys.stderr):
        reconfigure = getattr(stream, 'reconfigure', None)
        if reconfigure:
            reconfigure(encoding='utf-8')
    sys.exit(main())
