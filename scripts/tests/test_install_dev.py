"""Linux installer shell fixtures: only fake payloads and temporary HOME/XDG paths."""
import json
from pathlib import Path
import shlex
import shutil
import subprocess
import sys
import tempfile
import unittest

REPO = Path(__file__).resolve().parents[2]


class InstallDevTest(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(prefix="diffz-install-dev-")
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.repo = self.root / "source"
        for name in ("scripts/install-dev.sh", "packaging/linux/io.github.zzwong.Diffz.desktop",
                     "packaging/linux/icons/io.github.zzwong.Diffz.svg"):
            dest = self.repo / name
            dest.parent.mkdir(parents=True, exist_ok=True)
            shutil.copyfile(REPO / name, dest)
        # An allowlisted PATH prevents accidental build, desktop-cache or host app commands.
        commands = self.root / "commands"
        commands.mkdir()
        for name in ("bash", "dirname", "mkdir", "awk", "cat", "install", "rm"):
            (commands / name).symlink_to(shutil.which(name))
        (commands / "uname").write_text("#!/bin/sh\nprintf 'Linux\\n'\n")
        (commands / "uname").chmod(0o755)
        self.env = {"PATH": str(commands), "HOME": str(self.root / "home"),
                    "XDG_STATE_HOME": str(self.root / "xdg state"),
                    "XDG_DATA_HOME": str(self.root / "xdg data"),
                    "XDG_CONFIG_HOME": str(self.root / "xdg config"),
                    "XDG_CACHE_HOME": str(self.root / "xdg cache"),
                    "XDG_RUNTIME_DIR": str(self.root / "xdg runtime"),
                    "APPLICATIONS_DIR": str(self.root / "applications"),
                    "PREFIX": str(self.root / "home/.local"),
                    "DATADIR": str(self.root / "xdg data"),
                    "DEV_STATE_DIR": str(self.root / "xdg state/diffz-dev"),
                    "RECORD": str(self.root / "argv.json")}
        self.fake = self.root / "fake payload"
        self.fake.write_text(f"#!{sys.executable}\nimport json, os, sys\n"
                             "with open(os.environ['RECORD'], 'w') as f:\n"
                             "    json.dump(sys.argv[1:], f)\n"
                             "sys.exit(int(os.environ.get('PAYLOAD_EXIT', '0')))\n")
        self.fake.chmod(0o755)
        self.env["DEV_BIN"] = str(self.fake)

    @property
    def wrapper(self):
        return Path(self.env["PREFIX"]) / "bin/diffz-dev"

    @property
    def payload(self):
        return Path(self.env["PREFIX"]) / "libexec/diffz-dev"

    def installer(self, action, check=True):
        # Refuse to execute the installer if any fixture destination escaped its temp root.
        for key in ("HOME", "PREFIX", "DATADIR", "DEV_STATE_DIR", "APPLICATIONS_DIR", "DEV_BIN"):
            Path(self.env[key]).resolve().relative_to(self.root)
        return subprocess.run(["bash", str(self.repo / "scripts/install-dev.sh"), action],
                              env=self.env, cwd=self.root, capture_output=True, text=True, check=check)

    def invoke(self, args):
        result = subprocess.run([str(self.wrapper), *args], env=self.env, cwd=self.root,
                                capture_output=True, text=True)
        return result, json.loads(Path(self.env["RECORD"]).read_text())

    def test_argv_and_doctor_contract(self):
        self.installer("install")
        cases = [[], ["change with spaces.patch"], ["--fixture", "F01"],
                 ["--state-dir", str(self.root / "explicit state"), "--foreground"],
                 ["--state-dir", "first", "--state-dir", "last"],
                 ["--state-dir"], ["--font", "--state-dir"],
                 ["--doctor"], ["--doctor", "extra"], ["--help"], ["--version"],
                 ["", "a'b", 'a"b', "$literal", "*", "line\nbreak"]]
        for args in cases:
            with self.subTest(args=args):
                result, recorded = self.invoke(args)
                self.assertEqual(result.returncode, 0, result.stderr)
                expected = args if args == ["--doctor"] else ["--state-dir", self.env["DEV_STATE_DIR"], *args]
                self.assertEqual(recorded, expected)

    def test_configured_paths_and_desktop_use_same_wrapper(self):
        self.env.update(PREFIX=str(self.root / "prefix with ' apostrophe"),
                        DATADIR=str(self.root / "data with ' apostrophe"),
                        DEV_STATE_DIR=str(self.root / "custom state with ' apostrophe"))
        self.installer("install")
        self.env["DEV_STATE_DIR"] = str(self.root / "changed at runtime")
        self.env["XDG_STATE_HOME"] = str(self.root / "changed xdg")
        result, args = self.invoke([])
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(args, ["--state-dir", str(self.root / "custom state with ' apostrophe")])
        desktop = Path(self.env["DATADIR"]) / "applications/io.github.zzwong.Diffz.Dev.desktop"
        fields = dict(line.split("=", 1) for line in desktop.read_text().splitlines() if "=" in line)
        self.assertEqual(shlex.split(fields["Exec"]), [str(self.wrapper), "%f"])
        self.assertEqual(fields["TryExec"], str(self.wrapper))
        self.assertNotIn("MimeType", fields)
        self.assertEqual(self.payload.read_bytes(), self.fake.read_bytes())

    def test_exit_status_is_forwarded(self):
        self.installer("install")
        self.env["PAYLOAD_EXIT"] = "37"
        for args in ([], ["--doctor"]):
            self.assertEqual(self.invoke(args)[0].returncode, 37)

    def test_reinstall_and_uninstall_preserve_release_and_state(self):
        prefix, data = Path(self.env["PREFIX"]), Path(self.env["DATADIR"])
        sentinels = [prefix / "bin/diffz", prefix / "libexec/diffz",
                     data / "applications/io.github.zzwong.Diffz.desktop",
                     data / "icons/hicolor/scalable/apps/io.github.zzwong.Diffz.svg",
                     Path(self.env["DEV_STATE_DIR"]) / "keep",
                     Path(self.env["XDG_STATE_HOME"]) / "diffz/keep"]
        for path in sentinels:
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_text("untouched")
        # Upgrade an old direct-binary install, then reinstall the wrapper.
        self.wrapper.write_bytes(self.fake.read_bytes())
        self.wrapper.chmod(0o755)
        for _ in range(2):
            self.installer("install")
            self.assertEqual(self.invoke([])[1], ["--state-dir", self.env["DEV_STATE_DIR"]])
        installed = [self.wrapper, self.payload,
                     data / "applications/io.github.zzwong.Diffz.Dev.desktop",
                     data / "icons/hicolor/scalable/apps/io.github.zzwong.Diffz.Dev.svg"]
        self.assertTrue(all(path.is_file() for path in installed))
        for _ in range(2):
            self.installer("uninstall")
            self.assertTrue(all(not path.exists() for path in installed))
            self.assertTrue(all(path.read_text() == "untouched" for path in sentinels))

    def test_rejected_path_characters_cannot_reach_generated_shell(self):
        for value in ('"', '`', '$', '\\', '%', '\n'):
            with self.subTest(value=value):
                self.env["DEV_STATE_DIR"] = str(self.root / ("bad" + value + "state"))
                self.assertEqual(self.installer("install", check=False).returncode, 2)
                self.assertFalse(self.wrapper.exists())
                self.assertFalse(self.payload.exists())


if __name__ == "__main__":
    unittest.main()
