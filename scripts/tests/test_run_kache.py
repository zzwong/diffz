#!/usr/bin/env python3
"""Verify runtime isolation and command boundaries without Cargo compilation."""

import json
import math
import os
from pathlib import Path
import shutil
import signal
import subprocess
import sys
import tempfile
import time
import tomllib
import unittest


REPO = Path(__file__).resolve().parents[2]
SCRIPT = REPO / "scripts/run-kache.py"
CAPTURE = '''
import json, os, sys, tomllib
from pathlib import Path
config = Path(os.environ["KACHE_CONFIG"])
print(json.dumps({"argv":sys.argv[1:], "cwd":os.getcwd(),
 "config":str(config), "text":config.read_text(),
 "settings":tomllib.loads(config.read_text()),
 "env":{key:os.environ.get(key) for key in
 ("KACHE_HOST_CONFIG","KACHE_RUNTIME_DIR","KACHE_SOCKET_PATH","RUSTC_WRAPPER")}}, default=str))
'''


class RuntimeTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory(prefix="diffz-kr-test-")
        self.addCleanup(self.temporary.cleanup)
        # Child cwd resolves aliases such as macOS /var -> /private/var.
        self.base = Path(self.temporary.name).resolve()
        self.checkout = self.base / "checkout with spaces"
        self.checkout.mkdir()
        self.config = self.checkout / "policy.toml"
        self.config.write_text((REPO / "scripts/kache.toml").read_text())
        self.wrapper = self.base / "tools/kache"
        self.env = {"PATH": os.environ["PATH"], "HOME": str(self.base / "home"),
                    "KACHE_CONFIG": "/unrelated/config", "KACHE_HOST_CONFIG": "/unrelated/host",
                    "KACHE_RUNTIME_DIR": str(self.base / "ignored-runtime"),
                    "KACHE_SOCKET_PATH": "/unrelated/socket", "RUSTC_WRAPPER": "/unrelated/wrapper"}

    def command(self, *command, config=None, socket="target/kache/runtime/daemon.sock", wrapper=None):
        return [sys.executable, str(SCRIPT), "--bin", str(wrapper or self.wrapper),
                "--config", str(config or self.config), "--socket", str(socket), "--",
                *(command or (sys.executable, "-c", CAPTURE))]

    def invoke(self, *command, cwd=None, **kwargs):
        return subprocess.run(self.command(*command, **kwargs), cwd=cwd or self.checkout,
                              env=self.env, capture_output=True, text=True, timeout=20)

    def captured(self, **kwargs):
        result = self.invoke(**kwargs)
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        return json.loads(result.stdout)

    def test_checkout_absolute_runtime_and_operational_paths(self):
        result = self.captured()
        self.assertEqual(result["settings"]["cache"]["runtime_dir"],
                         str(self.checkout / "target/kache/runtime"))
        self.assertTrue(Path(result["config"]).is_absolute())
        self.assertEqual(Path(result["config"]).stat().st_mode & 0o777, 0o600)
        self.assertEqual(result["env"]["KACHE_SOCKET_PATH"],
                         str(self.checkout / "target/kache/runtime/daemon.sock"))
        self.assertEqual(result["env"]["RUSTC_WRAPPER"], str(self.wrapper))
        self.assertEqual(result["env"]["KACHE_HOST_CONFIG"], "")
        self.assertTrue(result["settings"]["cache"]["ignore_env"])
        self.assertTrue(result["settings"]["cache"]["local_only"])
        self.assertFalse((self.base / "ignored-runtime").exists())

    def test_custom_config_comments_other_tables_and_source_are_preserved(self):
        original = ('# Keep this policy comment.\n[cache]\nignore_env = true\n'
                    'local_only = true\nlocal_max_size = "64MiB"\n'
                    '"runtime_dir" = "custom runtime"\n'
                    '[another]\nruntime_dir = "untouched"\nvalues = [1, 2]\n')
        self.config.write_text(original)
        result = self.captured(socket="custom socket", wrapper="relative tools/kache")
        expected = tomllib.loads(original)
        expected["cache"]["runtime_dir"] = str(self.checkout / "custom runtime")
        self.assertEqual(result["settings"], expected)
        self.assertIn("# Keep this policy comment.", result["text"])
        self.assertEqual(self.config.read_text(), original)
        self.assertEqual(result["env"]["RUSTC_WRAPPER"], str(self.checkout / "relative tools/kache"))
        self.assertEqual(result["env"]["KACHE_SOCKET_PATH"], str(self.checkout / "custom socket"))

    def test_two_worktrees_and_home_expansion_have_separate_runtime(self):
        other = self.base / "another checkout"
        other.mkdir()
        one = self.captured()
        two = self.captured(cwd=other)
        self.assertNotEqual(one["config"], two["config"])
        self.assertNotEqual(one["settings"]["cache"]["runtime_dir"],
                            two["settings"]["cache"]["runtime_dir"])
        self.config.write_text('[cache]\nruntime_dir = "~/explicit runtime"\n')
        expanded = self.captured()
        self.assertEqual(expanded["settings"]["cache"]["runtime_dir"],
                         str(self.base / "home/explicit runtime"))

    def test_absolute_runtime_and_environment_override_keep_their_selected_location(self):
        explicit = str(self.base / "shared/link/../runtime")
        self.config.write_text('[cache]\nignore_env = false\nruntime_dir = ' + json.dumps(explicit) + "\n")
        self.env["KACHE_RUNTIME_DIR"] = "environment runtime"
        result = self.captured()
        self.assertEqual(result["settings"]["cache"]["runtime_dir"], explicit)
        self.assertFalse(result["settings"]["cache"]["ignore_env"])
        self.assertEqual(result["env"]["KACHE_RUNTIME_DIR"], str(self.checkout / "environment runtime"))
        self.env["KACHE_RUNTIME_DIR"] = str(self.base / "explicit environment runtime")
        result = self.captured()
        self.assertEqual(result["env"]["KACHE_RUNTIME_DIR"], self.env["KACHE_RUNTIME_DIR"])
        self.config.write_text(self.config.read_text().replace("ignore_env = false", "ignore_env = true"))
        self.env["KACHE_RUNTIME_DIR"] = "ignored relative runtime"
        result = self.captured()
        self.assertEqual(result["env"]["KACHE_RUNTIME_DIR"], "ignored relative runtime")

    def test_runtime_toml_string_and_table_forms_preserve_the_rest_of_the_policy(self):
        policies = (
            '[cache]\n"runtime_dir" = "custom runtime" # retain inline comment\n',
            "['cache']\n'runtime_dir' = 'custom # runtime'\n",
            'cache.runtime_dir = "custom runtime"\n',
            '"cache"."runtime_dir" = "custom runtime"\n',
            "cache = { runtime_dir = 'custom runtime', ignore_env = true }\n",
            '[cache]\nruntime_dir = """\ncustom\nruntime"""\n',
            "[cache]\nruntime_dir = '''\ncustom\nruntime'''\n",
            '[cache]\nruntime_dir = """a""b\\"c"""\n',
            '[cache]\nruntime_dir = """quoted ending"""""\n',
            "[cache]\nruntime_dir = '''quoted ending'''''\n",
        )
        for policy in policies:
            with self.subTest(policy=policy):
                source = ('# Quotes in comments "must not" become value tokens.\n' + policy
                    + '[metadata]\nruntime_dir = "custom runtime"\n'
                    + 'timestamp = 1979-05-27T07:32:00Z\nnot_a_number = nan\n'
                    + 'nested = [{ flag = true, values = [1, 2.5, "other"] }]\n')
                self.config.write_text(source)
                result = self.captured()
                expected = tomllib.loads(source)
                expected["cache"]["runtime_dir"] = str(self.checkout / expected["cache"]["runtime_dir"])
                actual = tomllib.loads(result["text"])
                self.assertTrue(math.isnan(expected["metadata"].pop("not_a_number")))
                self.assertTrue(math.isnan(actual["metadata"].pop("not_a_number")))
                self.assertEqual(actual, expected)
                self.assertIn('# Quotes in comments "must not" become value tokens.', result["text"])
                if "# retain inline comment" in source:
                    self.assertIn("# retain inline comment", result["text"])
                self.assertEqual(self.config.read_text(), source)

    def test_missing_runtime_uses_checkout_default_without_changing_policy(self):
        for source in ("# empty policy\n", "[cache]\nignore_env = true\n",
                       "[cache]", "cache.ignore_env = true\n",
                       '"cache" = { "ignore_env" = true }\n', "cache = {}\n",
                       "[cache.nested]\nother = 1\n"):
            with self.subTest(policy=source):
                self.config.write_text(source)
                result = self.captured()
                expected = tomllib.loads(source)
                expected.setdefault("cache", {})["runtime_dir"] = str(self.checkout / "target/kache/runtime")
                self.assertEqual(tomllib.loads(result["text"]), expected)
                self.assertEqual(self.config.read_text(), source)

    def test_different_concurrent_configs_cannot_overwrite_selected_snapshot(self):
        second = self.checkout / "second policy.toml"
        second.write_text(self.config.read_text().replace('"8GiB"', '"64MiB"'))
        gate = self.base / "continue"
        code = '''
import json, os, sys, time, tomllib
from pathlib import Path
config = Path(os.environ["KACHE_CONFIG"])
Path(sys.argv[1]).write_text(str(config))
deadline = time.monotonic() + 10
while not Path(sys.argv[2]).exists():
    if time.monotonic() > deadline: raise SystemExit(99)
    time.sleep(0.01)
print(tomllib.loads(config.read_text())["cache"]["local_max_size"])
'''
        processes = []
        selected = [self.base / "one", self.base / "two"]
        try:
            for config, marker in zip((self.config, second), selected):
                processes.append(subprocess.Popen(self.command(sys.executable, "-c", code,
                    str(marker), str(gate), config=config), cwd=self.checkout, env=self.env,
                    stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True))
            deadline = time.monotonic() + 10
            while not all(path.exists() for path in selected):
                if time.monotonic() > deadline:
                    self.fail("both commands did not select a complete config")
                time.sleep(0.01)
            self.assertNotEqual(selected[0].read_text(), selected[1].read_text())
            gate.touch()
            for process, expected in zip(processes, ("8GiB", "64MiB")):
                stdout, stderr = process.communicate(timeout=10)
                self.assertEqual(process.returncode, 0, stderr)
                self.assertEqual(stdout.strip(), expected)
        finally:
            gate.touch()
            for process in processes:
                if process.poll() is None:
                    process.kill()
                process.communicate(timeout=10)

    def test_bad_toml_or_runtime_never_starts_the_command(self):
        marker = self.base / "must-not-run"
        for bad in ('[cache\n', '[cache]\nruntime_dir = 42\n',
                    '[cache]\nruntime_dir = ""\n', '[cache]\nruntime_dir = "bad\\u0000path"\n',
                    'cache = "not a table"\n'):
            with self.subTest(config=bad):
                self.config.write_text(bad)
                result = self.invoke(sys.executable, "-c",
                    "from pathlib import Path; import sys; Path(sys.argv[1]).touch()", str(marker))
                self.assertNotEqual(result.returncode, 0)
                self.assertFalse(marker.exists())
                self.assertEqual(self.config.read_text(), bad)
        self.assertFalse((self.checkout / "target").exists())

    def test_existing_corrupt_or_symlink_snapshot_is_preserved(self):
        snapshot = Path(self.captured()["config"])
        snapshot.chmod(0o644)
        self.assertNotEqual(self.invoke().returncode, 0)
        self.assertEqual(snapshot.stat().st_mode & 0o777, 0o644)
        snapshot.chmod(0o600)
        snapshot.write_text("unexpected owned contents")
        self.assertNotEqual(self.invoke().returncode, 0)
        self.assertEqual(snapshot.read_text(), "unexpected owned contents")
        snapshot.unlink()
        unrelated = self.base / "unrelated.toml"
        unrelated.write_text("preserve me")
        snapshot.symlink_to(unrelated)
        self.assertNotEqual(self.invoke().returncode, 0)
        self.assertTrue(snapshot.is_symlink())
        self.assertEqual(unrelated.read_text(), "preserve me")

    def test_generated_config_directory_symlink_is_not_followed(self):
        unrelated = self.base / "unrelated directory"
        unrelated.mkdir()
        directory = self.checkout / "target/kache"
        directory.mkdir(parents=True)
        (directory / "config").symlink_to(unrelated, target_is_directory=True)
        result = self.invoke()
        self.assertNotEqual(result.returncode, 0)
        self.assertEqual(list(unrelated.iterdir()), [])

    def test_generated_config_directory_writable_by_others_is_preserved(self):
        directory = self.checkout / "target/kache/config"
        directory.mkdir(parents=True)
        directory.chmod(0o777)
        result = self.invoke()
        self.assertNotEqual(result.returncode, 0)
        self.assertEqual(directory.stat().st_mode & 0o777, 0o777)
        self.assertEqual(list(directory.iterdir()), [])

    def test_command_arguments_exit_and_signals_are_preserved(self):
        result = self.invoke(sys.executable, "-c", CAPTURE, "argument with spaces", "--flag=value")
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(json.loads(result.stdout)["argv"], ["argument with spaces", "--flag=value"])
        self.assertEqual(self.invoke(sys.executable, "-c", "raise SystemExit(42)").returncode, 42)
        result = self.invoke(sys.executable, "-c", "import os, signal; os.kill(os.getpid(), signal.SIGTERM)")
        self.assertEqual(result.returncode, -signal.SIGTERM)

    @unittest.skipUnless(os.environ.get("DIFFZ_TEST_KACHE_BIN") and os.environ.get("DIFFZ_TEST_RUSTC"),
                         "set explicit pinned Kache and rustc paths for the non-compiling real probe")
    def test_real_pinned_wrapper_keeps_nested_crate_events_in_one_runtime(self):
        binary = Path(os.environ["DIFFZ_TEST_KACHE_BIN"]).absolute()
        rustc = Path(os.environ["DIFFZ_TEST_RUSTC"]).absolute()
        for key, name in (("HOME", "home"), ("XDG_CACHE_HOME", "cache"),
                          ("XDG_CONFIG_HOME", "config"), ("XDG_STATE_HOME", "state"),
                          ("XDG_DATA_HOME", "data"), ("TMPDIR", "tmp"),
                          ("CARGO_HOME", "cargo"), ("RUSTUP_HOME", "rustup")):
            directory = self.base / name
            directory.mkdir(exist_ok=True)
            self.env[key] = str(directory)
        self.env.update(PATH=str(rustc.parent) + os.pathsep + "/usr/bin:/bin", RUSTC=str(rustc),
                        RUSTC_WORKSPACE_WRAPPER="", LC_ALL="C")
        self.config.write_text(self.config.read_text() + '\nauto_gc = false\nclean_incremental = false\n'
            + 'local_store = ' + json.dumps(str(self.base / "store")) + '\n')
        self.env.update(KACHE_CONFIG=str(self.config), KACHE_HOST_CONFIG="")
        version = subprocess.run([str(binary), "--version"], env=self.env, cwd=self.checkout,
                                 capture_output=True, text=True, timeout=10)
        self.assertEqual(version.returncode, 0, version.stderr)
        self.assertEqual(version.stdout.strip(), "kache 0.21.0")
        crate = self.checkout / "crates/core"
        crate.mkdir(parents=True)
        code = '''
import os, subprocess, sys
for cwd in sys.argv[2:]:
    result = subprocess.run([os.environ["RUSTC_WRAPPER"],sys.argv[1],"-vV"], cwd=cwd,
                            capture_output=True,text=True,timeout=10)
    assert result.returncode == 0, result.stderr
    assert result.stdout.startswith("rustc "), result.stdout
'''
        socket = self.checkout / "target/kache/runtime/daemon.sock"
        try:
            result = self.invoke(sys.executable, "-c", code, str(rustc), str(self.checkout), str(crate),
                                 wrapper=binary)
            self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
            events = self.checkout / "target/kache/runtime/events.jsonl"
            records = [json.loads(line) for line in events.read_text().splitlines()]
            self.assertEqual(len(records), 2)
            self.assertTrue(all(row["result"] == "passthrough" for row in records))
            self.assertFalse((crate / "target/kache/runtime").exists())
            self.assertFalse((self.base / "ignored-runtime").exists())
            self.assertFalse(socket.exists(), "version probes should not start a daemon")
        finally:
            if socket.exists():
                stopped = self.invoke(str(binary), "daemon", "stop", wrapper=binary)
                self.assertEqual(stopped.returncode, 0, stopped.stderr)


