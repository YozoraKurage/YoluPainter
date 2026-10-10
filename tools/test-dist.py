#!/usr/bin/env python3
"""tools/dist.py（対象の計画・目録・昇格の探索と確かめ・集める）と tools/check-workflows.py の試験。標準ライブラリだけで動く（check-workflows は PyYAML があるときだけ）。

GitHub の API は、同じ形の応答を返す偽の窓口で置き換える（`gh` も通信も使わない）。使い方: python3 tools/test-dist.py
"""
import contextlib
import hashlib
import importlib.util
import io
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest
import unittest.mock
import zipfile

sys.dont_write_bytecode = True
ROOT = Path(__file__).resolve().parents[1]


def load(name, file):
    spec = importlib.util.spec_from_file_location(name, ROOT / 'tools' / file)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


dist = load('dist', 'dist.py')
check_workflows = load('check_workflows', 'check-workflows.py')

REPO = 'owner/repo'
TREE = 'a' * 40
OTHER_TREE = 'b' * 40
WINDOWS = 'x86_64-pc-windows-msvc'
LINUX = 'x86_64-unknown-linux-gnu'
MACOS = 'universal-apple-darwin'
KEY = 'c' * 64


def make_zip(files):
    buffer = io.BytesIO()
    with zipfile.ZipFile(buffer, 'w') as archive:
        for name, data in files.items():
            archive.writestr(name, data)
    return buffer.getvalue()


def make_artifact_files(target, tree=TREE, key=KEY, version='1.2.3', extra=None, tamper=None):
    """目録つきの成果物の中身（ファイル名 → バイト列）。"""
    payload = {f'yolupainter-{version}-{target}.zip': b'zip-bytes-' + target.encode()}
    if target == WINDOWS:
        payload[f'yolupainter-{version}-{target}-setup.exe'] = b'setup-bytes'
    catalog = {
        'schema': 1, 'tree': tree, 'commit': 'd' * 40, 'version': version, 'target': target, 'update_public_key': key,
        'files': {name: {'sha256': hashlib.sha256(data).hexdigest(), 'size': len(data)} for name, data in payload.items()},
    }
    files = dict(payload)
    files['catalog.json'] = json.dumps(catalog).encode()
    if tamper:
        files[tamper] = b'tampered'
    if extra:
        files[extra] = b'extra'
    return files


class FakeGh:
    """gh api の代わり。runs: {実行 id: 設定}、artifacts: [{id, name, run, expired, files}]。"""

    def __init__(self):
        self.runs = {}
        self.artifacts = []
        self.calls = []
        self.corrupt_digest = set()
        self.missing_digest = {}  # 成果物 id -> digest の代わりに返す値（None・空文字）。キーごと無い場合は 'absent'

    def add_run(self, run_id, path='.github/workflows/ci.yml', event='pull_request', conclusion='success', status='completed',
                head_repo=REPO, jobs=None, head_tree=TREE, head_sha=None):
        self.runs[run_id] = dict(path=path, event=event, conclusion=conclusion, status=status, head_repo=head_repo,
                                 jobs=jobs if jobs is not None else [('linux', 'success'), ('windows', 'success'), ('dist', 'success')],
                                 head_tree=head_tree, head_sha=head_sha or f'{run_id:040x}')

    def add_artifact(self, artifact_id, run_id, target, tree=TREE, expired=False, created='2026-10-05T00:00:00Z', **kwargs):
        zip_bytes = make_zip(make_artifact_files(target, tree, **kwargs))
        self.artifacts.append(dict(id=artifact_id, name=f'dist-{tree}-{target}', run=run_id, expired=expired, created=created,
                                   zip=zip_bytes))

    def api(self, path):
        self.calls.append(path)
        route, _, query = path.partition('?')
        parts = route.split('/')
        if route == f'repos/{REPO}/actions/artifacts':
            name = dict(p.split('=') for p in query.split('&'))['name']
            name = dist.urllib.parse.unquote(name)
            listed = []
            for a in self.artifacts:
                if a['name'] != name:
                    continue
                item = {'id': a['id'], 'name': a['name'], 'expired': a['expired'], 'created_at': a['created'],
                        'digest': 'sha256:' + ('0' * 64 if a['id'] in self.corrupt_digest else hashlib.sha256(a['zip']).hexdigest()),
                        'workflow_run': {'id': a['run']}}
                if a['id'] in self.missing_digest:
                    if self.missing_digest[a['id']] == 'absent':
                        del item['digest']
                    else:
                        item['digest'] = self.missing_digest[a['id']]
                listed.append(item)
            return {'artifacts': listed}
        if parts[:4] == ['repos', 'owner', 'repo', 'actions'] and parts[4] == 'runs' and len(parts) == 6:
            run = self.runs[int(parts[5])]
            return {'path': run['path'], 'event': run['event'], 'status': run['status'], 'conclusion': run['conclusion'],
                    'repository': {'full_name': REPO}, 'head_repository': {'full_name': run['head_repo']} if run['head_repo'] else None,
                    'head_sha': run['head_sha']}
        if parts[4] == 'runs' and parts[6] == 'jobs':
            jobs = self.runs[int(parts[5])]['jobs']
            return {'total_count': len(jobs), 'jobs': [{'name': n, 'conclusion': c} for n, c in jobs]}
        if parts[3] == 'git' and parts[4] == 'commits':
            sha = parts[5]
            for run in self.runs.values():
                if run['head_sha'] == sha:
                    return {'tree': {'sha': run['head_tree']}}
        raise AssertionError(f'想定外の呼び出し: {path}')

    def api_bytes(self, path):
        self.calls.append(path)
        artifact_id = int(path.split('/')[-2])
        return next(a['zip'] for a in self.artifacts if a['id'] == artifact_id)


class Scratch(unittest.TestCase):
    def setUp(self):
        self._directory = tempfile.TemporaryDirectory()
        self.addCleanup(self._directory.cleanup)
        self.tmp = Path(self._directory.name)


