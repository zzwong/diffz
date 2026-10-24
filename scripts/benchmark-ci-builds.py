#!/usr/bin/env python3
"""One-shot hosted CI comparison; never changes required CI or production caches."""
import argparse
from collections import Counter
import hashlib
import json
import os
from pathlib import Path
import re
import shutil
import signal
import subprocess
import sys
import time

ROOT = Path(__file__).resolve().parent.parent
TOOLS = ("cargo", "cargo-kache", "bazel")
PROBE = ("crates/core/src/patch.rs", b"max_files: 10_000,", b"max_files: 10_007,")


def sha(path):
    digest = hashlib.sha256()
    with path.open("rb") as stream:
        for chunk in iter(lambda: stream.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def rust_tests(text):
    # Both Cargo and the Bazel wrappers expose the underlying libtest statuses.
    tests = Counter()
    for name, status in re.findall(r"^test (\S+) \.\.\. (ok|FAILED|ignored)\b", text, re.M):
        tests[f"{name}: {status}"] += 1
    if not tests or any(key.endswith(": FAILED") for key in tests):
        raise RuntimeError("Missing or failing Rust test statuses; not a passing comparison")
    return dict(sorted(tests.items()))


def safe_members(names, prefixes):
    for name in names:
        path = Path(name)
        if path.is_absolute() or ".." in path.parts:
            raise RuntimeError(f"Unsafe cache archive entry: {name}")
        if not any(path == Path(prefix) or Path(prefix) in path.parents for prefix in prefixes):
            raise RuntimeError(f"Unexpected cache archive entry: {name}")


def native_inputs(native):
    # ImageVersion is a descriptive label; packages/toolchains are measured below.
    return {key: value for key, value in native.items() if key != "image"}


def source_comparison(seed_revision, revision):
    # Only the experimental harness may differ when replaying an existing seed.
    return ["git", "diff", "--exit-code", seed_revision, revision, "--", ".",
            ":!.github/workflows/bazel-ci-comparison.yml",
            ":!scripts/benchmark-ci-builds.py", ":!scripts/tests/test_benchmark_ci_builds.py"]


def read_command(args):
    return subprocess.check_output(args, cwd=ROOT, text=True, timeout=60).strip()


class Trial:
    def __init__(self, tool, phase, jobs):
        self.tool, self.phase, self.jobs = tool, phase, jobs
        self.out = ROOT / "target/ci-results" / phase
        self.out.mkdir(parents=True, exist_ok=False)
        self.cache = ROOT / "target/ci-cache"
        self.cache.mkdir(parents=True, exist_ok=True)
        self.host = Path.home() / ".local/share/codex/worktrees/diffz/ci-benchmark-cache"
        self.env = os.environ | {
            "CARGO_HOME": str(self.cache / "cargo-home"), "CARGO_INCREMENTAL": "0",
            "CARGO_BUILD_JOBS": str(jobs), "CC": "/usr/bin/gcc", "CXX": "/usr/bin/g++",
            "RUSTC_WRAPPER": "", "RUSTC_WORKSPACE_WRAPPER": "",
            "DIFFZ_BAZEL_CACHE": "1", "DIFFZ_BAZEL_BATCH": "1",
            "DIFFZ_BAZEL_TEST_TMP_ROOT": "/var/tmp/diffz-ci-benchmark/tests",
            "DIFFZ_TEST_TMP_ROOT": "/var/tmp/diffz-ci-benchmark/tests",
        }
        Path(self.env["DIFFZ_TEST_TMP_ROOT"]).mkdir(parents=True, exist_ok=True)
        self.env.pop("DIFFZ_BAZEL_REMOTE_CACHE", None)
        self.env.pop("DIFFZ_BAZEL_REMOTE_CACHE_READ_ONLY", None)
        self.result = dict(schema_version=1, tool=tool, phase=phase, jobs=jobs,
                           revision=read_command(["git", "rev-parse", "HEAD"]),
                           complete=False, commands=[],
                           profiles="Cargo dev / Bazel fast-dev; operational configs, not identical rustc flags",
                           cache_transport="zstd snapshots through GitHub Actions artifacts; not a remote CAS service")
        self.daemon = None
        self.kache = self.cache / "kache-tool/bin/kache"
        self.policy = self.cache / "kache.toml"
        self.socket = ROOT / "target/kache/runtime/daemon.sock"

    def save(self):
        (self.out / "result.json").write_text(json.dumps(self.result, indent=2) + "\n")

    def run(self, name, command, timeout=2400):
        log = self.out / f"{len(self.result['commands']):02d}-{name}.log"
        record = dict(name=name, command=list(map(str, command)), log=log.name)
        self.result["commands"].append(record)
        self.save()
        print(f"START {name}: {command}", flush=True)
        started = time.monotonic()
        with log.open("w") as stream:
            process = subprocess.Popen(record["command"], cwd=ROOT, env=self.env,
                                       stdout=stream, stderr=subprocess.STDOUT, start_new_session=True)
            try:
                code = process.wait(timeout=timeout)
            except (subprocess.TimeoutExpired, KeyboardInterrupt):
                os.killpg(process.pid, signal.SIGTERM)
                try:
                    process.wait(timeout=8)
                except subprocess.TimeoutExpired:
                    os.killpg(process.pid, signal.SIGKILL)
                    process.wait()
                record.update(seconds=time.monotonic() - started, exit_code=process.returncode, timed_out=True)
                self.save()
                raise RuntimeError(f"{name} interrupted or exceeded {timeout}s")
        record.update(seconds=time.monotonic() - started, exit_code=code)
        self.save()
        print(f"END {name}: {record['seconds']:.2f}s, exit {code}", flush=True)
        if code:
            print(log.read_text(errors="replace")[-18000:], flush=True)
            raise RuntimeError(f"{name} failed with exit {code}")
        return log

    def identity(self):
        native = {name: read_command(command) for name, command in {
            "packages": ["dpkg-query", "-W", "-f=${binary:Package}=${Version}\\n"],
            "rustc": ["rustc", "-vV"], "cargo": ["cargo", "-V"],
            "cc": ["/usr/bin/gcc", "--version"], "cxx": ["/usr/bin/g++", "--version"],
        }.items()}
        native.update(image=os.environ.get("ImageVersion"), image_os=os.environ.get("ImageOS"),
                      os_release=Path("/etc/os-release").read_text())
        self.result.update(native=native, native_sha256=hashlib.sha256(json.dumps(native, sort_keys=True).encode()).hexdigest(),
                           cpu=read_command(["lscpu"]), memory=Path("/proc/meminfo").read_text(),
                           source_lock_sha256=sha(ROOT / "Cargo.lock"),
                           module_lock_before=sha(ROOT / "MODULE.bazel.lock"))
        self.save()

    def workspace_paths(self):
        if self.tool == "bazel":
            return ["target/bazel-disk-cache", "target/bazel-bootstrap", "target/ci-cache/repository-downloads",
                    "target/ci-cache/cargo-home/registry", "target/ci-cache/cargo-home/git"]
        paths = ["target/debug", "target/ci-cache/cargo-home/registry", "target/ci-cache/cargo-home/git"]
        if self.tool == "cargo-kache":
            paths += ["target/ci-cache/kache-tool", "target/ci-cache/kache-store"]
        return paths

    def restore(self):
        incoming = ROOT / "target/ci-incoming"
        seed = json.loads((incoming / "seed.json").read_text())
        for key in ("tool", "source_lock_sha256"):
            if seed[key] != self.result[key]:
                raise RuntimeError(f"Seed/consumer {key} mismatch; refusing incomparable cache reuse")
        if native_inputs(seed["native"]) != native_inputs(self.result["native"]):
            raise RuntimeError("Seed/consumer native input mismatch; refusing incomparable cache reuse")
        if seed["revision"] != self.result["revision"]:
            self.run("validate-source-revision", source_comparison(seed["revision"], self.result["revision"]), timeout=60)
        self.result["seed_image"] = seed["native"].get("image")
        if not seed["complete"]:
            raise RuntimeError("Seed was incomplete")
        self.result["seed"] = {k: seed[k] for k in ("revision", "native_sha256", "archives")}
        for archive in seed["archives"]:
            path = incoming / archive["name"]
            if sha(path) != archive["sha256"]:
                raise RuntimeError(f"Cache checksum mismatch: {path}")
            repository = archive["name"] == "repositories.tar.zst"
            directory = self.host / "target" if repository else ROOT
            prefixes = ["repositories"] if repository else self.workspace_paths()
            listing = self.run("validate-archive", ["tar", "--zstd", "-tf", path], timeout=180)
            safe_members(listing.read_text().splitlines(), prefixes)
            directory.mkdir(parents=True, exist_ok=True)
            self.run("restore-" + archive["name"], ["tar", "--zstd", "-xf", path,
                                                     "--no-same-owner", "-C", directory], timeout=600)
        self.save()

    def setup(self):
        if shutil.disk_usage(ROOT).free < 18 * 1024**3:
            raise RuntimeError("Need 18 GiB free before the full desktop trial")
        if self.tool == "bazel":
            if self.host.exists():
                raise RuntimeError("Refusing to reuse an existing cache-host checkout")
            self.host.parent.mkdir(parents=True, exist_ok=True)
            self.run("cache-host", ["git", "worktree", "add", "--detach", self.host, "HEAD"], timeout=60)
            self.env["DIFFZ_BAZEL_REPO_CONTENTS_CACHE"] = str(self.host / "target/repositories")
        if self.phase == "consumer":
            self.restore()
        if self.tool == "cargo-kache":
            self.policy.write_text('[cache]\nignore_env = true\nlocal_only = true\nlocal_max_size = "6GiB"\n'
                                   f'local_store = {json.dumps(str(self.cache / "kache-store"))}\n'
                                   'runtime_dir = "target/kache/runtime"\n')
            self.run("kache-bootstrap", ["make", "kache", f"KACHE_ROOT={self.cache / 'kache-tool'}"])
            with (self.out / "kache-daemon.log").open("w") as stream:
                self.daemon = subprocess.Popen(self.wrapper() + [str(self.kache), "daemon", "run"],
                                               cwd=ROOT, env=self.env, stdout=stream,
                                               stderr=subprocess.STDOUT, start_new_session=True)
            deadline = time.monotonic() + 20
            while not self.socket.exists():
                if self.daemon.poll() is not None or time.monotonic() > deadline:
                    raise RuntimeError("Owned Kache daemon did not become ready")
                time.sleep(0.1)
            self.run("kache-ready", self.wrapper() + [self.kache, "daemon", "status"], timeout=30)
        elif self.tool == "bazel":
            self.run("bazel-bootstrap", ["scripts/bazel-trial.sh", "--version"], timeout=180)
            self.run("bazel-fetch", self.bazel("fetch") + ["//:all_build", "--lockfile_mode=update"])
            self.result["module_lock_after"] = sha(ROOT / "MODULE.bazel.lock")
            (self.out / "MODULE.bazel.lock").write_bytes((ROOT / "MODULE.bazel.lock").read_bytes())
        self.save()

    def wrapper(self):
        return ["python3", "scripts/run-kache.py", "--bin", str(self.kache),
                "--config", str(self.policy), "--socket", str(self.socket), "--"]

    def bazel(self, command):
        if command == "fetch":
            return ["scripts/bazel-trial.sh", command, f"--repository_cache={self.cache / 'repository-downloads'}", "--color=no", "--curses=no"]
        return ["scripts/bazel-trial.sh", command, "--config=fast-dev", "--config=cache-eager",
                f"--jobs={self.jobs}", f"--local_resources=cpu={self.jobs}",
                f"--repository_cache={self.cache / 'repository-downloads'}", "--color=no", "--curses=no"]

    def build_and_test(self, scenario):
        if self.tool == "bazel":
            build = self.bazel("build") + ["//:all_build", "--lockfile_mode=error",
                                           f"--profile={self.out / (scenario + '.profile.json')}",
                                           f"--build_event_json_file={self.out / (scenario + '.bep.jsonl')}"]
            test = self.bazel("test") + ["//:tests", "--lockfile_mode=error", "--nocache_test_results", "--test_output=all"]
        else:
            prefix = self.wrapper() if self.tool == "cargo-kache" else []
            build = prefix + ["cargo", "build", "--locked", "--workspace", "--all-features", "--all-targets"]
            test = prefix + ["cargo", "test", "--locked", "--workspace", "--all-features"]
        self.run(scenario + "-build", build)
        log = self.run(scenario + "-tests", test)
        tests = rust_tests(log.read_text(errors="replace"))
        if "tests" in self.result and tests != self.result["tests"]:
            raise RuntimeError(f"Test membership/status changed in {scenario}")
        self.result["tests"] = tests
        binary = ROOT / ("target/bazel-bin/diffz" if self.tool == "bazel" else "target/debug/diffz")
        self.run(scenario + "-version", [binary, "--version"], timeout=30)
        inspect = self.run(scenario + "-inspect", [binary, "--state-dir", self.out / (scenario + "-state"),
                                                  "--inspect", "--fixture", "F01"], timeout=60)
        parsed = json.loads(inspect.read_text())
        if "inspection" in self.result and parsed != self.result["inspection"]:
            raise RuntimeError(f"Fixture inspection changed in {scenario}")
        self.result["inspection"] = parsed
        self.save()

    def stop(self):
        if self.daemon is None:
            return
        daemon, self.daemon = self.daemon, None
        try:
            if self.socket.exists() and daemon.poll() is None:
                self.run("kache-stats", self.wrapper() + [self.kache, "--json", "stats"], timeout=30)
                self.run("kache-stop", self.wrapper() + [self.kache, "daemon", "stop"], timeout=30)
        finally:
            try:
                daemon.wait(timeout=10)
            except subprocess.TimeoutExpired:
                os.killpg(daemon.pid, signal.SIGTERM)
                try:
                    daemon.wait(timeout=5)
                except subprocess.TimeoutExpired:
                    os.killpg(daemon.pid, signal.SIGKILL)
                    daemon.wait()
                raise RuntimeError("Kache required forced termination; cache snapshot not accepted")

    def pack(self):
        directory = ROOT / "target/ci-artifacts"
        directory.mkdir(parents=True, exist_ok=False)
        specs = [("workspace.tar.zst", ROOT, [p for p in self.workspace_paths() if (ROOT / p).exists()])]
        if self.tool == "bazel":
            specs.append(("repositories.tar.zst", self.host / "target", ["repositories"]))
        archives = []
        for name, base, paths in specs:
            path = directory / name
            self.run("pack-" + name, ["tar", "-I", "zstd -T2 -3", "-cf", path, "-C", base] + paths, timeout=900)
            archives.append(dict(name=name, bytes=path.stat().st_size, sha256=sha(path)))
        if sum(a["bytes"] for a in archives) > 12 * 1024**3:
            raise RuntimeError("Cache bundles exceed the 12 GiB trial budget")
        self.result["archives"] = archives
        self.result["complete"] = True
        self.save()
        (directory / "seed.json").write_text(json.dumps(self.result, indent=2) + "\n")

    def execute(self):
        try:
            self.identity()
            self.setup()
            self.build_and_test("cold" if self.phase == "seed" else "fresh-restore")
            if self.phase == "seed":
                self.build_and_test("warm-noop")
                self.stop()
                self.pack()
            else:
                path, old, new = PROBE
                source = ROOT / path
                original = source.read_bytes()
                if original.count(old) != 1:
                    raise RuntimeError("Core semantic probe no longer matches")
                source.write_bytes(original.replace(old, new))
                try:
                    self.build_and_test("core-edit")
                finally:
                    expected = original.replace(old, new)
                    if source.read_bytes() != expected:
                        raise RuntimeError("Concurrent source modification; refusing to overwrite it")
                    source.write_bytes(original)
                seed = json.loads((ROOT / "target/ci-incoming/seed.json").read_text())
                if self.result["tests"] != seed["tests"] or self.result["inspection"] != seed["inspection"]:
                    raise RuntimeError("Seed/consumer test or fixture parity mismatch")
                self.result["complete"] = True
        except Exception as error:
            self.result.update(complete=False, error=str(error))
            print(f"FAILED: {error}", file=sys.stderr, flush=True)
        finally:
            try:
                self.stop()
                if self.host.exists():
                    # Retain the cache host until the runner expires; no live service.
                    self.result["repository_cache_checkout"] = str(self.host)
                if sha(ROOT / "Cargo.lock") != self.result.get("source_lock_sha256"):
                    raise RuntimeError("Cargo.lock changed during the benchmark")
            except Exception as error:
                self.result.update(complete=False, cleanup_error=str(error))
            self.save()
        return 0 if self.result["complete"] else 1


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--tool", choices=TOOLS, required=True)
    parser.add_argument("--phase", choices=("seed", "consumer"), required=True)
    parser.add_argument("--jobs", type=int, default=4)
    args = parser.parse_args()
    if args.jobs < 1:
        parser.error("--jobs must be positive")
    def interrupt(signum, frame):
        raise KeyboardInterrupt
    signal.signal(signal.SIGTERM, interrupt)
    return Trial(args.tool, args.phase, args.jobs).execute()


if __name__ == "__main__":
    sys.exit(main())
