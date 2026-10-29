import copy
import importlib.util
from pathlib import Path
import unittest


SPEC = importlib.util.spec_from_file_location("qualification", Path(__file__).parents[1] / "qualify-gpui-activation.py")
qualification = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(qualification)


class LockedGraphTests(unittest.TestCase):
    def setUp(self):
        self.seed = {"version": 4, "package": [
            {"name": "app", "version": "1.0"},
            {"name": "dependency", "version": "2.0", "source": "registry+https://index.crates.io", "checksum": "abc"},
        ]}

    def test_app_pruning_preserves_dependency_identity(self):
        qualification.verify_lock(self.seed, {"version": 4, "package": [self.seed["package"][1]]})

    def test_changed_version_checksum_or_source_is_rejected(self):
        for field, value in (("version", "2.1"), ("checksum", "changed"), ("source", "git+https://example.com")):
            with self.subTest(field=field):
                candidate = copy.deepcopy(self.seed)
                candidate["package"][1][field] = value
                with self.assertRaises(RuntimeError):
                    qualification.verify_lock(self.seed, candidate)

    def test_new_dependency_is_rejected(self):
        candidate = copy.deepcopy(self.seed)
        candidate["package"].append({"name": "new", "version": "1.0"})
        with self.assertRaises(RuntimeError):
            qualification.verify_lock(self.seed, candidate)

    def test_new_dependency_edge_is_rejected(self):
        candidate = copy.deepcopy(self.seed)
        candidate["package"][0]["dependencies"] = ["dependency"]
        with self.assertRaises(RuntimeError):
            qualification.verify_lock(self.seed, candidate)

    def test_pruned_edge_and_version_shorthand_are_allowed(self):
        self.seed["package"][0]["dependencies"] = ["dependency 2.0"]
        candidate = copy.deepcopy(self.seed)
        candidate["package"][0]["dependencies"] = ["dependency"]
        qualification.verify_lock(self.seed, candidate)
        candidate["package"][0]["dependencies"] = []
        qualification.verify_lock(self.seed, candidate)

    def test_changed_unused_patch_revision_is_rejected(self):
        self.seed["patch"] = {"unused": [{"name": "fork", "version": "1.0", "source": "git+https://example.com?rev=one#one"}]}
        candidate = copy.deepcopy(self.seed)
        candidate["patch"]["unused"][0]["source"] = "git+https://example.com?rev=two#two"
        with self.assertRaises(RuntimeError):
            qualification.verify_lock(self.seed, candidate)

    def test_transitive_opposite_backend_is_rejected(self):
        metadata = {
            "workspace_members": ["linux"],
            "packages": [{"id": "linux", "name": "gpui-pre-linux"}, {"id": "core", "name": "gpui-pre"}],
            "resolve": {"nodes": [
                {"id": "linux", "features": ["wayland"], "deps": [{"pkg": "core"}]},
                {"id": "core", "features": ["wayland", "x11"], "deps": []},
            ]},
        }
        with self.assertRaises(RuntimeError):
            qualification.verify_features(metadata, "wayland")
        metadata["resolve"]["nodes"][1]["features"] = ["wayland"]
        qualification.verify_features(metadata, "wayland")


if __name__ == "__main__":
    unittest.main()