class Targets(unittest.TestCase):
    @staticmethod
    def names(config, enabled):
        return [t['target'] for t in dist.select_targets(config, enabled)]

    def test_windows_always_and_the_others_by_their_inputs(self):
        config = dist.load_targets()
        self.assertEqual(self.names(config, set()), [WINDOWS])
        self.assertEqual(self.names(config, {'linux'}), [WINDOWS, LINUX])
        self.assertEqual(self.names(config, {'macos'}), [WINDOWS, MACOS])
        self.assertEqual(self.names(config, {'linux', 'macos'}), [WINDOWS, LINUX, MACOS])

    def test_macos_is_on_unless_the_release_turns_it_off_and_linux_is_off_unless_turned_on(self):
        config = dist.load_targets()
        # 入力が無いとき（PR の CI）は、入力の既定が入の対象（macOS）までビルドする。Linux はビルドしない
        self.assertEqual(dist.default_inputs(config), {'macos'})
        self.assertEqual(self.names(config, dist.resolve_inputs(config, [])), [WINDOWS, MACOS])
        # 配布の入力: macos=false で外し、linux=true で足す。言わなかった入力は既定のまま
        self.assertEqual(self.names(config, dist.resolve_inputs(config, ['linux=false', 'macos=true'])), [WINDOWS, MACOS])
        self.assertEqual(self.names(config, dist.resolve_inputs(config, ['linux=false', 'macos=false'])), [WINDOWS])
        self.assertEqual(self.names(config, dist.resolve_inputs(config, ['linux=true'])), [WINDOWS, LINUX, MACOS])
        self.assertEqual(self.names(config, dist.resolve_inputs(config, ['macos=false', 'linux=true'])), [WINDOWS, LINUX])

    def test_matrix_carries_the_runner_and_the_rust_targets_of_a_universal_build(self):
        config = dist.load_targets()
        matrix = dist.plan(config, dist.resolve_inputs(config, []))
        self.assertEqual(matrix, {'include': [
            {'os': 'windows-latest', 'target': WINDOWS, 'installer': True},
            {'os': 'macos-14', 'target': MACOS, 'installer': False,
             'rust_targets': 'aarch64-apple-darwin x86_64-apple-darwin', 'experimental': True, 'timeout': 90},
        ]})
        # macOS を外した配布の matrix は Windows だけ（Rust のターゲットの項目も無い）
        self.assertEqual(dist.plan(config, dist.resolve_inputs(config, ['macos=false'])),
                         {'include': [{'os': 'windows-latest', 'target': WINDOWS, 'installer': True}]})

    def test_only_macos_is_experimental_and_has_the_longer_time_limit(self):
        config = dist.load_targets()
        self.assertEqual(dist.experimental_targets(config), {MACOS})
        self.assertEqual(dist.experimental_targets(config, [WINDOWS, LINUX]), set())
        self.assertEqual(dist.experimental_targets(config, [WINDOWS, MACOS]), {MACOS})
        matrix = dist.plan(config, dist.resolve_inputs(config, ['linux=true']))
        by_target = {item['target']: item for item in matrix['include']}
        # Windows・Linux は試作でも時間の指定もなく（今までどおり）、macOS だけ試作で 90 分
        for target in (WINDOWS, LINUX):
            self.assertNotIn('experimental', by_target[target])
            self.assertNotIn('timeout', by_target[target])
        self.assertEqual((by_target[MACOS]['experimental'], by_target[MACOS]['timeout']), (True, 90))

    def test_the_artifact_pattern_keeps_only_the_planned_targets(self):
        self.assertEqual(dist.artifact_pattern(TREE, [WINDOWS]), f'dist-{TREE}-{WINDOWS}')
        self.assertEqual(dist.artifact_pattern(TREE, [WINDOWS, MACOS]), f'dist-{TREE}-{{{WINDOWS},{MACOS}}}')

    def test_unknown_input_is_refused(self):
        with self.assertRaises(dist.DistError):
            dist.select_targets(dist.load_targets(), {'windows'})
        with self.assertRaises(dist.DistError):
            dist.resolve_inputs(dist.load_targets(), ['windows=false'])

    def test_plan_command_accepts_only_true_false_and_known_names(self):
        for bad in (['--input', 'linux=yes'], ['--input', 'windows=false'], ['--input', 'ios=true'], ['--input', 'macos']):
            with self.assertRaises(dist.DistError, msg=bad):
                dist.cmd_plan(type('A', (), {'input': [bad[1]]})())

    def test_plan_command_writes_the_matrix_the_targets_and_the_tree(self):
        with tempfile.TemporaryDirectory() as d:
            output = Path(d) / 'out'
            for items, expected in (([], f'{WINDOWS},{MACOS}'), (['macos=false'], WINDOWS), (['linux=true'], f'{WINDOWS},{LINUX},{MACOS}')):
                output.write_text('')
                with unittest.mock.patch.dict(os.environ, {'GITHUB_OUTPUT': str(output)}):
                    dist.cmd_plan(type('A', (), {'input': items})())
                lines = dict(line.split('=', 1) for line in output.read_text().splitlines())
                self.assertEqual(lines['targets'], expected)
                self.assertEqual(lines['experimental'], MACOS if MACOS in expected else '')
                self.assertEqual(lines['pattern'], dist.artifact_pattern(lines['tree'], expected.split(',')))
                self.assertEqual(json.loads(lines['matrix'])['include'][-1]['target'], expected.split(',')[-1])
                self.assertEqual(len(lines['tree']), 40)

    def test_duplicate_or_incomplete_lists_are_refused(self):
        with tempfile.TemporaryDirectory() as d:
            path = Path(d) / 't.json'
            path.write_text(json.dumps({'schema': 1, 'targets': [{'target': 'a', 'os': 'x', 'installer': False, 'input': None}] * 2}))
            with self.assertRaises(dist.DistError):
                dist.load_targets(path)
            path.write_text(json.dumps({'schema': 1, 'targets': [{'target': 'a'}]}))
            with self.assertRaises(dist.DistError):
                dist.load_targets(path)
            path.write_text(json.dumps({'schema': 2, 'targets': []}))
            with self.assertRaises(dist.DistError):
                dist.load_targets(path)
            # default は真偽値で、入力のある対象だけ。rust_targets は文字列
            base = {'target': 'a', 'os': 'x', 'installer': False}
            for extra in ({'input': 'i', 'default': 'yes'}, {'input': None, 'default': True}, {'input': 'i', 'rust_targets': ['a']}):
                path.write_text(json.dumps({'schema': 1, 'targets': [{**base, **extra}]}))
                with self.assertRaises(dist.DistError, msg=extra):
                    dist.load_targets(path)
            for extra in ({'input': None, 'experimental': 'yes'}, {'input': None, 'timeout_minutes': 0}, {'input': None, 'timeout_minutes': '90'},
                          {'input': None, 'timeout_minutes': True}):
                path.write_text(json.dumps({'schema': 1, 'targets': [{**base, **extra}]}))
                with self.assertRaises(dist.DistError, msg=extra):
                    dist.load_targets(path)
            path.write_text(json.dumps({'schema': 1, 'targets': [{**base, 'input': None, 'experimental': True, 'timeout_minutes': 90}]}))
            self.assertEqual(dist.experimental_targets(dist.load_targets(path)), {'a'})
            path.write_text(json.dumps({'schema': 1, 'targets': [{**base, 'input': 'i', 'default': True, 'rust_targets': 'a b'}]}))
            self.assertEqual(dist.default_inputs(dist.load_targets(path)), {'i'})


class Catalog(Scratch):
    def test_catalog_lists_every_file_with_sha256_and_not_itself(self):
        (self.tmp / 'a.zip').write_bytes(b'one')
        (self.tmp / 'b-setup.exe').write_bytes(b'two')
        env = {'GITHUB_REPOSITORY': REPO, 'GITHUB_RUN_ID': '9', 'YOLUPAINTER_UPDATE_PUBLIC_KEY': f' {KEY}\n', 'RUSTC_VERSION': 'rustc 1.0'}
        catalog = dist.build_catalog(self.tmp, WINDOWS, TREE, 'e' * 40, '1.2.3', env)
        self.assertEqual(catalog['files']['a.zip'], {'sha256': hashlib.sha256(b'one').hexdigest(), 'size': 3})
        self.assertEqual(set(catalog['files']), {'a.zip', 'b-setup.exe'})
        self.assertEqual((catalog['tree'], catalog['target'], catalog['version'], catalog['update_public_key']), (TREE, WINDOWS, '1.2.3', KEY))
        # 目録を書いたあとにもう一度作っても、目録自身は載らない。
        (self.tmp / 'catalog.json').write_text('{}')
        self.assertEqual(set(dist.build_catalog(self.tmp, WINDOWS, TREE, 'e' * 40, '1.2.3', env)['files']), {'a.zip', 'b-setup.exe'})

    def test_empty_folder_or_a_directory_inside_is_refused(self):
        with self.assertRaises(dist.DistError):
            dist.build_catalog(self.tmp, WINDOWS, TREE, 'e' * 40, '1', {})
        (self.tmp / 'sub').mkdir()
        with self.assertRaises(dist.DistError):
            dist.build_catalog(self.tmp, WINDOWS, TREE, 'e' * 40, '1', {})

    def test_artifact_name_has_the_tree_and_the_target(self):
        self.assertEqual(dist.artifact_name(TREE, WINDOWS), f'dist-{TREE}-{WINDOWS}')


