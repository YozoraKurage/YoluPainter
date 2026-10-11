#!/usr/bin/env python3
"""全文の欠落・変更・未承認が配布用の成功にならないことを確かめる。"""
import hashlib
import importlib.util
import io
import json
from pathlib import Path, PureWindowsPath
import subprocess
import tempfile
import unittest
import sys
sys.dont_write_bytecode = True
from unittest.mock import Mock, patch

ROOT = Path(__file__).resolve().parents[1]
spec = importlib.util.spec_from_file_location('third_party', ROOT / 'tools/third-party.py')
licenses = importlib.util.module_from_spec(spec)
spec.loader.exec_module(licenses)


class LicenseChecks(unittest.TestCase):
    def setUp(self):
        (ROOT / 'target').mkdir(exist_ok=True)
        self.directory = tempfile.TemporaryDirectory(dir=ROOT / 'target')
        self.addCleanup(self.directory.cleanup)
        self.root = Path(self.directory.name)
        (self.root / 'LICENSE').write_text('試験用の原文')
        self.package = {'name': 'fixture', 'version': '1.0.0', 'license': 'MIT',
                        'id': 'fixture-id', 'source': 'registry+fixture', 'repository': None,
                        'manifest_path': str(self.root / 'Cargo.toml')}
        self.metadata = {'packages': [self.package], 'workspace_members': []}
        self.review = {'declared': 'MIT', 'selected': ['MIT'], 'blocked': [], 'files': [{
            'path': 'LICENSE', 'sha256': hashlib.sha256((self.root / 'LICENSE').read_bytes()).hexdigest()}]}
        self.config = {'crates': {'fixture@1.0.0': self.review}}

    def inspect(self):
        with patch.object(licenses, 'dependency_keys', return_value={'fixture@1.0.0'}):
            return licenses.inventory('fixture', self.metadata, self.config, True)

    def test_verified_text(self):
        rows, texts, errors = self.inspect()
        self.assertFalse(errors)
        self.assertIn('試験用の原文', texts[0])

    def repo_text(self, content='[上流の記載から組み立てた表示]\nCopyright (c) 作者\n'):
        folder = self.root / 'tools/license-texts'
        folder.mkdir(parents=True, exist_ok=True)
        path = folder / 'mit.txt'
        path.write_text(content, encoding='utf-8')
        return {'repo': 'tools/license-texts/mit.txt', 'sha256': hashlib.sha256(path.read_bytes()).hexdigest()}

    def test_a_text_kept_in_the_repository_is_verified_and_named_as_such(self):
        spec = self.repo_text()
        with patch.object(licenses, 'ROOT', self.root):
            origin, text = licenses.read_source(self.package, spec, True)
            self.assertEqual(origin, 'repo:tools/license-texts/mit.txt')
            self.assertIn('Copyright (c) 作者', text)
            # 全文の束にも載る（取得元は repo: の形で書かれる）
            self.review['files'] = [spec, self.review['files'][0]]
            rows, texts, errors = self.inspect()
        self.assertFalse(errors)
        self.assertIn('repo:tools/license-texts/mit.txt', texts[0])
        self.assertIn('試験用の原文', texts[1])

    def test_a_changed_repository_text_or_one_outside_the_folder_is_rejected(self):
        spec = self.repo_text()
        with patch.object(licenses, 'ROOT', self.root):
            (self.root / 'tools/license-texts/mit.txt').write_text('書き換えた文', encoding='utf-8')
            with self.assertRaisesRegex(ValueError, 'SHA-256'):
                licenses.read_source(self.package, spec, True)
            # フォルダの外のファイル・.. を通る道・フォルダの外へのリンクは、SHA-256 が合っていても受けない
            outside = {'repo': 'LICENSE', 'sha256': hashlib.sha256((self.root / 'LICENSE').read_bytes()).hexdigest()}
            roundabout = {'repo': 'tools/license-texts/../../LICENSE', 'sha256': outside['sha256']}
            link = self.root / 'tools/license-texts/link.txt'
            try:
                link.symlink_to(self.root / 'LICENSE')
            except OSError:
                link = None
            for bad in [outside, roundabout, *([{'repo': 'tools/license-texts/link.txt', 'sha256': outside['sha256']}] if link else [])]:
                with self.assertRaisesRegex(ValueError, 'license-texts'):
                    licenses.read_source(self.package, bad, True)
            # 無いファイルは OSError（照合の失敗として main が拾う）
            with self.assertRaises(OSError):
                licenses.read_source(self.package, {'repo': 'tools/license-texts/none.txt', 'sha256': '0' * 64}, True)

    def test_the_eleven_crates_without_an_upstream_license_text_carry_a_copyright_line_and_the_mit_text(self):
        config = json.loads((ROOT / 'tools/licenses-reviewed.json').read_text(encoding='utf-8'))
        authors = {
            'objc2@0.6.4': ['Mads Marquart'], 'block2@0.6.2': ['Mads Marquart'], 'objc2-encode@4.1.0': ['Mads Marquart'],
            'objc2-foundation@0.3.2': ['Mads Marquart'], 'objc2-app-kit@0.3.2': ['Mads Marquart'],
            'objc2-core-foundation@0.3.2': ['Mads Marquart'], 'objc2-core-graphics@0.3.2': ['Mads Marquart'],
            'objc2-metal@0.3.2': ['Mads Marquart'], 'objc2-quartz-core@0.3.2': ['Mads Marquart'],
            'dispatch2@0.3.1': ['Mads Marquart', 'Mary'], 'dispatch@0.2.0': ['Steven Sheldon'],
        }
        for key, names in authors.items():
            with self.subTest(key):
                repo = [spec for spec in config['crates'][key]['files'] if 'repo' in spec]
                self.assertEqual(len(repo), 1)
                path = ROOT / repo[0]['repo']
                self.assertEqual(path.parent, ROOT / 'tools/license-texts')
                self.assertEqual(licenses.digest(path.read_bytes()), repo[0]['sha256'])
                text = path.read_text(encoding='utf-8')
                # 上流の原文ではなく、上流の記載から組み立てた表示であることが、頭に書いてある
                self.assertTrue(text.startswith('[上流の記載から組み立てた表示'), key)
                self.assertIn('上流の原文ではありません', text)
                self.assertIn('not an upstream original', text)
                # 著作権の行は作者ごとに 1 行（年は上流の記載に無いので書かない）、本文は MIT の標準の文
                lines = [line for line in text.splitlines() if line.startswith('Copyright')]
                self.assertEqual(lines, [f'Copyright (c) {name}' for name in names])
                self.assertIn('Permission is hereby granted, free of charge', text)
                self.assertIn('THE SOFTWARE IS PROVIDED "AS IS"', text)
                self.assertNotIn('\r', text)
                self.assertEqual(config['crates'][key]['selected'], ['MIT'])
        # ほかのクレートは、この形（repo）を使わない（上流・クレートに本文があるものは、その原文を固定する）
        users = sorted(k for k, v in config['crates'].items() if any('repo' in spec for spec in v['files']))
        self.assertEqual(users, sorted(authors))

    def test_changed_text_is_rejected(self):
        (self.root / 'LICENSE').write_text('変更された原文')
        self.assertIn('SHA-256', ' '.join(self.inspect()[2]))

    def test_missing_text_is_rejected(self):
        (self.root / 'LICENSE').unlink()
        self.assertTrue(self.inspect()[2])

    def test_unknown_version_is_rejected(self):
        self.config['crates'].clear()
        self.assertIn('未確認', ' '.join(self.inspect()[2]))

    def test_changed_declaration_is_rejected(self):
        self.package['license'] = 'GPL-3.0-only'
        self.assertTrue(self.inspect()[2])

    def test_removing_block_note_does_not_approve_license(self):
        self.review['selected'] = ['MIT', 'GPL-3.0-only']
        self.assertIn('許容外', ' '.join(self.inspect()[2]))

    def test_approved_licenses_still_require_matching_text(self):
        for license_id in ['BSL-1.0', 'Bitstream-Vera']:
            with self.subTest(license=license_id):
                self.package['license'] = license_id
                self.review['declared'] = license_id
                self.review['selected'] = [license_id]
                (self.root / 'LICENSE').write_text('試験用の原文')
                self.assertFalse(self.inspect()[2])
                (self.root / 'LICENSE').write_text('変更された原文')
                self.assertIn('SHA-256', ' '.join(self.inspect()[2]))

    def bundled(self):
        """実行ファイルに埋め込む書体（クレートでない同梱物）の設定と、その原本・許諾の全文。"""
        (self.root / 'face.ttf').write_bytes(b'font bytes')
        (self.root / 'OFL.txt').write_text('同梱の書体の許諾の全文')
        digest = lambda name: hashlib.sha256((self.root / name).read_bytes()).hexdigest()
        item = {'name': 'Face', 'version': '1.0', 'declared': 'OFL-1.1', 'selected': ['OFL-1.1'],
                'repository': 'https://example.invalid/face', 'commit': 'abc123',
                'files': [{'path': 'face.ttf', 'sha256': digest('face.ttf')}],
                'license_file': {'path': 'OFL.txt', 'sha256': digest('OFL.txt')}}
        return {'bundled': {'fixture': [item]}}, item, digest

    def test_bundled_font_is_verified_and_its_license_text_joins_the_bundle(self):
        config, _, _ = self.bundled()
        with patch.object(licenses, 'ROOT', self.root):
            texts, errors = licenses.bundled_assets('fixture', config)
        self.assertFalse(errors)
        self.assertIn('同梱の書体の許諾の全文', texts[0])
        self.assertIn('Face 1.0', texts[0])
        self.assertEqual(licenses.bundled_assets('another', config), ([], []), '別の製品には付かない')

    def test_bundled_font_that_changed_from_the_original_is_rejected(self):
        config, _, _ = self.bundled()
        (self.root / 'face.ttf').write_bytes(b'edited font')
        with patch.object(licenses, 'ROOT', self.root):
            self.assertIn('SHA-256', ' '.join(licenses.bundled_assets('fixture', config)[1]))

    def test_bundled_font_without_the_license_text_or_with_an_unapproved_license_is_rejected(self):
        config, item, _ = self.bundled()
        (self.root / 'OFL.txt').write_text('書き換えた許諾')
        with patch.object(licenses, 'ROOT', self.root):
            texts, errors = licenses.bundled_assets('fixture', config)
        self.assertIn('SHA-256', ' '.join(errors))
        self.assertFalse(texts, '全文が原本と違うなら束に入れない')
        (self.root / 'OFL.txt').unlink()
        with patch.object(licenses, 'ROOT', self.root):
            self.assertTrue(licenses.bundled_assets('fixture', config)[1])
        config, item, _ = self.bundled()
        item['selected'] = ['OFL-1.1', 'GPL-3.0-only']
        with patch.object(licenses, 'ROOT', self.root):
            self.assertIn('許容外', ' '.join(licenses.bundled_assets('fixture', config)[1]))

    def test_bundled_font_outside_the_repository_is_rejected(self):
        config, item, _ = self.bundled()
        item['files'][0]['path'] = '../outside.ttf'
        with patch.object(licenses, 'ROOT', self.root):
            self.assertIn('リポジトリ外', ' '.join(licenses.bundled_assets('fixture', config)[1]))

    def test_inventory_adds_the_bundled_font_errors_and_text(self):
        config, item, _ = self.bundled()
        with patch.object(licenses, 'ROOT', self.root):
            _, texts, errors = self.inspect_with({**self.config, **config})
            self.assertFalse(errors)
            self.assertTrue(any('同梱の書体の許諾の全文' in t for t in texts))
            (self.root / 'face.ttf').write_bytes(b'edited font')
            self.assertIn('SHA-256', ' '.join(self.inspect_with({**self.config, **config})[2]))

    def inspect_with(self, config):
        with patch.object(licenses, 'dependency_keys', return_value={'fixture@1.0.0'}):
            return licenses.inventory('fixture', self.metadata, config, True)

    def test_git_dependency_is_rejected(self):
        self.package['source'] = 'git+https://example.invalid/repo'
        self.assertTrue(self.inspect()[2])

    def test_target_is_used_for_dependency_resolution(self):
        for target in ['x86_64-pc-windows-msvc', 'x86_64-unknown-linux-gnu']:
            with patch.object(licenses, 'TARGET', target), patch.object(licenses, 'cargo', return_value='fixture v1.0.0') as cargo:
                self.assertEqual(licenses.dependency_keys('fixture', 'normal,build', True), {'fixture@1.0.0'})
                self.assertIn(target, cargo.call_args.args)

    COMBINED_TREE = (
        '0app v1.0.0 (/w/app)\n1shared v1.0.0\n2heavy v2.0.0\n1gui v3.0.0\n\n'
        '0cli v1.0.0 (/w/cli)\n1shared v1.0.0\n2heavy v2.0.0\n2libm v0.2.16\n1rmcp v3.5.1\n'
    )

    def test_the_tree_of_a_product_built_together_with_another_counts_the_unified_features(self):
        # 同じ cargo のビルドで作ると機能が合わさり、単独の木に無い依存（libm）が入る。木は根ごとに分け、重なる部分木も展開された物を読む
        self.assertEqual(licenses.subtree_keys(self.COMBINED_TREE, 'cli'),
                         {'cli@1.0.0', 'shared@1.0.0', 'heavy@2.0.0', 'libm@0.2.16', 'rmcp@3.5.1'})
        self.assertEqual(licenses.subtree_keys(self.COMBINED_TREE, 'app'),
                         {'app@1.0.0', 'shared@1.0.0', 'heavy@2.0.0', 'gui@3.0.0'}, 'もう一方の根の依存は混ざらない')
        with self.assertRaises(ValueError):
            licenses.subtree_keys(self.COMBINED_TREE, 'absent')
        with self.assertRaises(ValueError):
            licenses.subtree_keys('0cli v1.0.0\nnot a line\n', 'cli')
        with patch.object(licenses, 'cargo', return_value=self.COMBINED_TREE) as cargo:
            keys = licenses.dependency_keys('cli', 'normal,build', True, 'app')
        self.assertIn('libm@0.2.16', keys)
        arguments = list(cargo.call_args.args)
        self.assertEqual([arguments[i + 1] for i, a in enumerate(arguments) if a == '-p'], ['app', 'cli'], '2 つの製品を 1 回の木で')
        self.assertIn('--no-dedupe', arguments)

    def test_built_with_needs_another_single_product(self):
        for arguments in (['--built-with', 'yolu-app'], ['--package', 'yolu-cli', '--built-with', 'yolu-cli']):
            with patch('sys.argv', ['third-party.py', *arguments]), patch('sys.stderr', io.StringIO()), self.assertRaises(SystemExit):
                licenses.main()

    def test_update_dependencies_are_explicitly_included(self):
        with patch.object(licenses, 'dependency_keys', side_effect=[set(), set(), {'fixture@1.0.0'}, {'fixture@1.0.0'}]):
            records, _, errors = licenses.inventory('yolu-app', self.metadata, self.config, True, True)
        self.assertFalse(errors)
        self.assertEqual(records[0]['crate'], 'fixture')

    def test_target_failure_does_not_remove_other_target_bundle(self):
        target = 'x86_64-pc-windows-msvc'
        failed = self.root / target / 'yolu-app/THIRD_PARTY_LICENSES.txt'
        other = self.root / 'x86_64-unknown-linux-gnu/yolu-app/THIRD_PARTY_LICENSES.txt'
        for bundle in [failed, other]:
            bundle.parent.mkdir(parents=True)
            bundle.write_text('前回の成功')
        with patch.object(licenses, 'OUT', self.root), patch.object(licenses, 'CONFIG', self.root / 'missing.json'), patch.object(licenses, 'TARGET', licenses.TARGET), patch('sys.argv', ['third-party.py', '--package', 'yolu-app', '--target', target, '--bundle']):
            with self.assertRaises(OSError):
                licenses.main()
        self.assertFalse(failed.exists())
        self.assertTrue(other.exists())

    def test_stale_bundle_removed_before_config_failure(self):
        bundle = self.root / 'yolu-cli/THIRD_PARTY_LICENSES.txt'
        bundle.parent.mkdir()
        bundle.write_text('前回の成功')
        with patch.object(licenses, 'OUT', self.root), patch.object(licenses, 'CONFIG', self.root / 'missing.json'), patch('sys.argv', ['third-party.py', '--package', 'yolu-cli', '--bundle']):
            with self.assertRaises(OSError):
                licenses.main()
        self.assertFalse(bundle.exists())

    def test_stale_bundle_removed_before_metadata_failure(self):
        bundle = self.root / 'yolu-cli/THIRD_PARTY_LICENSES.txt'
        bundle.parent.mkdir()
        bundle.write_text('前回の成功')
        with patch.object(licenses, 'OUT', self.root), patch.object(licenses, 'cargo', side_effect=OSError('失敗')), patch('sys.argv', ['third-party.py', '--package', 'yolu-cli', '--bundle']):
            with self.assertRaises(OSError):
                licenses.main()
        self.assertFalse(bundle.exists())

    def run_main_with_ansi_pipes(self):
        """Windows の runner と同じ、日本語を書けない（cp1252 の）標準出力・標準エラーで main を回す。"""
        config = self.root / 'reviewed.json'
        config.write_text(json.dumps({'schema': 1, 'target': licenses.TARGET,
                                      'targets': [licenses.TARGET], **self.config}), encoding='utf-8')
        out = io.TextIOWrapper(io.BytesIO(), encoding='cp1252')
        err = io.TextIOWrapper(io.BytesIO(), encoding='cp1252')
        with patch.object(licenses, 'OUT', self.root / 'out'), patch.object(licenses, 'CONFIG', config), \
                patch.object(licenses, 'TARGET', licenses.TARGET), \
                patch.object(licenses, 'cargo', return_value=json.dumps(self.metadata)), \
                patch.object(licenses, 'dependency_keys', return_value={'fixture@1.0.0'}), \
                patch('sys.stdout', out), patch('sys.stderr', err), \
                patch('sys.argv', ['third-party.py', '--package', 'xtask', '--bundle', '--offline']):
            code = licenses.main()
        out.flush()
        err.flush()
        return code, out.buffer.getvalue().decode('utf-8'), err.buffer.getvalue().decode('utf-8')

    def test_japanese_output_does_not_depend_on_the_console_encoding(self):
        # 前提: この符号化のままでは日本語の print は UnicodeEncodeError になる。
        with self.assertRaises(UnicodeEncodeError):
            print('照合成功', file=io.TextIOWrapper(io.BytesIO(), encoding='cp1252'))
        code, out, _ = self.run_main_with_ansi_pipes()
        self.assertEqual(code, 0)
        self.assertIn('照合成功', out)
        self.assertTrue((self.root / 'out/xtask/THIRD_PARTY_LICENSES.txt').exists())

    def test_japanese_errors_do_not_depend_on_the_console_encoding(self):
        self.config['crates'].clear()
        code, _, err = self.run_main_with_ansi_pipes()
        self.assertEqual(code, 1)
        self.assertIn('要確認', err)


