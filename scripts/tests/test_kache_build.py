#!/usr/bin/env python3
"""Exercise the optional Kache wrapper against hostile Cargo/Kache settings."""

import json
import os
from pathlib import Path
import shlex
import shutil
import socket
import subprocess
import tempfile
import threading
import tomllib
import unittest


REPO = Path(__file__).resolve().parents[2]
KACHE_VERSION = "0.21.0"


class KacheConfigTests(unittest.TestCase):
    def test_checked_in_configs_pin_forced_local_policy(self):
        kache = tomllib.loads((REPO / "scripts/kache.toml").read_text())["cache"]
        self.assertTrue(kache["ignore_env"])
        self.assertTrue(kache["local_only"])
        self.assertEqual(kache["local_max_size"], "8GiB")
        self.assertEqual(kache["runtime_dir"], "target/kache/runtime")
        self.assertNotIn("remote", kache)
        self.assertNotIn("fallback", kache)

        cargo_config = tomllib.loads((REPO / "scripts/kache-cargo.toml").read_text())
        build = cargo_config["build"]
        self.assertEqual(build["rustc-wrapper"], "target/kache/0.21.0/bin/kache")
        self.assertEqual(build["rustc-workspace-wrapper"], "")
        env = cargo_config["env"]
        for key in (
            "KACHE_CONFIG",
            "KACHE_HOST_CONFIG",
            "KACHE_RUNTIME_DIR",
            "KACHE_SOCKET_PATH",
            "KACHE_LOCAL_ONLY",
            "KACHE_MAX_SIZE",
            "KACHE_FALLBACK",
            "RUSTC_WRAPPER",
            "RUSTC_WORKSPACE_WRAPPER",
        ):
            self.assertTrue(env[key]["force"], key)
        self.assertTrue(env["KACHE_CONFIG"]["relative"])
        self.assertTrue(env["KACHE_RUNTIME_DIR"]["relative"])
        self.assertTrue(env["KACHE_SOCKET_PATH"]["relative"])
        self.assertEqual(env["KACHE_HOST_CONFIG"]["value"], "")
        self.assertEqual(env["KACHE_FALLBACK"]["value"], "")
        self.assertEqual(env["RUSTC_WRAPPER"]["value"], "")
        self.assertEqual(env["RUSTC_WORKSPACE_WRAPPER"]["value"], "")

        bootstrap_config = tomllib.loads(
            (REPO / "scripts/kache-bootstrap-cargo.toml").read_text()
        )
        bootstrap_build = bootstrap_config["build"]
        self.assertEqual(bootstrap_build["rustc-wrapper"], "")
        self.assertEqual(bootstrap_build["rustc-workspace-wrapper"], "")
        bootstrap_env = bootstrap_config["env"]
        for key in ("RUSTC_WRAPPER", "RUSTC_WORKSPACE_WRAPPER"):
            self.assertEqual(bootstrap_env[key]["value"], "")
            self.assertTrue(bootstrap_env[key]["force"], key)


class KacheBuildTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.cargo = shutil.which("cargo")
        cls.kache_source = REPO / "target/kache" / KACHE_VERSION / "bin/kache"
        if cls.cargo is None or not cls.kache_source.is_file():
            raise unittest.SkipTest(
                "cargo and the bootstrapped pinned Kache are required for integration checks"
            )

    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(prefix="dk-")
        self.addCleanup(self.temp.cleanup)
        self.checkout = Path(self.temp.name) / "checkout with spaces"
        scripts = self.checkout / "scripts"
        scripts.mkdir(parents=True)
        for name in (
            "kache-build.sh",
            "kache.toml",
            "kache-cargo.toml",
            "kache-bootstrap-cargo.toml",
        ):
            shutil.copy2(REPO / "scripts" / name, scripts / name)

        self.kache_bin = self.checkout / "target/kache" / KACHE_VERSION / "bin/kache"
        self.kache_bin.parent.mkdir(parents=True)
        shutil.copy2(self.kache_source, self.kache_bin)
        self.local_store = Path(self.temp.name) / "local-store"
        kache_config = scripts / "kache.toml"
        kache_config.write_text(
            kache_config.read_text()
            + f"local_store = {json.dumps(str(self.local_store))}\n"
        )

        (self.checkout / "Cargo.toml").write_text(
            '[package]\nname = "kache-fixture"\nversion = "0.1.0"\nedition = "2021"\n'
        )
        (self.checkout / "Cargo.lock").write_text(
            'version = 4\n\n[[package]]\nname = "kache-fixture"\nversion = "0.1.0"\n'
        )
        source = self.checkout / "src"
        source.mkdir()
        (source / "lib.rs").write_text("pub fn fixture() -> u32 { 42 }\n")

        self.ambient_socket = Path(self.temp.name) / "ambient-daemon.sock"
        self.ambient_listener = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
        self.ambient_listener.bind(str(self.ambient_socket))
        self.ambient_listener.listen()
        self.ambient_listener.settimeout(0.1)
        self.listener_stop = threading.Event()
        self.ambient_connections = Path(self.temp.name) / "ambient-daemon-ran"
        self.listener_thread = threading.Thread(
            target=self.accept_ambient_connections, daemon=True
        )
        self.listener_thread.start()
        self.addCleanup(self.stop_ambient_listener)

        self.ambient_runtime = Path(self.temp.name) / "ambient-runtime"
        self.ambient_runtime.mkdir()
        self.ambient_remote = Path(self.temp.name) / "ambient-remote"
        self.ambient_remote.mkdir()
        self.fallback_marker = Path(self.temp.name) / "fallback-ran"
        self.workspace_marker = Path(self.temp.name) / "workspace-wrapper-ran"
        self.rustc_marker = Path(self.temp.name) / "ambient-rustc-wrapper-ran"
        self.fallback_wrapper = Path(self.temp.name) / "fallback-wrapper"
        self.workspace_wrapper = Path(self.temp.name) / "workspace-wrapper"
        self.rustc_wrapper = Path(self.temp.name) / "ambient-rustc-wrapper"
        for path, marker in (
            (self.fallback_wrapper, self.fallback_marker),
            (self.workspace_wrapper, self.workspace_marker),
            (self.rustc_wrapper, self.rustc_marker),
        ):
            self.write_executable(
                path,
                f"#!/usr/bin/env bash\ntouch -- {shlex.quote(str(marker))}\nexec \"$@\"\n",
            )

        self.ambient_kache_config = Path(self.temp.name) / "ambient-kache.toml"
        self.ambient_kache_config.write_text(
            "[cache]\n"
            "ignore_env = false\n"
            "local_only = false\n"
            "local_hit_daemon = true\n"
            "local_max_size = \"1MiB\"\n"
            f"fallback = {json.dumps(str(self.fallback_wrapper))}\n"
            f"runtime_dir = {json.dumps(str(self.ambient_runtime))}\n"
            "[cache.remote]\n"
            "type = \"filesystem\"\n"
            f"path = {json.dumps(str(self.ambient_remote))}\n"
        )
        self.ambient_host_config = Path(self.temp.name) / "ambient-host.toml"
        self.ambient_host_config.write_text(
            "[cache]\n"
            f"fallback = {json.dumps(str(self.fallback_wrapper))}\n"
            "[cache.remote]\n"
            "type = \"filesystem\"\n"
            f"path = {json.dumps(str(self.ambient_remote))}\n"
        )

        cargo_home = Path(self.temp.name) / "cargo-home"
        cargo_home.mkdir()
        (cargo_home / "config.toml").write_text(
            "[build]\n"
            f"rustc-wrapper = {json.dumps(str(self.rustc_wrapper))}\n"
            f"rustc-workspace-wrapper = {json.dumps(str(self.workspace_wrapper))}\n\n"
            "[env]\n"
            f"KACHE_CONFIG = {{ value = {json.dumps(str(self.ambient_kache_config))}, force = true }}\n"
            f"KACHE_HOST_CONFIG = {{ value = {json.dumps(str(self.ambient_host_config))}, force = true }}\n"
            f"KACHE_RUNTIME_DIR = {{ value = {json.dumps(str(self.ambient_runtime))}, force = true }}\n"
            f"KACHE_SOCKET_PATH = {{ value = {json.dumps(str(self.ambient_socket))}, force = true }}\n"
            "KACHE_LOCAL_ONLY = { value = \"0\", force = true }\n"
            "KACHE_MAX_SIZE = { value = \"1MiB\", force = true }\n"
            f"KACHE_FALLBACK = {{ value = {json.dumps(str(self.fallback_wrapper))}, force = true }}\n"
            f"RUSTC_WRAPPER = {{ value = {json.dumps(str(self.rustc_wrapper))}, force = true }}\n"
            f"RUSTC_WORKSPACE_WRAPPER = {{ value = {json.dumps(str(self.workspace_wrapper))}, force = true }}\n"
        )
        self.cargo_home = cargo_home

    def accept_ambient_connections(self):
        while not self.listener_stop.is_set():
            try:
                connection, _ = self.ambient_listener.accept()
            except socket.timeout:
                continue
            except OSError:
                return
            self.ambient_connections.touch()
            connection.close()

    def stop_ambient_listener(self):
        self.listener_stop.set()
        self.ambient_listener.close()
        self.listener_thread.join(timeout=2)

    @staticmethod
    def write_executable(path, content):
        path.write_text(content)
        path.chmod(0o755)

    def hostile_env(self):
        env = {
            key: value
            for key, value in os.environ.items()
            if not key.startswith(("CARGO", "KACHE", "RUSTC")) and key != "RUSTC"
        }
        env.update(
            PATH=os.environ.get("PATH", ""),
            CARGO_HOME=str(self.cargo_home),
            KACHE_CONFIG=str(self.ambient_kache_config),
            KACHE_HOST_CONFIG=str(self.ambient_host_config),
            KACHE_LOCAL_ONLY="0",
            KACHE_MAX_SIZE="1MiB",
            KACHE_RUNTIME_DIR=str(self.ambient_runtime),
            KACHE_SOCKET_PATH=str(self.ambient_socket),
            KACHE_FALLBACK=str(self.fallback_wrapper),
            RUSTC_WRAPPER=str(self.rustc_wrapper),
            RUSTC_WORKSPACE_WRAPPER=str(self.workspace_wrapper),
            CARGO_BUILD_RUSTC_WRAPPER=str(self.rustc_wrapper),
            CARGO_BUILD_RUSTC_WORKSPACE_WRAPPER=str(self.workspace_wrapper),
        )
        return env

    def run_wrapper(self, *args):
        return subprocess.run(
            ["bash", str(self.checkout / "scripts/kache-build.sh"), *args],
            cwd=self.checkout,
            env=self.hostile_env(),
            capture_output=True,
            text=True,
            timeout=60,
        )

    def test_real_kache_isolated_from_forced_ambient_cargo_and_kache_config(self):
        result = self.run_wrapper("--locked", "-vv")
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)

        self.assertIn(str(self.kache_bin.resolve()), result.stderr)
        self.assertFalse(self.fallback_marker.exists())
        self.assertFalse(self.rustc_marker.exists())
        self.assertFalse(self.workspace_marker.exists())
        self.assertFalse(self.ambient_connections.exists())
        self.assertEqual(list(self.ambient_remote.iterdir()), [])
        self.assertEqual(list(self.ambient_runtime.iterdir()), [])
        self.assertTrue(self.ambient_socket.exists())

    def test_wrong_pinned_version_fails_clearly(self):
        self.write_executable(
            self.kache_bin,
            "#!/usr/bin/env bash\nprintf 'kache 0.20.0\\n'\n",
        )
        result = self.run_wrapper("--dry-run")
        self.assertEqual(result.returncode, 1)
        self.assertIn("Expected kache 0.21.0", result.stderr)
        self.assertIn("got: kache 0.20.0.", result.stderr)

    def test_unavailable_pinned_version_fails_clearly(self):
        self.kache_bin.unlink()
        fake_bin = Path(self.temp.name) / "fake-bin"
        fake_bin.mkdir()
        fake_cargo = fake_bin / "cargo"
        self.write_executable(
            fake_cargo,
            "#!/usr/bin/env bash\n"
            "if [[ \"${1:-}\" == install ]]; then\n"
            "  echo 'simulated pinned Kache registry failure' >&2\n"
            "  exit 42\n"
            "fi\n"
            f"exec {shlex.quote(self.cargo)} \"$@\"\n",
        )
        env = self.hostile_env()
        env["PATH"] = f"{fake_bin}:{env['PATH']}"
        result = subprocess.run(
            ["bash", str(self.checkout / "scripts/kache-build.sh"), "--dry-run"],
            cwd=self.checkout,
            env=env,
            capture_output=True,
            text=True,
            timeout=60,
        )
        self.assertEqual(result.returncode, 1)
        self.assertIn("Unable to install pinned kache 0.21.0", result.stderr)


if __name__ == "__main__":
    unittest.main()