class Revision(Scratch):
    HEAD = 'e' * 40  # PR の CI では merge の commit
    PR_HEAD = '1234567' + 'a' * 33

    def test_a_pull_request_embeds_the_pull_request_head_not_the_merge_commit(self):
        full, short = dist.embedded_revision({'PR_HEAD_SHA': self.PR_HEAD}, self.HEAD)
        self.assertEqual((full, short), (self.PR_HEAD, '1234567'))

    def test_without_a_pull_request_the_head_is_embedded(self):
        for environment in ({}, {'PR_HEAD_SHA': ''}, {'PR_HEAD_SHA': ' \n'}):
            with self.subTest(environment):
                self.assertEqual(dist.embedded_revision(environment, self.HEAD), (self.HEAD, 'eeeeeee'))

    def test_a_malformed_pull_request_head_is_refused_instead_of_falling_back_to_the_merge_commit(self):
        for given in ('main', 'a' * 39, 'g' * 40, '1234567 --evil'):
            with self.subTest(given), self.assertRaises(dist.DistError):
                dist.embedded_revision({'PR_HEAD_SHA': given}, self.HEAD)
        with self.assertRaises(dist.DistError):
            dist.embedded_revision({}, 'not a sha')

    def test_the_command_hands_both_ids_to_the_following_steps(self):
        env_file = self.tmp / 'github.env'
        old = dict(os.environ)
        os.environ.update(PR_HEAD_SHA=self.PR_HEAD, GITHUB_ENV=str(env_file), GITHUB_STEP_SUMMARY=str(self.tmp / 'summary.md'))
        original = dist.git
        dist.git = lambda *args, **kwargs: self.HEAD
        try:
            with contextlib.redirect_stdout(io.StringIO()):
                dist.cmd_revision(None)
        finally:
            dist.git = original
            os.environ.clear()
            os.environ.update(old)
        self.assertEqual(env_file.read_text().splitlines(), ['YOLU_GIT_REV=1234567', f'YOLU_REVISION={self.PR_HEAD}'])

    def test_the_catalog_records_where_the_embedded_id_came_from(self):
        (self.tmp / 'a.zip').write_bytes(b'one')
        catalog = dist.build_catalog(self.tmp, WINDOWS, TREE, self.HEAD, '1.2.3', {'YOLU_REVISION': self.PR_HEAD})
        self.assertEqual((catalog['commit'], catalog['revision']), (self.HEAD, self.PR_HEAD))
        # 手元・古い呼び出しなど、元の ID が渡されないときは commit と同じ
        self.assertEqual(dist.build_catalog(self.tmp, WINDOWS, TREE, self.HEAD, '1.2.3', {})['revision'], self.HEAD)


class Verify(Scratch):
    def folder(self, **kwargs):
        directory = self.tmp / 'dist'
        directory.mkdir()
        for name, data in make_artifact_files(WINDOWS, **kwargs).items():
            (directory / name).write_bytes(data)
        return directory

    def test_matching_folder_has_no_problems(self):
        self.assertEqual(dist.verify_catalog(self.folder(), TREE, WINDOWS, '1.2.3', KEY), [])

    def test_each_mismatch_is_named(self):
        # (成果物を作るときの引数, 照らす側の (木, 対象, 版, 鍵), 期待する語)
        cases = [
            (dict(tree=OTHER_TREE), (TREE, WINDOWS, '1.2.3', KEY), '木が違います'),
            (dict(), (TREE, LINUX, '1.2.3', KEY), '対象が違います'),
            (dict(), (TREE, WINDOWS, '9.9.9', KEY), '版が違います'),
            (dict(), (TREE, WINDOWS, '1.2.3', 'f' * 64), '公開鍵'),
        ]
        for index, (made, checked, text) in enumerate(cases):
            with self.subTest(text):
                directory = self.tmp / f'case-{index}'
                directory.mkdir()
                for name, data in make_artifact_files(WINDOWS, **made).items():
                    (directory / name).write_bytes(data)
                problems = dist.verify_catalog(directory, *checked)
                self.assertTrue(any(text in p for p in problems), problems)

    def test_tampered_missing_extra_and_catalog_problems(self):
        zip_name = f'yolupainter-1.2.3-{WINDOWS}.zip'
        directory = self.folder(tamper=zip_name)
        self.assertTrue(any('SHA-256' in p for p in dist.verify_catalog(directory, TREE, WINDOWS)))
        (directory / zip_name).unlink()
        self.assertTrue(any('ありません' in p for p in dist.verify_catalog(directory, TREE, WINDOWS)))
        (directory / 'stray.txt').write_text('x')
        self.assertTrue(any('目録に無い' in p for p in dist.verify_catalog(directory, TREE, WINDOWS)))
        (directory / 'catalog.json').unlink()
        self.assertTrue(any('目録（catalog.json）がありません' in p for p in dist.verify_catalog(directory, TREE, WINDOWS)))
        (directory / 'catalog.json').write_text('not json')
        self.assertTrue(any('読めません' in p for p in dist.verify_catalog(directory, TREE, WINDOWS)))
        (directory / 'catalog.json').write_text('{"schema": 1, "files": {}}')
        self.assertTrue(dist.verify_catalog(directory, TREE, WINDOWS))

    def test_catalog_file_names_cannot_leave_the_folder(self):
        directory = self.folder()
        catalog = json.loads((directory / 'catalog.json').read_text())
        catalog['files']['../escape'] = {'sha256': '0' * 64, 'size': 1}
        (directory / 'catalog.json').write_text(json.dumps(catalog))
        self.assertTrue(any('不正' in p for p in dist.verify_catalog(directory, TREE, WINDOWS)))


class Extract(Scratch):
    def test_extracts_flat_regular_files(self):
        dist.safe_extract(make_zip({'a.zip': b'x', 'catalog.json': b'{}'}), self.tmp / 'out')
        self.assertEqual({p.name for p in (self.tmp / 'out').iterdir()}, {'a.zip', 'catalog.json'})

    def test_unsafe_names_links_and_nested_folders_are_refused(self):
        for name in ('../x', '/abs', 'a/b.txt', 'a\\b', 'c:x'):
            with self.subTest(name):
                with self.assertRaises(dist.DistError):
                    dist.safe_extract(make_zip({name: b'x'}), self.tmp / 'o')
        buffer = io.BytesIO()
        with zipfile.ZipFile(buffer, 'w') as archive:
            info = zipfile.ZipInfo('link')
            info.external_attr = 0o120777 << 16
            archive.writestr(info, 'target')
        with self.assertRaises(dist.DistError):
            dist.safe_extract(buffer.getvalue(), self.tmp / 'o2')
        with self.assertRaises(dist.DistError):
            dist.safe_extract(b'not a zip', self.tmp / 'o3')

    def test_too_large_is_refused(self):
        old = dist.MAX_EXTRACTED
        dist.MAX_EXTRACTED = 10
        try:
            with self.assertRaises(dist.DistError):
                dist.safe_extract(make_zip({'a': b'x' * 11}), self.tmp / 'o')
        finally:
            dist.MAX_EXTRACTED = old