class BundledAuditChecks(unittest.TestCase):

    def setUp(self):
        (ROOT / 'target').mkdir(exist_ok=True)
        self.tmp = tempfile.TemporaryDirectory(dir=ROOT / 'target')
        self.addCleanup(self.tmp.cleanup)
        self.root = Path(self.tmp.name).resolve()
        (self.root / 'a.png').write_bytes(b'asset')
        (self.root / 'NOTICE').write_text('許諾', encoding='utf-8')

        def f(name):
            return {'path': name, 'sha256': licenses.digest((self.root / name).read_bytes())}
        self.item = {'name': 'fixture', 'version': '1', 'selected': ['MIT'], 'origin': 'https://example.invalid', 'files': [f('a.png')], 'file_globs': ['*.png'], 'license_file': f('NOTICE')}
        self.config = {'bundled': {'fixture': [self.item]}}

    def run_assets(self):
        with patch.object(licenses, 'ROOT', self.root):
            return licenses.bundled_assets('fixture', self.config)

    def test_new_image_rejected(self):
        (self.root / 'b.png').write_bytes(b'new')
        self.assertIn('未登録', str(self.run_assets()[1]))

    def test_changed_notice_rejected(self):
        (self.root / 'NOTICE').write_text('changed', encoding='utf-8')
        (texts, errors) = self.run_assets()
        self.assertFalse(texts)
        self.assertTrue(errors)

    def test_escaped_notice_not_read(self):
        self.item['license_file']['path'] = '../outside'
        (texts, errors) = self.run_assets()
        self.assertFalse(texts)
        self.assertIn('リポジトリ外', str(errors))

    def test_notice_not_utf8_rejected(self):
        (self.root / 'NOTICE').write_bytes(b'\xff')
        self.item['license_file']['sha256'] = licenses.digest(b'\xff')
        self.assertIn('UTF-8', str(self.run_assets()[1]))

    def test_no_selected_license_rejected(self):
        self.item['selected'] = []
        self.assertTrue(self.run_assets()[1])

    def test_lock_categorizes_test_and_other_targets(self):
        packages = []
        reviews = {}
        for name in ['runtime', 'test', 'other']:
            p = self.root / name
            p.mkdir()
            (p / 'LICENSE').write_text('許諾', encoding='utf-8')
            packages.append({'name': name, 'version': '1.0', 'source': 'registry+fixture', 'id': name, 'license': 'MIT', 'repository': None, 'manifest_path': str(p / 'Cargo.toml')})
            if name != 'other':
                reviews[name + '@1.0'] = {'declared': 'MIT', 'selected': ['MIT'], 'blocked': [], 'files': [{'path': 'LICENSE', 'sha256': licenses.digest('許諾'.encode())}]}
        (self.root / 'Cargo.lock').write_text('\n'.join((f'[[package]]\nname = "{n}"\nversion = "1.0"\nsource = "registry+fixture"\n' for n in ['runtime', 'test', 'other'])), encoding='utf-8')

        def cargo(*args):
            return '\nruntime v1.0\n\ntest v1.0\n' if 'normal,build,dev' in args else 'runtime v1.0\n'
        with patch.object(licenses, 'ROOT', self.root), patch.object(licenses, 'OUT', self.root / 'out'), patch.object(licenses, 'cargo', side_effect=cargo):
            self.assertEqual(licenses.audit_lock({'packages': packages, 'workspace_members': []}, {'targets': ['fixture'], 'crates': reviews}, True), 0)
        r = json.loads((self.root / 'out/lock-inventory.json').read_text(encoding='utf-8'))
        self.assertEqual({x['crate']: x['scope'] for x in r['records']}, {'runtime': '通常・ビルド', 'test': '試験のみ', 'other': '対象外'})
        reviews['stale@1.0'] = reviews['test@1.0']
        with patch.object(licenses, 'ROOT', self.root), patch.object(licenses, 'OUT', self.root / 'out'), patch.object(licenses, 'cargo', side_effect=cargo):
            self.assertEqual(licenses.audit_lock({'packages': packages, 'workspace_members': []}, {'targets': ['fixture'], 'crates': reviews}, True), 1)

    def test_windows_paths_match_registered_posix_paths(self):
        # Windows の relative_to が返す区切りを、Linux の CI でも再現する。
        (self.root / 'assets').mkdir()
        (self.root / 'a.png').rename(self.root / 'assets/a.png')
        self.item['files'][0]['path'] = 'assets/a.png'
        self.item['file_globs'] = ['assets/*.png']
        entry = Mock()
        entry.is_file.return_value = True
        entry.relative_to.return_value = PureWindowsPath('assets/a.png')
        self.assertEqual(str(entry.relative_to(self.root)), 'assets\\a.png')
        with patch.object(type(self.root), 'glob', return_value=[entry]):
            texts, errors = self.run_assets()
        self.assertFalse(errors)
        self.assertTrue(texts)

    def test_notice_hashes_survive_autocrlf_checkout(self):
        # 本番の属性・原文・登録ハッシュを使い、Git 自身の CRLF 変換を通す。
        config = json.loads((ROOT / 'tools/licenses-reviewed.json').read_text(encoding='utf-8'))
        notices = [item['license_file'] for item in config['bundled']['yolu-app']]
        (self.root / '.gitattributes').write_bytes((ROOT / '.gitattributes').read_bytes())
        for spec in notices:
            path = self.root / spec['path']
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_bytes((ROOT / spec['path']).read_bytes())
        # 対照ファイルが CRLF になることで、試験が変換を実際に通したと確認する。
        (self.root / 'control.txt').write_bytes(b'first\nsecond\n')
        def git(*args):
            return subprocess.run(['git', '-c', 'core.autocrlf=true', '-c', 'core.safecrlf=false',
                                   *args], cwd=self.root, check=True, capture_output=True)
        git('init', '--quiet')
        git('add', '.gitattributes', 'control.txt', *[spec['path'] for spec in notices])
        for name in ['control.txt', *[spec['path'] for spec in notices]]:
            (self.root / name).unlink()
        git('checkout-index', '--all', '--force')
        self.assertEqual((self.root / 'control.txt').read_bytes(), b'first\r\nsecond\r\n')
        for spec in notices:
            with self.subTest(path=spec['path']):
                self.assertEqual(licenses.digest((self.root / spec['path']).read_bytes()), spec['sha256'])


if __name__ == '__main__':
    unittest.main()
