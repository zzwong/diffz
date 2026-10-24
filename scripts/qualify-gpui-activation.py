#!/usr/bin/env python3
"""Qualify exact GPUI activation unit sources on the existing hosted Linux job.

SOURCE-ONLY REVIEW CANDIDATE: this file has not been executed or syntax-checked.
Requires explicit root approval before publication/execution. No source edits,
lock updates, test exclusions, substitute fonts, installs, or GUI operations.
"""

from __future__ import annotations

import argparse
from collections import Counter
import hashlib
import json
import os
from pathlib import Path, PurePosixPath
import re
import resource
import shlex
import shutil
import signal
import stat
import subprocess
import sys
import threading
import time
import tomllib
import traceback
import urllib.request


BASELINE = "a3ab4f1bd071dd23372251c4de34098267ed050a"
GPUI = "d79c3d1191aa68d4753e1ca410c2d10c261b4572"
CORE_TREE = "337da148ab56dc5495613b86794464f28917e72d"
VENDOR_TREE = "af6afb57920f0d41bcbdd7c45ce4ea876515197a"
WGPU = "389e7f33bed4f6237ccb53f2b4884d996274a453"
ZED = "93d745668805029476e6d7fbb19652116fed51de"
TARGET = "x86_64-unknown-linux-gnu"
TOOLCHAIN = "1.99.0"
CARGO = ["rustup", "run", TOOLCHAIN, "cargo"]
APP_BLOBS = {
    "Cargo.toml": "a34fe2c8dda7c970e95b7093337903c3e7c2ac6b",
    "Cargo.lock": "84c187a3ba886c4802a82ea994e85e57ec123a06",
    "rust-toolchain.toml": "b432996f101609389fcd631582395680520eb494",
    ".cargo/config.toml": "fe832ac518a6b23649d018850933cac76c0b893d",
}
CORE_BLOBS = {
    "Cargo.toml": "6a925e621bbc04b22ceec6697d3b82c07fd56ee1",
    "Cargo.lock": "41dd2fefaf31f034dc2ccd48ce2050d3d45315c0",
}
FONTS = (
    ("assets/fonts/ibm-plex-sans/IBMPlexSans-Regular.ttf",
     "bd6817d5202895da5ac4fad88de3da71e652881a", 200500),
    ("assets/fonts/lilex/Lilex-Regular.ttf",
     "cb98a69b0ffb1976805bf5af3047307fe2422b1d", 194308),
)
PATCHES = {
    "gpui-pre-wgpu": {"git": "https://github.com/zzwong/gpui-pre-wgpu", "rev": WGPU},
    "gpui-pre": {"git": "https://github.com/zzwong/gpui-pre", "rev": GPUI},
    "gpui-base": {"git": "https://github.com/zzwong/gpui-base",
                  "rev": "6235089ccddd423369749017dabb4f6df8a3c169"},
    "gpui-pre-linux": {"path": "vendor/gpui-pre-linux"},
    "gpui-pre-macos": {"git": "https://github.com/zzwong/gpui-pre-macos",
                       "rev": "28057de61df9442cb521497a63193e70dc36019f"},
    "tree-sitter-gleam": {"git": "https://github.com/gleam-lang/tree-sitter-gleam",
                          "rev": "c610c282ef73f830d80c1f0999dce8e83f024ef5"},
}
CORE_TESTS = (
    "window::tests::external_activation_dispatches_exactly_once_without_claiming_focus",
    "platform::tests::graphical_environment_and_nested_windowing_debug_redact_activation_token",
)
VENDOR_TESTS = (
    "linux::wayland::client::activation_lifetime_tests::absent_environment_is_not_mutated_but_every_present_value_is_removed",
    "linux::wayland::client::activation_lifetime_tests::startup_token_is_consumed_only_by_the_first_toplevel",
    "linux::wayland::window::external_activation_tests::external_submission_is_exact_and_once_without_a_diffz_size_limit",
    "linux::wayland::window::external_activation_tests::invalid_or_unsupported_context_never_calls_the_sink",
)
CORE_FEATURES = ["--features", "test-support"]  # Keep the documented default features.
UNIT_FEATURES = ["--no-default-features", "--features",
                 "gpui-pre-linux/wayland,gpui-pre-linux/x11,gpui-pre-linux/test-support"]