class Find(Scratch):
    def promoted(self, gh, targets=(WINDOWS,), **kwargs):
        return dist.find(gh, REPO, TREE, list(targets), self.tmp / 'work', KEY, **kwargs)

    def good(self, gh=None, run_id=100, targets=(WINDOWS,)):
        gh = gh or FakeGh()
        gh.add_run(run_id)
        for index, target in enumerate(targets):
            gh.add_artifact(run_id * 10 + index, run_id, target)
        return gh

    def test_promotes_a_successful_pull_request_run_with_the_same_tree(self):
        gh = self.good()
        decision = self.promoted(gh)
        self.assertTrue(decision.promote, decision.reasons)
        self.assertEqual(decision.run_id, 100)
        self.assertTrue((self.tmp / 'work' / f'dist-{TREE}-{WINDOWS}' / 'catalog.json').is_file())

    def test_rebuild_input_makes_no_api_call(self):
        gh = self.good()
        decision = self.promoted(gh, rebuild=True)
        self.assertFalse(decision.promote)
        self.assertEqual(gh.calls, [])
        self.assertIn('rebuild', decision.reasons[0])

    def test_no_artifact_for_this_tree(self):
        gh = FakeGh()
        gh.add_run(1)
        gh.add_artifact(10, 1, WINDOWS, tree=OTHER_TREE)
        decision = self.promoted(gh)
        self.assertFalse(decision.promote)
        self.assertIn('見つからない', decision.reasons[0])

    def test_each_run_problem_refuses_the_artifact_with_its_reason(self):
        cases = {
            'expired': (dict(), dict(expired=True), '期限切れ'),
            'other workflow': (dict(path='.github/workflows/release.yml'), dict(), 'CI のワークフロー'),
            'dispatch run': (dict(event='workflow_dispatch'), dict(), 'pull_request'),
            'failed run': (dict(conclusion='failure'), dict(), '成功していません'),
            'running': (dict(status='in_progress', conclusion=None), dict(), '成功していません'),
            'fork': (dict(head_repo='someone/repo'), dict(), '同じリポジトリ'),
            'deleted fork': (dict(head_repo=None), dict(), '同じリポジトリ'),
            'a job failed': (dict(jobs=[('linux', 'success'), ('windows', 'failure')]), dict(), 'windows（failure）'),
            'a job skipped': (dict(jobs=[('linux', 'success'), ('windows', 'skipped')]), dict(), 'windows（skipped）'),
            'no jobs': (dict(jobs=[]), dict(), 'ジョブの一覧'),
            'head tree differs': (dict(head_tree=OTHER_TREE), dict(), '先頭の木'),
        }
        for name, case in cases.items():
            run_args, artifact_args, text = case
            with self.subTest(name):
                gh = FakeGh()
                gh.add_run(1, **run_args)
                gh.add_artifact(10, 1, WINDOWS, **artifact_args)
                decision = self.promoted(gh)
                self.assertFalse(decision.promote)
                self.assertTrue(any(text in reason for reason in decision.reasons), decision.reasons)

    def test_path_with_a_ref_suffix_is_accepted_only_for_ci_yml(self):
        gh = FakeGh()
        gh.add_run(1, path='.github/workflows/ci.yml@refs/pull/3/merge')
        gh.add_artifact(10, 1, WINDOWS)
        self.assertTrue(self.promoted(gh).promote)
        gh = FakeGh()
        gh.add_run(1, path='.github/workflows/evil-ci.yml@refs/pull/3/merge')
        gh.add_artifact(10, 1, WINDOWS)
        self.assertFalse(self.promoted(gh).promote)

    def test_newest_acceptable_run_wins_and_a_bad_newer_run_is_skipped(self):
        gh = self.good(run_id=100)
        gh.add_run(200, conclusion='failure')
        gh.add_artifact(2000, 200, WINDOWS, created='2026-10-06T00:00:00Z')
        decision = self.promoted(gh)
        self.assertTrue(decision.promote)
        self.assertEqual(decision.run_id, 100)
        gh.add_run(300)
        gh.add_artifact(3000, 300, WINDOWS, created='2026-10-07T00:00:00Z')
        self.assertEqual(self.promoted(gh).run_id, 300)

    def test_all_targets_must_come_from_the_same_run(self):
        gh = FakeGh()
        gh.add_run(1)
        gh.add_run(2)
        gh.add_artifact(10, 1, WINDOWS)
        gh.add_artifact(20, 2, LINUX)
        decision = self.promoted(gh, targets=(WINDOWS, LINUX))
        self.assertFalse(decision.promote)
        self.assertIn('そろった', decision.reasons[-1])
        gh.add_artifact(11, 2, WINDOWS)
        decision = self.promoted(gh, targets=(WINDOWS, LINUX))
        self.assertTrue(decision.promote)
        self.assertEqual(decision.run_id, 2)

    def test_macos_travels_with_windows_and_a_release_without_it_still_takes_the_rest(self):
        gh = FakeGh()
        gh.add_run(1)
        gh.add_artifact(10, 1, WINDOWS)
        gh.add_artifact(11, 1, MACOS)
        decision = self.promoted(gh, targets=(WINDOWS, MACOS), optional={MACOS})
        self.assertTrue(decision.promote)
        self.assertEqual((decision.run_id, decision.notes), (1, []))
        # macos を切った配布: CI に macOS の成果物があっても、Windows だけで受け取る
        self.assertTrue(self.promoted(gh, targets=(WINDOWS,)).promote)
        # macOS の成果物が無い CI（macOS を足す前の PR）の成果物は、macOS が試作でなければ受け取らず、ビルドし直す
        decision = self.promoted(self.good(), targets=(WINDOWS, MACOS))
        self.assertFalse(decision.promote)
        self.assertTrue(any(MACOS in reason for reason in decision.reasons))

    MAC_JOB = '配る物のビルド / 配る物のビルド（universal-apple-darwin）'
    WINDOWS_JOB = '配る物のビルド / 配る物のビルド（x86_64-pc-windows-msvc）'

    def test_a_failed_experimental_build_does_not_stop_windows_from_being_taken(self):
        # macOS のビルドが落ちた CI の実行: 結論は（continue-on-error が効かなかった場合も）失敗で、macOS の成果物は無い
        for conclusion in ('success', 'failure'):
            with self.subTest(conclusion):
                gh = FakeGh()
                gh.add_run(1, conclusion=conclusion, jobs=[('linux', 'success'), (self.WINDOWS_JOB, 'success'), (self.MAC_JOB, 'failure')])
                gh.add_artifact(10, 1, WINDOWS)
                decision = self.promoted(gh, targets=(WINDOWS, MACOS), optional={MACOS})
                self.assertTrue(decision.promote, decision.reasons)
                self.assertEqual(decision.run_id, 1)
                # 載らない理由が言える（macOS の対象の名前と、成果物が無いこと）
                self.assertTrue(any(MACOS in note and '成果物' in note for note in decision.notes), decision.notes)
        # キャンセルや飛ばされた macOS のジョブも、試作の対象のものは数えない
        for state in ('cancelled', 'skipped', 'timed_out'):
            gh = FakeGh()
            gh.add_run(1, jobs=[('linux', 'success'), (self.WINDOWS_JOB, 'success'), (self.MAC_JOB, state)])
            gh.add_artifact(10, 1, WINDOWS)
            self.assertTrue(self.promoted(gh, targets=(WINDOWS, MACOS), optional={MACOS}).promote, state)

    def test_a_failed_windows_build_still_stops_even_when_the_experimental_one_passed_or_failed(self):
        for mac in ('success', 'failure'):
            gh = FakeGh()
            gh.add_run(1, conclusion='failure', jobs=[('linux', 'success'), (self.WINDOWS_JOB, 'failure'), (self.MAC_JOB, mac)])
            gh.add_artifact(10, 1, WINDOWS)
            gh.add_artifact(11, 1, MACOS)
            decision = self.promoted(gh, targets=(WINDOWS, MACOS), optional={MACOS})
            self.assertFalse(decision.promote, mac)
            self.assertTrue(any('x86_64-pc-windows-msvc' in reason for reason in decision.reasons), decision.reasons)
        # 試作の印が無ければ、macOS のジョブの失敗も今までどおり数える
        gh = FakeGh()
        gh.add_run(1, jobs=[(self.WINDOWS_JOB, 'success'), (self.MAC_JOB, 'failure')])
        gh.add_artifact(10, 1, WINDOWS)
        gh.add_artifact(11, 1, MACOS)
        self.assertFalse(self.promoted(gh, targets=(WINDOWS, MACOS)).promote)
        # 結論が失敗なのに、失敗したジョブが見つからない実行は受け取らない
        gh = FakeGh()
        gh.add_run(1, conclusion='failure', jobs=[(self.WINDOWS_JOB, 'success'), (self.MAC_JOB, 'success')])
        gh.add_artifact(10, 1, WINDOWS)
        self.assertFalse(self.promoted(gh, targets=(WINDOWS, MACOS), optional={MACOS}).promote)

    def test_an_experimental_artifact_that_does_not_check_out_is_left_out_not_fatal(self):
        gh = FakeGh()
        gh.add_run(1)
        gh.add_artifact(10, 1, WINDOWS)
        gh.add_artifact(11, 1, MACOS, tamper=f'yolupainter-1.2.3-{MACOS}.zip')
        decision = self.promoted(gh, targets=(WINDOWS, MACOS), optional={MACOS})
        self.assertTrue(decision.promote)
        self.assertTrue(any(MACOS in note and 'SHA-256' in note for note in decision.notes), decision.notes)
        # 期限切れも同じ
        gh = FakeGh()
        gh.add_run(1)
        gh.add_artifact(10, 1, WINDOWS)
        gh.add_artifact(11, 1, MACOS, expired=True)
        decision = self.promoted(gh, targets=(WINDOWS, MACOS), optional={MACOS})
        self.assertTrue(decision.promote)
        self.assertTrue(any(MACOS in note and '期限切れ' in note for note in decision.notes), decision.notes)
        # Windows の成果物が目録と違えば、試作が無事でも受け取らない
        gh = FakeGh()
        gh.add_run(1)
        gh.add_artifact(10, 1, WINDOWS, tamper=f'yolupainter-1.2.3-{WINDOWS}.zip')
        gh.add_artifact(11, 1, MACOS)
        self.assertFalse(self.promoted(gh, targets=(WINDOWS, MACOS), optional={MACOS}).promote)

    def test_a_missing_target_in_ci_means_rebuild(self):
        # Linux を足した配布で、CI は Windows だけビルドしている。
        gh = self.good()
        decision = self.promoted(gh, targets=(WINDOWS, LINUX))
        self.assertFalse(decision.promote)
        self.assertTrue(any(LINUX in reason for reason in decision.reasons))

    def test_digest_and_catalog_mismatches_fall_back_to_the_next_run_or_rebuild(self):
        gh = self.good(run_id=100)
        gh.add_run(200)
        gh.add_artifact(2000, 200, WINDOWS, created='2026-10-06T00:00:00Z')
        gh.corrupt_digest.add(2000)
        decision = self.promoted(gh)
        self.assertTrue(decision.promote)
        self.assertEqual(decision.run_id, 100)
        gh.corrupt_digest.add(1000)
        decision = self.promoted(gh)
        self.assertFalse(decision.promote)
        self.assertTrue(any('digest' in reason for reason in decision.reasons))

    def test_an_artifact_without_a_recorded_digest_is_not_promoted(self):
        # digest が無い（キーごと無い・null・空）成果物は確かめようが無いので受け取らない。次の実行があればそちら、無ければビルドし直す。
        for missing in ('absent', None, ''):
            with self.subTest(missing):
                gh = self.good(run_id=100)
                gh.add_run(200)
                gh.add_artifact(2000, 200, WINDOWS, created='2026-10-06T00:00:00Z')
                gh.missing_digest[2000] = missing
                decision = self.promoted(gh)
                self.assertTrue(decision.promote)
                self.assertEqual(decision.run_id, 100)
                gh.missing_digest[1000] = missing
                before = len(gh.calls)
                decision = self.promoted(gh)
                self.assertFalse(decision.promote)
                self.assertTrue(any('digest が無い' in reason for reason in decision.reasons), decision.reasons)
                # 確かめようが無い成果物の zip は落とさない
                self.assertFalse(any(call.endswith('/zip') for call in gh.calls[before:]))

    def test_catalog_with_another_key_or_tampered_file_is_not_promoted(self):
        gh = FakeGh()
        gh.add_run(1)
        gh.add_artifact(10, 1, WINDOWS, key='f' * 64)
        self.assertFalse(self.promoted(gh).promote)
        gh = FakeGh()
        gh.add_run(1)
        gh.add_artifact(10, 1, WINDOWS, tamper=f'yolupainter-1.2.3-{WINDOWS}.zip')
        decision = self.promoted(gh)
        self.assertFalse(decision.promote)
        self.assertTrue(any('SHA-256' in reason for reason in decision.reasons))

    def test_command_falls_back_to_a_build_when_the_lookup_itself_fails(self):
        class Broken:
            def api(self, path):
                raise dist.DistError('rate limit')

        outputs = self.tmp / 'out.txt'
        summary = self.tmp / 'summary.md'
        old = dict(os.environ)
        os.environ.update(GITHUB_OUTPUT=str(outputs), GITHUB_STEP_SUMMARY=str(summary))
        original = dist.Gh
        dist.Gh = Broken
        try:
            args = type('A', (), dict(repo=REPO, tree=TREE, targets=WINDOWS, work=str(self.tmp / 'w'), rebuild='false', optional=''))()
            with contextlib.redirect_stdout(io.StringIO()):
                dist.cmd_find(args)
        finally:
            dist.Gh = original
            os.environ.clear()
            os.environ.update(old)
        self.assertIn('promote=false', outputs.read_text())
        self.assertIn('rate limit', summary.read_text())