class MakeRuntimeTests(unittest.TestCase):
    # Keep Make plumbing separate from direct launcher fixtures.
    setUp = RuntimeTests.setUp

    def test_make_quoted_config_version_override_and_plain_bypass(self):
        scripts = self.checkout / "scripts"
        scripts.mkdir()
        for name in ("run-kache.py", "bootstrap-kache.py", "kache.toml"):
            shutil.copy2(REPO / "scripts" / name, scripts / name)
        shutil.copy2(REPO / "Makefile", self.checkout / "Makefile")
        commands = self.base / "commands"
        commands.mkdir()
        cargo = commands / "cargo"
        cargo.write_text("#!" + sys.executable + "\n" + CAPTURE)
        cargo.chmod(0o755)
        rustc = commands / "rustc"
        rustc.write_text("#!" + sys.executable + "\nprint('rustc 1.99.0\\nhost: x86_64-unknown-linux-gnu')\n")
        rustc.chmod(0o755)
        self.wrapper.parent.mkdir()
        self.wrapper.write_text("#!" + sys.executable + "\nprint('kache 1.2.2')\n")
        self.wrapper.chmod(0o755)
        self.env["PATH"] = str(commands) + os.pathsep + self.env["PATH"]
        result = subprocess.run(["make", "--no-print-directory", "build", "KACHE_VERSION=1.2.2",
            "KACHE_BIN=" + str(self.wrapper), "KACHE_CONFIG=" + str(self.config),
            "KACHE_SOCKET=custom socket"], cwd=self.checkout, env=self.env,
            capture_output=True, text=True, timeout=20)
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        captured = json.loads(result.stdout.splitlines()[-1])
        self.assertEqual(captured["argv"], ["build", "--locked", "-p", "diffz"])
        self.assertEqual(captured["settings"]["cache"]["runtime_dir"], str(self.checkout / "target/kache/runtime"))
        self.assertEqual(captured["env"]["KACHE_SOCKET_PATH"], str(self.checkout / "custom socket"))
        self.config.write_text("invalid TOML")
        cargo.write_text("#!" + sys.executable + "\nimport os\nassert os.environ['RUSTC_WRAPPER']==''\nassert os.environ['RUSTC_WORKSPACE_WRAPPER']==''\n")
        result = subprocess.run(["make", "--no-print-directory", "build-plain", "KACHE_CONFIG=" + str(self.config)],
            cwd=self.checkout, env=self.env, capture_output=True, text=True, timeout=20)
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)


if __name__ == "__main__":
    unittest.main()