class QualificationError(Exception):
    pass


class NoRedirect(urllib.request.HTTPRedirectHandler):
    def redirect_request(self, request, response, code, message, headers, new_url):
        raise QualificationError("Font HTTP redirect refused before contacting its destination")


def require(condition: bool, message: str) -> None:
    if not condition:
        raise QualificationError(message)


def blob_id(data: bytes) -> str:
    return hashlib.sha1(b"blob " + str(len(data)).encode("ascii") + b"\0" + data).hexdigest()


def regular_bytes(path: Path) -> bytes:
    require(not path.is_symlink(), f"Symlink not accepted: {path}")
    require(path.is_file(), f"Required regular file missing: {path}")
    return path.read_bytes()


def verify_blob(path: Path, expected: str) -> bytes:
    data = regular_bytes(path)
    require(blob_id(data) == expected, f"Git blob mismatch: {path}")
    return data


def fingerprint(root: Path) -> dict[str, str]:
    """Read only the fresh, harness-owned source stage, never a broad filesystem."""
    result = {}
    for directory, dirs, files in os.walk(root, followlinks=False):
        for name in dirs:
            require(not (Path(directory) / name).is_symlink(), "Staged directory symlink")
        for name in files:
            path = Path(directory) / name
            data = regular_bytes(path)
            mode = stat.S_IMODE(path.stat().st_mode)
            result[str(path.relative_to(root))] = f"{mode:o}:{hashlib.sha256(data).hexdigest()}"
    return result


def read_metadata(path: Path) -> dict:
    value = json.loads(path.read_text())
    require(isinstance(value, dict) and value.get("resolve"), "Full resolved metadata required")
    return value


def vendor_manifest() -> str:
    lines = ['[workspace]', 'resolver = "2"', 'members = ["vendor/gpui-pre-linux"]',
             'default-members = ["vendor/gpui-pre-linux"]', '', '[patch.crates-io]']
    for name, fields in PATCHES.items():
        entries = ", ".join(f"{key} = {json.dumps(value)}" for key, value in fields.items())
        lines.append(f"{name} = {{ {entries} }}")
    return "\n".join(lines) + "\n"