class Install(Scratch):
    def received(self, targets=(WINDOWS,), folder='received', **kwargs):
        for target in targets:
            directory = self.tmp / folder / f'dist-{TREE}-{target}'
            directory.mkdir(parents=True)
            for name, data in make_artifact_files(target, **kwargs).items():
                (directory / name).write_bytes(data)
        return self.tmp / folder

    def test_copies_the_assets_without_the_catalog(self):
        copied = dist.install(self.received(), TREE, [WINDOWS], self.tmp / 'dist', '1.2.3', KEY)
        self.assertEqual(sorted(copied), [f'yolupainter-1.2.3-{WINDOWS}-setup.exe', f'yolupainter-1.2.3-{WINDOWS}.zip'])
        self.assertEqual(sorted(p.name for p in (self.tmp / 'dist').iterdir()), sorted(copied))

    def test_two_targets_are_collected_and_a_missing_one_is_refused(self):
        received = self.received((WINDOWS, LINUX))
        copied = dist.install(received, TREE, [WINDOWS, LINUX], self.tmp / 'dist', '1.2.3', KEY)
        self.assertEqual(len(copied), 3)
        with self.assertRaises(dist.DistError) as error:
            dist.install(received, TREE, [WINDOWS, 'x86_64-apple-darwin'], self.tmp / 'dist2', '1.2.3', KEY)
        self.assertIn('成果物がありません', str(error.exception))

    def test_nothing_is_copied_when_any_target_does_not_match(self):
        received = self.received((WINDOWS, LINUX))
        (received / f'dist-{TREE}-{LINUX}' / f'yolupainter-1.2.3-{LINUX}.zip').write_bytes(b'tampered')
        with self.assertRaises(dist.DistError):
            dist.install(received, TREE, [WINDOWS, LINUX], self.tmp / 'dist', '1.2.3', KEY)
        self.assertFalse((self.tmp / 'dist').exists())

    def test_a_missing_or_broken_experimental_target_is_left_out_and_the_rest_is_collected(self):
        # 試作の対象（macOS）の成果物が無い: Windows だけ集めて、理由を返す
        received = self.received()
        omitted = {}
        copied = dist.install(received, TREE, [WINDOWS, MACOS], self.tmp / 'dist', '1.2.3', KEY, {MACOS}, omitted)
        self.assertEqual(len(copied), 2)
        self.assertEqual(list(omitted), [MACOS])
        self.assertIn('成果物がありません', omitted[MACOS][0])
        self.assertFalse(any(MACOS in p.name for p in (self.tmp / 'dist').iterdir()))
        # 目録に合わない macOS の成果物も同じ（SHA-256 の食い違いを理由に出す）
        received = self.received((WINDOWS, MACOS), 'received2')
        (received / f'dist-{TREE}-{MACOS}' / f'yolupainter-1.2.3-{MACOS}.zip').write_bytes(b'tampered')
        omitted = {}
        copied = dist.install(received, TREE, [WINDOWS, MACOS], self.tmp / 'dist2', '1.2.3', KEY, {MACOS}, omitted)
        self.assertEqual(len(copied), 2)
        self.assertIn('SHA-256', omitted[MACOS][0])
        # macOS が無事なら、両方を集め、理由は何も無い
        received = self.received((WINDOWS, MACOS), 'received3')
        omitted = {}
        copied = dist.install(received, TREE, [WINDOWS, MACOS], self.tmp / 'dist3', '1.2.3', KEY, {MACOS}, omitted)
        self.assertEqual((len(copied), omitted), (3, {}))

    def test_a_broken_required_target_still_stops_even_with_an_experimental_one_beside_it(self):
        received = self.received((WINDOWS, MACOS))
        (received / f'dist-{TREE}-{WINDOWS}' / f'yolupainter-1.2.3-{WINDOWS}.zip').write_bytes(b'tampered')
        with self.assertRaises(dist.DistError):
            dist.install(received, TREE, [WINDOWS, MACOS], self.tmp / 'dist', '1.2.3', KEY, {MACOS}, {})
        self.assertFalse((self.tmp / 'dist').exists())
        # Windows の成果物が無いときも止まる（macOS だけの下書きは作らない）
        with self.assertRaises(dist.DistError):
            dist.install(self.received((MACOS,), 'only'), TREE, [WINDOWS, MACOS], self.tmp / 'dist3', '1.2.3', KEY, {MACOS}, {})

    def test_the_command_says_in_the_summary_why_the_experimental_target_is_not_in(self):
        received = self.received()
        outputs = self.tmp / 'summary.md'
        old = dict(os.environ)
        os.environ.update(GITHUB_STEP_SUMMARY=str(outputs))
        try:
            args = type('A', (), dict(received=str(received), tree=TREE, targets=f'{WINDOWS},{MACOS}', out=str(self.tmp / 'dist'),
                                      version='1.2.3', update_public_key=KEY, optional=MACOS))()
            with contextlib.redirect_stdout(io.StringIO()):
                dist.cmd_install(args)
        finally:
            os.environ.clear()
            os.environ.update(old)
        text = outputs.read_text(encoding='utf-8')
        self.assertIn(f'{MACOS}（試作）は載らなかった', text)
        self.assertIn('成果物がありません', text)
        self.assertIn(f'yolupainter-1.2.3-{WINDOWS}.zip', text)

    def test_version_and_key_must_match_the_current_ones(self):
        received = self.received()
        for version, key in (('9.9.9', KEY), ('1.2.3', 'f' * 64)):
            with self.subTest((version, key)):
                with self.assertRaises(dist.DistError):
                    dist.install(received, TREE, [WINDOWS], self.tmp / 'dist', version, key)
                self.assertFalse((self.tmp / 'dist').exists())


