#!/usr/bin/env python3
"""全文の欠落・変更・未承認が配布用の成功にならないことを確かめる。"""
import hashlib
import importlib.util
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

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

    def test_git_dependency_is_rejected(self):
        self.package['source'] = 'git+https://example.invalid/repo'
        self.assertTrue(self.inspect()[2])

    def test_stale_bundle_removed_before_config_failure(self):
        bundle = self.root / 'yolu-bridge/THIRD_PARTY_LICENSES.txt'
        bundle.parent.mkdir()
        bundle.write_text('前回の成功')
        with patch.object(licenses, 'OUT', self.root), patch.object(licenses, 'CONFIG', self.root / 'missing.json'), patch('sys.argv', ['third-party.py', '--package', 'yolu-bridge', '--bundle']):
            with self.assertRaises(OSError):
                licenses.main()
        self.assertFalse(bundle.exists())

    def test_stale_bundle_removed_before_metadata_failure(self):
        bundle = self.root / 'yolu-bridge/THIRD_PARTY_LICENSES.txt'
        bundle.parent.mkdir()
        bundle.write_text('前回の成功')
        with patch.object(licenses, 'OUT', self.root), patch.object(licenses, 'cargo', side_effect=OSError('失敗')), patch('sys.argv', ['third-party.py', '--package', 'yolu-bridge', '--bundle']):
            with self.assertRaises(OSError):
                licenses.main()
        self.assertFalse(bundle.exists())


if __name__ == '__main__':
    unittest.main()