class Harness:
    def __init__(self, repo: Path, output: Path):
        runner_temp = Path(os.environ["RUNNER_TEMP"]).resolve(strict=True)
        require(sys.platform == "linux", "This qualification is hosted-Linux only")
        require(os.environ.get("GITHUB_ACTIONS") == "true", "Requires the admitted GitHub Actions job")
        require(os.environ.get("GITHUB_JOB") == "native", "Only the existing native job is admitted")
        require(repo == Path(os.environ["GITHUB_WORKSPACE"]).resolve(strict=True),
                "Repository must be the existing job checkout")
        require(output.parent.resolve(strict=True) == runner_temp,
                "Output must be a new direct child of RUNNER_TEMP")
        require(not output.exists() and not output.is_symlink(), "Refusing to reuse/overwrite output")
        output.mkdir(mode=0o700)
        self.repo, self.root = repo, output
        self.logs = output / "logs"
        self.logs.mkdir()
        self.status: dict[str, dict] = {}
        self.source_snapshots: dict[str, dict] = {}
        self.serial = 0
        self.lock = threading.Lock()
        self.env = dict(os.environ)
        prohibited = [key for key in self.env if key in {
            "RUSTC", "RUSTC_WRAPPER", "RUSTC_WORKSPACE_WRAPPER", "RUSTFLAGS",
            "CARGO_ENCODED_RUSTFLAGS", "RUSTUP_TOOLCHAIN", "CARGO_BUILD_RUSTC",
            "CARGO_BUILD_RUSTC_WRAPPER", "CARGO_BUILD_RUSTC_WORKSPACE_WRAPPER",
            "CARGO_BUILD_RUSTFLAGS", "CARGO_BUILD_TARGET",
        } or (key.startswith("CARGO_TARGET_") and key.endswith("_RUSTFLAGS"))]
        require(not prohibited, f"Unreviewed compiler override variables: {sorted(prohibited)}")
        for key in ("DISPLAY", "WAYLAND_DISPLAY", "XDG_ACTIVATION_TOKEN", "DESKTOP_STARTUP_ID"):
            self.env.pop(key, None)  # Never log these values or use an existing graphical session.
        self.build_env = {
            "CARGO_INCREMENTAL": "0", "CARGO_BUILD_JOBS": "1",
            "CARGO_PROFILE_DEV_DEBUG": "0", "CARGO_PROFILE_TEST_DEBUG": "0",
            "CARGO_TERM_COLOR": "never", "RUST_BACKTRACE": "1",
        }
        self.env.update(self.build_env)
        self.receipt = {
            "baseline_diffz": BASELINE, "gpui": GPUI, "gpui_tree": CORE_TREE,
            "vendor_tree": VENDOR_TREE, "target": TARGET, "toolchain": TOOLCHAIN,
            "harness_sha256": hashlib.sha256(Path(__file__).read_bytes()).hexdigest(),
            "build_environment": self.build_env, "initial_disk_free": shutil.disk_usage(output).free,
        }

    def save(self) -> None:
        data = {**self.receipt, "stages": self.status}
        (self.root / "result.json").write_text(json.dumps(data, indent=2) + "\n")

    def stage(self, label: str, callback):
        self.status[label] = {"state": "running"}
        self.save()
        try:
            value = callback()
        except Exception as error:
            detail = traceback.format_exc()
            print(f"\nFAILED {label}: {error}\n{detail}", flush=True)
            (self.logs / f"{label}.failure.txt").write_text(detail)
            self.status[label] = {"state": "failed", "error": str(error)}
            self.save()
            return False, None
        self.status[label] = {"state": "passed"}
        self.save()
        return True, value

    def block(self, labels: list[str], prerequisite: str) -> None:
        for label in labels:
            self.status[label] = {"state": "blocked", "prerequisite": prerequisite}
        self.save()

    def run(self, label: str, argv: list[str], cwd: Path, *, target_dir: Path | None = None,
            source_snapshot: tuple[Path, dict] | None = None, quiet_stdout: bool = False) -> Path:
        self.serial += 1
        stem = self.logs / f"{self.serial:03d}-{label}"
        stdout_path, stderr_path = Path(f"{stem}.stdout"), Path(f"{stem}.stderr")
        receipt_path = Path(f"{stem}.json")
        env = dict(self.env)
        if target_dir is not None:
            env["CARGO_TARGET_DIR"] = str(target_dir)
        record = {"argv": argv, "cwd": str(cwd), "build_environment": self.build_env,
                  "target_dir": str(target_dir) if target_dir else None,
                  "spawned": False,
                  "started_utc": time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime())}
        receipt_path.write_text(json.dumps(record, indent=2) + "\n")
        print(f"\n[{label}] cwd={cwd}\n$ {shlex.join(argv)}", flush=True)
        if source_snapshot is not None:
            require(fingerprint(source_snapshot[0]) == source_snapshot[1],
                    f"Source or lock changed before {label}")
        started = time.monotonic()
        interrupted = False
        failure = None
        returncode = None
        stream_errors = []
        with stdout_path.open("wb") as out, stderr_path.open("wb") as err:
            process = None
            threads = []
            def spool(pipe, destination, visible):
                try:
                    while chunk := pipe.read1(65536):
                        destination.write(chunk)
                        destination.flush()
                        if visible:
                            with self.lock:
                                sys.stdout.buffer.write(chunk)
                                sys.stdout.buffer.flush()
                except Exception as error:
                    with self.lock:
                        stream_errors.append(repr(error))
                finally:
                    pipe.close()
            try:
                process = subprocess.Popen(argv, cwd=cwd, env=env, stdout=subprocess.PIPE,
                                           stderr=subprocess.PIPE, start_new_session=True)
                threads = [threading.Thread(target=spool, args=(process.stdout, out, not quiet_stdout)),
                           threading.Thread(target=spool, args=(process.stderr, err, True))]
                for thread in threads:
                    thread.start()
                returncode = process.wait()
            except BaseException as error:
                failure = error
                interrupted = isinstance(error, KeyboardInterrupt)
                if process is not None:
                    try:
                        os.killpg(process.pid, signal.SIGTERM)
                    except ProcessLookupError:
                        pass
                    try:
                        returncode = process.wait(timeout=5)
                    except subprocess.TimeoutExpired:
                        try:
                            os.killpg(process.pid, signal.SIGKILL)
                        except ProcessLookupError:
                            pass
                        returncode = process.wait()
            finally:
                for thread in threads:
                    if thread.ident is not None:
                        thread.join()
        integrity_error = None
        try:
            intact = source_snapshot is None or fingerprint(source_snapshot[0]) == source_snapshot[1]
        except Exception as error:
            intact = False
            integrity_error = repr(error)
        record.update({"returncode": returncode, "elapsed_seconds": time.monotonic() - started,
                       "spawned": process is not None,
                       "ended_utc": time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime()),
                       "interrupted": interrupted, "source_and_lock_unchanged": intact,
                       "integrity_error": integrity_error,
                       "stream_errors": stream_errors,
                       "cumulative_child_maxrss_kib": resource.getrusage(resource.RUSAGE_CHILDREN).ru_maxrss})
        receipt_path.write_text(json.dumps(record, indent=2) + "\n")
        print("COMMAND_RESULT " + json.dumps(record, separators=(",", ":")), flush=True)
        if returncode and quiet_stdout:
            sys.stdout.buffer.write(stdout_path.read_bytes())
            sys.stdout.buffer.flush()
        if failure is not None:
            raise failure
        require(returncode == 0, f"{label} exited {returncode}; raw logs: {stem}.*")
        require(not stream_errors, f"{label} output capture failed: {stream_errors}")
        require(intact, f"{label} changed staged source or lock; no repair/unlocked retry allowed")
        return stdout_path

    def git(self, label: str, repo: Path, *args: str) -> bytes:
        return self.run(label, ["git", "-C", str(repo), *args], repo, quiet_stdout=True).read_bytes()

    def validate_inputs(self) -> None:
        require(not (self.repo / ".cargo").is_symlink(), "Repository Cargo config directory is a symlink")
        for name, expected in APP_BLOBS.items():
            verify_blob(self.repo / name, expected)
        manifest = tomllib.loads((self.repo / "Cargo.toml").read_text())
        require(manifest["patch"]["crates-io"] == PATCHES, "App fork patches differ from review")
        self.receipt["actual_diffz_head"] = self.git("diffz-head", self.repo, "rev-parse", "HEAD").decode().strip()
        tree = self.git("vendor-tree", self.repo, "rev-parse", "HEAD:vendor/gpui-pre-linux").decode().strip()
        require(tree == VENDOR_TREE, "Vendor tree differs from reviewed source")
        self.receipt["app_input_blobs"] = APP_BLOBS
        # Explicitly reject hidden Cargo compiler/source overrides, without printing contents.
        cargo_home = Path(self.env.get("CARGO_HOME", str(Path.home() / ".cargo"))).resolve()
        candidates = [cargo_home / "config", cargo_home / "config.toml"]
        for ancestor in [self.root, *self.root.parents, self.repo, *self.repo.parents]:
            candidates.extend([ancestor / ".cargo/config", ancestor / ".cargo/config.toml"])
        approved_config = self.repo / ".cargo/config.toml"
        present_configs = sorted({str(path) for path in candidates
                                  if path != approved_config and (path.exists() or path.is_symlink())})
        require(not present_configs,
                f"Applicable Cargo config requires separate review (not read/changed): {present_configs}")
        self.receipt["cargo_home"] = str(cargo_home)

    def validate_toolchain(self) -> None:
        versions = {}
        for binary in ("rustc", "cargo"):
            path = self.run(f"{binary}-version", ["rustup", "run", TOOLCHAIN, binary, "-vV"], self.repo)
            version = path.read_text()
            require(re.search(r"^release: 1\.99\.0$", version, re.MULTILINE) is not None,
                    f"Exact {binary} 1.99.0 required")
            if binary == "rustc":
                require(f"host: {TARGET}\n" in version, "Native hosted x86_64 Linux target required")
            versions[binary] = version
        self.receipt["versions"] = versions

    def export_tree(self, repo: Path, treeish: str, source: Path, destination: Path) -> None:
        require(not destination.exists(), f"Refusing existing source destination: {destination}")
        listing = self.git("tracked-source", repo, "ls-tree", "-r", "-z", treeish)
        entries = listing.split(b"\0")
        destination.mkdir(parents=True)
        for entry in entries:
            if not entry:
                continue
            header, raw_name = entry.split(b"\t", 1)
            mode, kind, expected = header.decode("ascii").split()
            name = PurePosixPath(raw_name.decode("utf-8"))
            require(not name.is_absolute() and ".." not in name.parts and ".git" not in name.parts,
                    "Unsafe tracked source path")
            require(kind == "blob" and mode in ("100644", "100755"),
                    f"Unreviewed symlink/submodule/file mode: {name}")
            original = source.joinpath(*name.parts)
            require(original.resolve().is_relative_to(source.resolve()), "Source symlink escape")
            data = verify_blob(original, expected)
            copied = destination.joinpath(*name.parts)
            copied.parent.mkdir(parents=True, exist_ok=True)
            copied.write_bytes(data)
            copied.chmod(int(mode, 8) & 0o777)

    def stage_vendor(self) -> Path:
        root = self.root / "vendor-stage"
        self.export_tree(self.repo, "HEAD:vendor/gpui-pre-linux",
                         self.repo / "vendor/gpui-pre-linux", root / "vendor/gpui-pre-linux")
        require(not (root / "vendor/gpui-pre-linux/Cargo.lock").exists(), "Unexpected nested vendor lock")
        (root / "Cargo.toml").write_text(vendor_manifest())
        (root / "Cargo.lock").write_bytes(verify_blob(self.repo / "Cargo.lock", APP_BLOBS["Cargo.lock"]))
        self.source_snapshots[str(root)] = fingerprint(root)
        return root

    def stage_core(self) -> Path:
        # This selects app workspace members, not a dependency-only -p/feature combination.
        before = {name: verify_blob(self.repo / name, expected) for name, expected in APP_BLOBS.items()}
        path = self.run("app-source-metadata", CARGO + ["metadata", "--manifest-path",
                        str(self.repo / "Cargo.toml"), "--locked", "--format-version", "1",
                        "--filter-platform", TARGET, "--all-features"], self.repo, quiet_stdout=True)
        for name, data in before.items():
            require(regular_bytes(self.repo / name) == data, "App input changed during metadata")
        metadata = read_metadata(path)
        expected_source = f"git+{PATCHES['gpui-pre']['git']}?rev={GPUI}#{GPUI}"
        packages = [p for p in metadata["packages"] if p["name"] == "gpui-pre"]
        require(len(packages) == 1 and packages[0]["version"] == "0.3.9"
                and packages[0]["source"] == expected_source, "App metadata did not select exact core")
        source = Path(packages[0]["manifest_path"]).resolve(strict=True).parent
        require(self.git("core-toplevel", source, "rev-parse", "--show-toplevel").decode().strip()
                == str(source), "Core manifest is not the exact repository root")
        require(self.git("core-head", source, "rev-parse", "HEAD").decode().strip() == GPUI,
                "Core checkout HEAD is not the full pinned revision")
        require(self.git("core-tree", source, "rev-parse", "HEAD^{tree}").decode().strip() == CORE_TREE,
                "Core checkout tree mismatch")
        root = self.root / "core-stage"
        self.export_tree(source, GPUI, source, root / "crates/gpui")
        for name, expected in CORE_BLOBS.items():
            verify_blob(root / "crates/gpui" / name, expected)
        fonts = []
        for name, expected, size in FONTS:
            url = f"https://raw.githubusercontent.com/zed-industries/zed/{ZED}/{name}"
            opener = urllib.request.build_opener(NoRedirect())
            with opener.open(url, timeout=30) as response:
                require(response.geturl() == url, "Unreviewed font redirect")
                data = response.read(size + 1)
            require(len(data) == size and blob_id(data) == expected, f"Authentic font mismatch: {name}")
            target = root / name
            target.parent.mkdir(parents=True, exist_ok=True)
            target.write_bytes(data)
            fonts.append({"path": name, "url": url, "bytes": size, "git_blob": expected,
                          "sha256": hashlib.sha256(data).hexdigest()})
        self.receipt["fonts"] = fonts
        print("Verified font provenance: " + json.dumps(fonts), flush=True)
        self.source_snapshots[str(root)] = fingerprint(root)
        return root

    def audit_graph(self, metadata: dict, root: Path, lane: str) -> dict:
        core = lane == "core"
        name = "gpui-pre" if core else "gpui-pre-linux"
        manifest = root / ("crates/gpui/Cargo.toml" if core else "vendor/gpui-pre-linux/Cargo.toml")
        packages = {p["id"]: p for p in metadata["packages"]}
        selected = [p for p in packages.values() if p["name"] == name and p["source"] is None]
        require(len(selected) == 1 and selected[0]["version"] == "0.3.9", "Wrong staged member")
        package = selected[0]
        require(Path(package["manifest_path"]).resolve() == manifest.resolve(), "Wrong member manifest")
        expected_root = root / "crates/gpui" if core else root
        require(Path(metadata["workspace_root"]).resolve() == expected_root.resolve(), "Wrong workspace root")
        for field in ("workspace_members", "workspace_default_members"):
            require(metadata[field] == [package["id"]], f"Unexpected {field}")
        nodes = {node["id"]: node for node in metadata["resolve"]["nodes"]}
        reachable, pending = set(), [package["id"]]
        while pending:
            current = pending.pop()
            if current in reachable:
                continue
            reachable.add(current)
            pending.extend(dep["pkg"] for dep in nodes[current]["deps"])
        lock_path = expected_root / "Cargo.lock"
        lock_data = tomllib.loads(lock_path.read_text())
        identities = {(p["name"], p["version"], p.get("source")): p for p in lock_data["package"]}
        for pid in reachable:
            p = packages[pid]
            key = (p["name"], p["version"], p["source"])
            require(key in identities, f"Resolved package absent from selected lock: {key}")
            if p["source"] and p["source"].startswith("registry+"):
                require(re.fullmatch(r"[0-9a-f]{64}", identities[key].get("checksum", "")) is not None,
                        f"Registry package lacks locked checksum: {key}")
            patch = PATCHES.get(p["name"])
            if not core and patch and "git" in patch:
                expected = f"git+{patch['git']}?rev={patch['rev']}#{patch['rev']}"
                require(p["source"] == expected, f"Fork substitution: {p['name']}")

        def features(package_name: str) -> set[str]:
            found = [pid for pid in reachable if packages[pid]["name"] == package_name]
            require(len(found) == 1, f"Expected one reachable {package_name}, got {len(found)}")
            return set(nodes[found[0]]["features"])

        active = features(name)
        gpui_features = features("gpui-pre")
        if core:
            require({"default", "font-kit", "windows-manifest", "test-support", "wayland", "x11"}
                    <= active, "Core default/test-support features missing")
        else:
            require(all(packages[pid]["version"] == "0.3.9" for pid in reachable
                        if packages[pid]["name"] in ("gpui-pre", "gpui-pre-wgpu")), "Wrong GPUI version")
            renderer = features("gpui-pre-wgpu")
            if lane == "vendor-unit":
                require({"wayland", "x11", "test-support"} <= active, "Vendor unit features missing")
                require("test-support" in gpui_features, "TestAppContext support missing")
            else:
                backend = lane.removeprefix("vendor-")
                opposite = "x11" if backend == "wayland" else "wayland"
                require(backend in active and not {opposite, "test-support", "default"} & active,
                        "Vendor backend feature isolation failed")
                require(not {opposite, "test-support"} & gpui_features, "Core opposite backend/test support leaked")
                require("test-support" not in renderer, "Renderer test-support leaked")
        receipt = {
            "lane": lane, "target": TARGET, "workspace_root": str(expected_root),
            "lock_git_blob": blob_id(lock_path.read_bytes()),
            "lock_sha256": hashlib.sha256(lock_path.read_bytes()).hexdigest(),
            "selected_features": sorted(active),
            "packages": [{"id": pid, "name": packages[pid]["name"], "version": packages[pid]["version"],
                          "source": packages[pid]["source"], "features": nodes[pid]["features"],
                          "deps": nodes[pid]["deps"]} for pid in sorted(reachable)],
        }
        (self.root / f"{lane}-resolved.json").write_text(json.dumps(receipt, indent=2) + "\n")
        # Compact complete graph remains in the existing job log; no new upload action is required.
        print("RESOLVED_GRAPH " + json.dumps(receipt, separators=(",", ":")), flush=True)
        return receipt

    def lane(self, root: Path, lane: str, features: list[str], tests: tuple[str, ...] = ()) -> None:
        core = lane == "core"
        manifest = root / ("crates/gpui/Cargo.toml" if core else "Cargo.toml")
        target = self.root / "target" / lane
        # Reuse the original stage fingerprint across all vendor lanes. Never bless
        # a prior command's source/lock mutation by taking a new baseline.
        snapshot = (root, self.source_snapshots[str(root)])
        common = ["--manifest-path", str(manifest), "--locked"]
        selection = [] if core else ["--package", "gpui-pre-linux"]
        labels = [f"{lane}-compile", f"{lane}-inventory"] + [f"{lane}-test-{i+1}" for i in range(len(tests))]
        if not tests:
            labels = [f"{lane}-check"]

        def metadata_stage():
            path = self.run(f"{lane}-metadata", CARGO + ["metadata", *common, "--format-version", "1",
                            "--filter-platform", TARGET, *features], root, target_dir=target,
                            source_snapshot=snapshot, quiet_stdout=True)
            return self.audit_graph(read_metadata(path), root, lane)

        ok, graph = self.stage(f"{lane}-metadata", metadata_stage)
        if not ok:
            self.block(labels, f"{lane}-metadata")
            return
        args = [*common, "--target", TARGET, *selection, *features, "--lib"]
        if not tests:
            def check_stage():
                path = self.run(f"{lane}-check", CARGO + ["check", *args, "--verbose"], root,
                                target_dir=target, source_snapshot=snapshot)
                stderr = path.with_suffix(".stderr").read_text()
                invocations = []
                for line in stderr.splitlines():
                    match = re.search(r"Running `(.*)`$", line)
                    if not match:
                        continue
                    words = shlex.split(match.group(1))
                    if "--crate-name" not in words or words[words.index("--crate-name") + 1] != "gpui_linux":
                        continue
                    cfg = [words[i+1] for i, word in enumerate(words[:-1]) if word == "--cfg"]
                    actual = {value[len('feature="'):-1] for value in cfg if value.startswith('feature="') and value.endswith('"')}
                    invocations.append(actual)
                require(len(invocations) == 1 and invocations[0] == set(graph["selected_features"]),
                        "Verbose compiler feature proof absent or differs from resolved vendor features")
            self.stage(f"{lane}-check", check_stage)
            return
        ok, _ = self.stage(f"{lane}-compile", lambda: self.run(f"{lane}-compile",
                           CARGO + ["test", *args, "--no-run"], root, target_dir=target, source_snapshot=snapshot))
        if not ok:
            self.block(labels[1:], f"{lane}-compile")
            return

        def inventory_stage():
            path = self.run(f"{lane}-inventory", CARGO + ["test", *args, "--", "--list"], root,
                            target_dir=target, source_snapshot=snapshot)
            counts = Counter(line.removesuffix(": test") for line in path.read_text().splitlines()
                             if line.endswith(": test"))
            require(all(counts[name] == 1 for name in tests), "Expected test missing or duplicated in inventory")

        ok, _ = self.stage(f"{lane}-inventory", inventory_stage)
        if not ok:
            self.block(labels[2:], f"{lane}-inventory")
            return
        for index, name in enumerate(tests, 1):
            def exact_test(test_name=name, number=index):
                path = self.run(f"{lane}-test-{number}", CARGO + ["test", *args, test_name, "--",
                                "--exact", "--nocapture", "--test-threads=1"], root,
                                target_dir=target, source_snapshot=snapshot)
                result = path.read_text()
                require(re.search(r"^running 1 test$", result, re.MULTILINE) is not None,
                        "Exact invocation did not run one test")
                require(re.search(r"^test " + re.escape(test_name) + r" \.\.\. ok$", result, re.MULTILINE) is not None,
                        "Expected exact named test did not report ok")
                require(re.search(r"^test result: ok\. 1 passed; 0 failed; 0 ignored;", result, re.MULTILINE) is not None,
                        "Single non-ignored passing test result absent")
            self.stage(f"{lane}-test-{index}", exact_test)

    def execute(self) -> int:
        valid, _ = self.stage("inputs", self.validate_inputs)
        compiler, _ = self.stage("toolchain", self.validate_toolchain) if valid else (False, None)
        if not valid or not compiler:
            self.block(["core", "vendor-unit", "vendor-wayland", "vendor-x11"], "inputs/toolchain")
            return 1
        vendor_ok, vendor = self.stage("vendor-source", self.stage_vendor)
        core_ok, core = self.stage("core-source", self.stage_core)
        if core_ok:
            self.lane(core, "core", CORE_FEATURES, CORE_TESTS)
        else:
            self.block(["core"], "core-source")
        if vendor_ok:
            self.lane(vendor, "vendor-unit", UNIT_FEATURES, VENDOR_TESTS)
            for backend in ("wayland", "x11"):
                self.lane(vendor, f"vendor-{backend}", ["--no-default-features", "--features", f"gpui-pre-linux/{backend}"])
        else:
            self.block(["vendor-unit", "vendor-wayland", "vendor-x11"], "vendor-source")
        self.receipt["final_disk_free"] = shutil.disk_usage(self.root).free
        self.save()
        print("QUALIFICATION_PROVENANCE " + json.dumps(self.receipt, sort_keys=True), flush=True)
        print("QUALIFICATION_RESULT " + json.dumps(self.status, sort_keys=True), flush=True)
        return 0 if all(item["state"] == "passed" for item in self.status.values()) else 1


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output", required=True, type=Path)
    args = parser.parse_args()
    harness = None

    def interrupted(signum, frame):
        raise KeyboardInterrupt

    signal.signal(signal.SIGTERM, interrupted)
    try:
        harness = Harness(Path.cwd().resolve(strict=True), args.output.absolute())
        return harness.execute()
    except KeyboardInterrupt:
        if harness is not None:
            harness.status["interruption"] = {"state": "failed", "error": "Interrupted; never a pass"}
            harness.save()
        print("Qualification interrupted; incomplete stages are not passes", file=sys.stderr, flush=True)
        return 130
    except Exception:
        traceback.print_exc()
        return 1


if __name__ == "__main__":
    sys.exit(main())