class CiOk(Scratch):
    """main-tested.yml が使う `ci-ok`: PR の先頭の CI の実行が成功か。試作の対象のジョブだけが落ちた実行は成功とみなす。"""

    def run_command(self, gh, head='a' * 40):
        original = dist.Gh
        dist.Gh = lambda: gh
        errors = io.StringIO()
        try:
            with contextlib.redirect_stdout(io.StringIO()) as out, contextlib.redirect_stderr(errors):
                code = dist.main(['ci-ok', '--repo', REPO, '--head', head])
        finally:
            dist.Gh = original
        return code, out.getvalue(), errors.getvalue()

    class Listing(FakeGh):
        def api(self, path):
            route, _, _ = path.partition('?')
            if route == f'repos/{REPO}/actions/workflows/ci.yml/runs':
                return {'workflow_runs': [{'id': run_id, 'status': run['status'], 'conclusion': run['conclusion']}
                                          for run_id, run in list(self.runs.items())[:1]]}
            if route.endswith('/jobs'):
                jobs = self.runs[int(route.split('/')[-2])]['jobs']
                return {'total_count': len(jobs), 'jobs': [{'name': n, 'conclusion': c} for n, c in jobs]}
            raise AssertionError(path)

    MAC_JOB = Find.MAC_JOB
    WINDOWS_JOB = Find.WINDOWS_JOB

    def gh(self, **kwargs):
        gh = self.Listing()
        gh.add_run(1, **kwargs)
        return gh

    def test_a_successful_run_is_ok_and_a_missing_one_is_not(self):
        code, out, _ = self.run_command(self.gh(jobs=[(self.WINDOWS_JOB, 'success'), (self.MAC_JOB, 'success')]))
        self.assertEqual(code, 0)
        self.assertIn('成功しています', out)
        code, _, err = self.run_command(self.Listing())
        self.assertEqual(code, 1)
        self.assertIn('見つかりません', err)

    def test_only_the_experimental_job_failing_still_counts_as_passed(self):
        for conclusion in ('failure', 'success'):
            code, out, _ = self.run_command(self.gh(conclusion=conclusion, jobs=[(self.WINDOWS_JOB, 'success'), (self.MAC_JOB, 'failure')]))
            self.assertEqual(code, 0, conclusion)
            self.assertIn('試作の対象の失敗は数えていない', out)

    def test_any_other_failure_or_an_unfinished_run_is_not_ok(self):
        for name, kwargs in {
            'windows failed': dict(conclusion='failure', jobs=[(self.WINDOWS_JOB, 'failure'), (self.MAC_JOB, 'success')]),
            'both failed': dict(conclusion='failure', jobs=[(self.WINDOWS_JOB, 'failure'), (self.MAC_JOB, 'failure')]),
            'a test job failed': dict(conclusion='failure', jobs=[('linux', 'failure'), (self.MAC_JOB, 'failure')]),
            'in progress': dict(status='in_progress', conclusion=None, jobs=[(self.WINDOWS_JOB, 'success')]),
            'failure without a failed job': dict(conclusion='failure', jobs=[(self.WINDOWS_JOB, 'success')]),
            'cancelled': dict(conclusion='cancelled', jobs=[(self.WINDOWS_JOB, 'success')]),
        }.items():
            with self.subTest(name):
                code, _, err = self.run_command(self.gh(**kwargs))
                self.assertEqual(code, 1)
                self.assertTrue(err.strip(), name)
                if name == 'windows failed':
                    self.assertIn('x86_64-pc-windows-msvc', err)


