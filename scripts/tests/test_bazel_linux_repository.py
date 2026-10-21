#!/usr/bin/env python3
"""Check that the Linux alias repository cannot mutate or mirror source files."""
from pathlib import Path
from types import SimpleNamespace
import unittest

ROOT = Path(__file__).resolve().parents[2]


class AliasRepositoryTests(unittest.TestCase):
    def test_implementation_materializes_only_build_text(self):
        definitions = {}
        namespace = {
            'repository_rule': lambda **kwargs: definitions.update(kwargs),
            'attr': SimpleNamespace(label=lambda **kwargs: kwargs),
        }
        exec(compile((ROOT / 'bazel/linux_backend_repository.bzl').read_text(),
                     'linux_backend_repository.bzl', 'exec'), namespace)
        writes = []
        content = (ROOT / 'bazel/gpui-linux.BUILD.bazel').read_text()
        # This context intentionally has no path, symlink, execute, or delete.
        ctx = SimpleNamespace(
            attr=SimpleNamespace(build_file='declared-template'),
            read=lambda label: content if label == 'declared-template' else self.fail(label),
            file=lambda path, data, **kwargs: writes.append((path, data, kwargs)),
        )
        definitions['implementation'](ctx)
        self.assertEqual(writes, [('BUILD.bazel', content, {'executable': False})])
        self.assertIn('actual = "@@//:gpui_linux"', content)
        self.assertEqual(set(definitions['attrs']), {'build_file'})

    def test_module_uses_alias_only_repository_with_existing_override(self):
        text = (ROOT / 'MODULE.bazel').read_text()
        self.assertIn('use_repo_rule("//bazel:linux_backend_repository.bzl", "linux_backend_repository")', text)
        self.assertIn('linux_backend_repository(name = "gpui_linux_local", build_file = "//bazel:gpui-linux.BUILD.bazel")', text)
        self.assertIn('override_repo(crate, **{"crates__gpui-pre-linux-0.3.9": "gpui_linux_local"})', text)
        self.assertNotIn('new_local_repository(', text)

    def test_backend_sources_remain_owned_by_root_package(self):
        self.assertFalse((ROOT / 'vendor/gpui-pre-linux/BUILD.bazel').exists())
        self.assertIn('vendor_linux()', (ROOT / 'BUILD.bazel').read_text())
        self.assertIn('native.glob(["vendor/gpui-pre-linux/src/**/*.rs"])',
                      (ROOT / 'bazel/vendor_linux.bzl').read_text())


if __name__ == '__main__':
    unittest.main()