def yaml_edit(mutate):
    """YAML を読んで dict のまま書き換えて書き戻す。Action の SHA・字下げ・並びに依らない（SHA を更新しても、別のワークフローを足しても試験は動く）。
    書き換えが何も変えなかった（狙った場所が無かった）ときは落とす。"""
    import copy

    def edit(text):
        document = check_workflows.yaml.safe_load(text)
        before = copy.deepcopy(document)
        mutate(document)
        assert document != before, '書き換えの狙った場所が見つからない'
        return check_workflows.yaml.safe_dump(document, sort_keys=False, allow_unicode=True)
    return edit


def all_steps(document):
    for job in document['jobs'].values():
        yield from job.get('steps') or []


def first_step_using(document, action):
    return next(s for s in all_steps(document) if str(s.get('uses', '')).startswith(action + '@'))


class Workflows(unittest.TestCase):
    @unittest.skipIf(check_workflows.yaml is None, 'PyYAML が無い')
    def test_this_repositorys_workflows_pass(self):
        documents, problems = check_workflows.check(ROOT)
        self.assertEqual(problems, [])
        # 別のワークフロー（コード解析など）を足しても落ちない。決まりの対象の 4 本があることだけ確かめる。
        self.assertLessEqual({'ci.yml', 'release.yml', 'dist-build.yml', 'beta-channel.yml'}, set(documents))

    @unittest.skipIf(check_workflows.yaml is None, 'PyYAML が無い')
    def test_each_rule_fails_on_a_broken_copy(self):
        import shutil
        switch = next(t['input'] for t in json.loads((ROOT / 'tools/dist-targets.json').read_text(encoding='utf-8'))['targets'] if t['input'])

        def add_push(document):
            on = document['on'] if 'on' in document else document[True]
            on['push'] = {'branches': ['main']}

        def unpin(document):
            step = first_step_using(document, 'actions/checkout')
            step['uses'] = 'actions/checkout@v6'

        def add_cache(document):
            first_step_using(document, 'actions/setup-python').setdefault('with', {})['cache'] = 'pip'

        def switch_on(document):
            on = document['on'] if 'on' in document else document[True]
            on['workflow_dispatch']['inputs'][switch]['default'] = True

        def stale_input(document):
            on = document['on'] if 'on' in document else document[True]
            on['workflow_dispatch']['inputs']['windows'] = {'description': 'x', 'required': True, 'type': 'boolean', 'default': False}

        def macos_off(document):
            on = document['on'] if 'on' in document else document[True]
            on['workflow_dispatch']['inputs']['macos']['default'] = False

        def plan_forgets_macos(document):
            step = document['jobs']['plan']['steps'][-1]
            step['run'] = step['run'].replace(' --input macos=${{ inputs.macos }}', '')

        def build_job(document):
            return document['jobs']['build']

        def no_continue_on_error(document):
            del build_job(document)['continue-on-error']

        def fixed_timeout(document):
            build_job(document)['timeout-minutes'] = 60

        def single_rust_step_only(document):
            steps = build_job(document)['steps']
            build_job(document)['steps'] = [st for st in steps if str(st.get('if', '')).replace(' ', '') != '${{matrix.rust_targets}}']

        def rust_targets_not_added(document):
            for st in build_job(document)['steps']:
                if str(st.get('if', '')).replace(' ', '') == '${{matrix.rust_targets}}':
                    st['run'] = st['run'].replace('rustup target add --toolchain stable ${{ matrix.rust_targets }}', 'echo skip')

        def rust_step_passes_the_artifact_name(document):
            for st in build_job(document)['steps']:
                if str(st.get('if', '')).replace(' ', '') == '${{!matrix.rust_targets}}':
                    st['run'] = st['run'].replace('--target ${{ matrix.target }}', '')

        def metadata_waits_for_build(document):
            document['jobs']['metadata']['if'] = "${{ !cancelled() && needs.find.result == 'success' && needs.build.result == 'success' }}"

        def find_forgets_optional(document):
            for st in document['jobs']['find']['steps']:
                if 'run' in st:
                    st['run'] = st['run'].replace(' --optional "$OPTIONAL"', '')

        def install_forgets_optional(document):
            for st in document['jobs']['metadata']['steps']:
                if 'tools/dist.py install' in str(st.get('run', '')):
                    st['run'] = st['run'].replace(' --optional "$OPTIONAL"', '')

        def broad_pattern(document):
            for st in document['jobs']['metadata']['steps']:
                if str(st.get('uses', '')).startswith('actions/download-artifact@'):
                    st['with']['pattern'] = 'dist-${{ needs.plan.outputs.tree }}-*'

        def plan_loses_pattern(document):
            del document['jobs']['plan']['outputs']['pattern']

        def tested_counts_the_experimental(document):
            for st in document['jobs']['tested']['steps']:
                if 'run' in st:
                    st['run'] = st['run'].replace('tools/dist.py ci-ok', 'echo')

        def not_called(document):
            for job in document['jobs'].values():
                if job.get('uses') == './.github/workflows/dist-build.yml':
                    job['uses'] = './.github/workflows/other.yml'

        def lose_condition(document):
            for job in document['jobs'].values():
                if "github.base_ref == 'main'" in str(job.get('if', '')):
                    job['if'] = str(job['if']).replace("github.base_ref == 'main'", "github.base_ref == 'dev'")

        def on_of(document):
            return document['on'] if 'on' in document else document[True]

        def beta_dispatch(document):
            on_of(document)['workflow_dispatch'] = {}

        def beta_prereleased(document):
            on_of(document)['release']['types'] = ['prereleased', 'published']

        def beta_write_everywhere(document):
            document['permissions'] = {'contents': 'write'}

        def beta_no_prefix(document):
            for job in document['jobs'].values():
                job['if'] = str(job['if']).replace(" && startsWith(github.event.release.tag_name, 'v')", '')

        def beta_stable_too(document):
            for job in document['jobs'].values():
                job['if'] = str(job['if']).replace('github.event.release.prerelease', 'true')

        def beta_other_tag(document):
            document['env']['CHANNEL_TAG'] = 'updater-beta-2'

        def beta_cache(document):
            first_step_using(document, 'actions/checkout').setdefault('with', {})['cache'] = 'x'

        cases = {
            'beta channel: also on dispatch': ('beta-channel.yml', yaml_edit(beta_dispatch), 'published だけ'),
            'beta channel: prereleased': ('beta-channel.yml', yaml_edit(beta_prereleased), 'published だけ'),
            'beta channel: write everywhere': ('beta-channel.yml', yaml_edit(beta_write_everywhere), 'contents: read だけ'),
            'beta channel: runs on its own release': ('beta-channel.yml', yaml_edit(beta_no_prefix), "startsWith(github.event.release.tag_name, 'v')"),
            'beta channel: runs for stable': ('beta-channel.yml', yaml_edit(beta_stable_too), 'github.event.release.prerelease'),
            'beta channel: tag drift': ('beta-channel.yml', yaml_edit(beta_other_tag), 'BETA_CHANNEL_TAG'),
            'beta channel: schema drift': ('beta-channel.yml', lambda t: t.replace('updater-v1.json', 'updater-v2.json'), 'UPDATER_FILE'),
            'beta channel: cache': ('beta-channel.yml', yaml_edit(beta_cache), 'キャッシュ'),
            'push trigger': ('ci.yml', yaml_edit(add_push), 'push で動かさない'),
            'unpinned action': ('ci.yml', yaml_edit(unpin), '固定されていません'),
            'cache in dist': ('dist-build.yml', yaml_edit(add_cache), 'キャッシュ'),
            'switch default on': ('release.yml', yaml_edit(switch_on), '既定が切'),
            'stale input': ('release.yml', yaml_edit(stale_input), '使う対象が'),
            'macos default off': ('release.yml', yaml_edit(macos_off), '入力 macos は既定が入'),
            'plan forgets macos': ('release.yml', yaml_edit(plan_forgets_macos), 'plan が入力 macos を渡していません'),
            'experimental build is required': ('dist-build.yml', yaml_edit(no_continue_on_error), 'continue-on-error'),
            'fixed time limit': ('dist-build.yml', yaml_edit(fixed_timeout), 'timeout-minutes'),
            'no multi-target rust step': ('dist-build.yml', yaml_edit(single_rust_step_only), 'rustup target add'),
            'rust targets not added': ('dist-build.yml', yaml_edit(rust_targets_not_added), 'rustup target add'),
            'rust step passes the artifact name': ('dist-build.yml', yaml_edit(rust_step_passes_the_artifact_name), '--target ${{ matrix.target }}'),
            'metadata waits for build': ('release.yml', yaml_edit(metadata_waits_for_build), 'build の結果を見ています'),
            'find forgets optional': ('release.yml', yaml_edit(find_forgets_optional), 'find が'),
            'install forgets optional': ('release.yml', yaml_edit(install_forgets_optional), 'metadata が'),
            'broad artifact pattern': ('release.yml', yaml_edit(broad_pattern), 'pattern'),
            'plan loses the pattern': ('release.yml', yaml_edit(plan_loses_pattern), '出力 pattern'),
            'main-tested counts the experimental': ('main-tested.yml', yaml_edit(tested_counts_the_experimental), 'ci-ok'),
            'dist-build not called': ('ci.yml', yaml_edit(not_called), '呼んでいません'),
            'condition lost': ('ci.yml', yaml_edit(lose_condition), 'github.base_ref'),
            'broken yaml': ('release.yml', lambda t: t + '\n  bad: [\n', 'YAML を読めません'),
            # 状態の関数の無い下書きの条件は、成果物を受け取って build が skipped の回に下書きまで飛ばす
            'draft skipped with build': ('release.yml', lambda t: t.replace(
                "if: ${{ !cancelled() && !inputs.dry-run && needs.metadata.result == 'success' }}",
                'if: ${{ !inputs.dry-run }}'), "draft の条件に !cancelled()"),
        }
        for name, (file, edit, text) in cases.items():
            with self.subTest(name), tempfile.TemporaryDirectory() as d:
                root = Path(d)
                (root / '.github/workflows').mkdir(parents=True)
                (root / 'tools').mkdir()
                shutil.copy(ROOT / 'tools/dist-targets.json', root / 'tools/dist-targets.json')
                (root / 'crates/yolu-update/src').mkdir(parents=True)
                shutil.copy(ROOT / 'crates/yolu-update/src/lib.rs', root / 'crates/yolu-update/src/lib.rs')
                for path in (ROOT / '.github/workflows').glob('*.yml'):
                    shutil.copy(path, root / '.github/workflows' / path.name)
                target = root / '.github/workflows' / file
                original = target.read_text(encoding='utf-8')
                target.write_text(edit(original), encoding='utf-8')
                self.assertNotEqual(original, target.read_text(encoding='utf-8'), name)
                _, problems = check_workflows.check(root)
                self.assertTrue(any(text in p for p in problems), (name, problems))
                # 壊した 1 つ以外の決まりには触れていない（書き戻しで元の決まりを壊していない）。
                self.assertLessEqual(len(problems), 2, (name, problems))

    @unittest.skipIf(check_workflows.yaml is None, 'PyYAML が無い')
    def test_the_beta_channel_workflow_and_the_library_constants_are_required(self):
        import shutil
        for name, remove, text in [
            ('no workflow', '.github/workflows/beta-channel.yml', 'beta-channel.yml がありません'),
            ('no library', 'crates/yolu-update/src/lib.rs', 'BETA_CHANNEL_TAG・UPDATER_FILE が読めません'),
        ]:
            with self.subTest(name), tempfile.TemporaryDirectory() as d:
                root = Path(d)
                (root / '.github/workflows').mkdir(parents=True)
                (root / 'tools').mkdir()
                (root / 'crates/yolu-update/src').mkdir(parents=True)
                shutil.copy(ROOT / 'tools/dist-targets.json', root / 'tools/dist-targets.json')
                shutil.copy(ROOT / 'crates/yolu-update/src/lib.rs', root / 'crates/yolu-update/src/lib.rs')
                for path in (ROOT / '.github/workflows').glob('*.yml'):
                    shutil.copy(path, root / '.github/workflows' / path.name)
                (root / remove).unlink()
                _, problems = check_workflows.check(root)
                self.assertTrue(any(text in p for p in problems), (name, problems))

    @unittest.skipIf(check_workflows.yaml is None, 'PyYAML が無い')
    def test_command_exit_codes(self):
        run = lambda *a: subprocess.run([sys.executable, str(ROOT / 'tools/check-workflows.py'), *a], capture_output=True, text=True)
        self.assertEqual(run().returncode, 0)
        with tempfile.TemporaryDirectory() as d:
            (Path(d) / 'tools').mkdir()
            (Path(d) / '.github/workflows').mkdir(parents=True)
            (Path(d) / 'tools/dist-targets.json').write_text((ROOT / 'tools/dist-targets.json').read_text())
            self.assertEqual(run('--root', d).returncode, 1)


if __name__ == '__main__':
    unittest.main(verbosity=1)
