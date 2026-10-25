#!/usr/bin/env python3
"""Qualify exact GPUI activation sources or collect unaccepted prerequisite evidence.

The default qualification mode never edits source or locks. An explicit
--vendor-lock-proposal-only diagnostic can resolve a disposable COPY offline;
it emits the full candidate and diff for review, never compiles/tests with it,
never changes app/core/seed locks, and exits nonzero because it is not a pass.
Adding --include-core-lock-proposal enables scoped online metadata resolution
for a separate core dev-dependency proposal. No proposal is compiled or accepted.
"""

from __future__ import annotations

import argparse
from collections import Counter
import gzip
import hashlib
import io
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
import tarfile
import threading
import time
import tomllib
import traceback
import urllib.parse
import urllib.request


BASELINE = "a3ab4f1bd071dd23372251c4de34098267ed050a"
GPUI = "d79c3d1191aa68d4753e1ca410c2d10c261b4572"
CORE_TREE = "337da148ab56dc5495613b86794464f28917e72d"
VENDOR_TREE = "af6afb57920f0d41bcbdd7c45ce4ea876515197a"
WGPU = "389e7f33bed4f6237ccb53f2b4884d996274a453"
ZED = "93d745668805029476e6d7fbb19652116fed51de"
TARGET = "x86_64-unknown-linux-gnu"
TOOLCHAIN = "1.99.0"
MACRO_PACKAGE = "gpui-pre-macros"
MACRO_VERSION = "0.3.9"
MACRO_CHECKSUM = "0bb80dac5e1f6b8cde1d2740389e77dfc0071bfcc81fba5526d5dc90f68171cc"
PROPTEST_MACRO_CHECKSUM = "e99782ea31698c2206c6887c1220d2f594b291c26cb9ec5b6d7cf175320bc212"
REGISTRY_SOURCE = "registry+https://github.com/rust-lang/crates.io-index"
CORE_PROPOSED_MANIFEST_BLOB = "3b5791e6586e39af96321fe939b3fead9c036889"
CORE_MANIFEST_PATCH_SHA256 = "92837a398a4c129af2ab7161c44be1d8f42bd3c8367ed25313cd5c302f7b5a4e"
ZED_HELPER_MANIFESTS = {
    "util_macros": ("crates/util_macros/Cargo.toml", "cf90bfa64ec8dab0b6376bbd05fea5de435420ea"),
    "perf": ("tooling/perf/Cargo.toml", "c7a7c7faa5f747ef0153911087655e1ded96353b"),
    "collections": ("crates/collections/Cargo.toml", "2a86e88d4f669e54a5db96fb6fa18c2a08625cdc"),
    "gpui_util": ("crates/gpui_util/Cargo.toml", "5586f369208373ca1df53684a4dc33550a6c778f"),
}
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


def bounded_regular_bytes(path: Path, limit: int) -> bytes:
    """Enforce a literal read bound even if a cached file changes after inspection."""
    require(not path.is_symlink(), f"Symlink not accepted: {path}")
    descriptor = os.open(path, os.O_RDONLY | os.O_NOFOLLOW | os.O_NONBLOCK)
    with os.fdopen(descriptor, "rb") as handle:
        require(stat.S_ISREG(os.fstat(handle.fileno()).st_mode), f"Required regular file: {path}")
        data = handle.read(limit + 1)
    require(len(data) <= limit, f"Diagnostic input exceeds {limit}-byte bound: {path}")
    return data


def bounded_cache_bytes(cargo_home: Path, relative: PurePosixPath, limit: int,
                        *, optional: bool = False) -> bytes | None:
    """Read one explicit cache file with no symlink traversal or directory scan."""
    require(not relative.is_absolute() and relative.parts
            and all(part not in ("", ".", "..") for part in relative.parts), "Unsafe cache path")
    directory = os.open(cargo_home, os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW)
    try:
        for part in relative.parts[:-1]:
            child = os.open(part, os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW, dir_fd=directory)
            os.close(directory)
            directory = child
        try:
            descriptor = os.open(relative.name, os.O_RDONLY | os.O_NOFOLLOW | os.O_NONBLOCK,
                                 dir_fd=directory)
        except FileNotFoundError:
            if optional:
                return None
            raise
        with os.fdopen(descriptor, "rb") as handle:
            require(stat.S_ISREG(os.fstat(handle.fileno()).st_mode), "Cache input is not a regular file")
            data = handle.read(limit + 1)
        require(len(data) <= limit, "Cached input exceeds its literal byte limit")
        return data
    finally:
        os.close(directory)


def pinned_cached_archive(cargo_home: Path, package_root: Path, package_name: str,
                          package_version: str, checksum: str) -> tuple[bytes, dict, dict]:
    """Authenticate one existing .crate; index bounded bytes, never extract or fetch."""
    prefix = package_name + "-" + package_version
    source_relative = package_root.relative_to(cargo_home / "registry/src")
    require(len(source_relative.parts) == 2 and source_relative.name == prefix,
            "Unexpected exact registry source-cache layout")
    registry = source_relative.parts[0]
    require(re.fullmatch(r"index[.]crates[.]io-[0-9a-f]+", registry) is not None,
            "Only the metadata-selected ordinary crates.io cache is admitted")
    relative = PurePosixPath("registry", "cache", registry, prefix + ".crate")
    compressed_limit, expanded_limit, member_limit = 1024 * 1024, 8 * 1024 * 1024, 256
    compressed = bounded_cache_bytes(cargo_home, relative, compressed_limit)
    require(hashlib.sha256(compressed).hexdigest() == checksum,
            "Cached registry archive differs from the original locked checksum")
    with gzip.GzipFile(fileobj=io.BytesIO(compressed), mode="rb") as stream:
        expanded = stream.read(expanded_limit + 1)
    require(len(expanded) <= expanded_limit, "Cached archive exceeds expanded byte limit")
    files, seen = {}, set()
    with tarfile.open(fileobj=io.BytesIO(expanded), mode="r:") as archive:
        for member in archive:
            require(len(seen) < member_limit, "Cached archive member-count limit exceeded")
            require(member.type in (tarfile.REGTYPE, tarfile.AREGTYPE, tarfile.DIRTYPE)
                    and member.sparse is None, "Archive links/special/sparse members refused")
            name = member.name[:-1] if member.isdir() and member.name.endswith("/") else member.name
            path = PurePosixPath(name)
            require(len(name) <= 512 and not path.is_absolute() and str(path) == name
                    and ".." not in path.parts and "\\" not in name
                    and path.parts and path.parts[0] == prefix,
                    "Unsafe or wrong-root archive member")
            require(name not in seen, "Duplicate archive member refused")
            seen.add(name)
            if member.isdir():
                require(member.size == 0, "Directory archive member has payload")
                continue
            require(len(path.parts) >= 2 and 0 <= member.size <= expanded_limit
                    and 0 <= member.offset_data <= len(expanded) - member.size,
                    "Archive member range is invalid")
            files[str(PurePosixPath(*path.parts[1:]))] = (member.offset_data, member.size)
    require("Cargo.toml" in files, "Authenticated archive has no normalized manifest")
    provenance = {"path": str(cargo_home.joinpath(*relative.parts)), "sha256": checksum,
                  "compressed_bytes": len(compressed), "expanded_bytes": len(expanded),
                  "members": len(seen), "compressed_limit_bytes": compressed_limit,
                  "expanded_limit_bytes": expanded_limit, "member_limit": member_limit,
                  "acquired": False, "independently_hashed": True,
                  "selection": "one metadata-derived cache path; no scan, extraction, fallback or network"}
    return expanded, files, provenance


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


def lock_identity(package: dict) -> tuple[str, str, str]:
    return package["name"], package["version"], package.get("source", "")


def lock_diff(before: dict, after: dict) -> dict:
    """Describe ALL changes; never decide that a version/source change is acceptable."""
    old = {lock_identity(p): p for p in before["package"]}
    new = {lock_identity(p): p for p in after["package"]}
    require(len(old) == len(before["package"]) and len(new) == len(after["package"]),
            "Duplicate package identity in proposal lock")
    return {
        "lock_format_before": before["version"], "lock_format_after": after["version"],
        "added": [new[key] for key in sorted(new.keys() - old.keys())],
        "removed": [old[key] for key in sorted(old.keys() - new.keys())],
        "changed": [{"identity": key, "before": old[key], "after": new[key]}
                    for key in sorted(old.keys() & new.keys()) if old[key] != new[key]],
        "patch_metadata_before": before.get("patch", {}),
        "patch_metadata_after": after.get("patch", {}),
        "all_identities_before": [old[key] for key in sorted(old)],
        "all_identities_after": [new[key] for key in sorted(new)],
        "automatic_acceptance": False,
    }


def verify_publishable_lock(lock: dict) -> None:
    """Full lock text is required evidence; never emit credential-bearing URLs."""
    packages = lock.get("package", []) + lock.get("patch", {}).get("unused", [])
    for package in packages:
        source = package.get("source")
        if not source:
            continue
        require(source.startswith(("registry+https://", "git+https://")),
                "Unreviewed lock source scheme; full-text emission withheld")
        url = urllib.parse.urlsplit(source.split("+", 1)[1])
        require(url.username is None and url.password is None,
                "Credential-bearing lock URL; full-text emission withheld")
        require(url.hostname in ("github.com", "index.crates.io"),
                "Unreviewed lock host; full-text emission withheld")
        require(all(key in ("rev", "branch", "tag") for key, _ in urllib.parse.parse_qsl(url.query, keep_blank_values=True)),
                "Unreviewed lock URL query; full-text emission withheld")


def rust_wrapper_body(source: str, function: str) -> str:
    declarations = list(re.finditer(r"(?m)^[ \t]*(?:pub(?:\([^)]*\))?[ \t]+)?fn[ \t]+"
                                    + re.escape(function) + r"\b", source))
    require(len(declarations) == 1, f"Cannot uniquely locate source wrapper {function}")
    prefix = (r"(?ms)^[ \t]*(?:pub(?:\([^)]*\))?[ \t]+)?fn[ \t]+"
              + re.escape(function) + r"\b[^\{]*\{")
    # Generated facade wrappers may be on one line; their body has no nested
    # blocks. Fall back only to a conventional outer closing-brace line.
    simple = list(re.finditer(prefix + r"(?P<body>[^{}]*)\}", source))
    if len(simple) == 1:
        return simple[0].group("body")
    pattern = prefix + r"(?P<body>.*?)^\}"
    matches = list(re.finditer(pattern, source))
    require(len(matches) == 1, f"Cannot uniquely locate source wrapper {function}")
    return matches[0].group("body")


def rust_wrapper_references(source: str, function: str) -> set[tuple[str, str]]:
    """Bounded source inspection, not a Rust parser or evaluation of the macro.

    Follow only an unambiguous module::function call in a short, conventionally
    formatted wrapper. Long/inline implementations are emitted without guessing
    further paths. The complete emitted source remains the review authority.
    """
    body = rust_wrapper_body(source, function)
    if len(body.encode("utf-8")) > 4096:
        return set()
    references = set(re.findall(r"\b([a-z_][a-z0-9_]*)::([a-z_][a-z0-9_]*)\s*\(", body))
    local = {(module, called) for module, called in references
             if re.search(r"(?m)^[ \t]*(?:pub(?:\([^)]*\))?[ \t]+)?mod[ \t]+"
                          + re.escape(module) + r"[ \t]*;", source)}
    return local


def declared_module_path(source: str, caller: str, module: str, files: dict, *, crate_root: bool = False) -> str:
    pattern = (r'(?m)^[ \t]*(?:pub(?:\([^)]*\))?[ \t]+)?mod[ \t]+'
               + re.escape(module) + r'[ \t]*;')
    matches = list(re.finditer(pattern, source))
    require(len(matches) == 1, f"Cannot uniquely locate declared local module {module}")
    parent = PurePosixPath(caller).parent
    prefix = source[:matches[0].start()]
    block = re.search(r'(?m)((?:^[ \t]*#\[[^\n]*\][ \t]*\n|^[ \t]*//[^\n]*\n|^[ \t]*\n)*)\Z', prefix)
    require(block is not None, "Cannot bound module attribute block")
    earlier = prefix[:block.start()].rstrip()
    if earlier:
        last = earlier.splitlines()[-1].strip()
        require(not ((last.endswith("]") or last.endswith("*/")) and not last.startswith("//")),
                "Unsupported/multiline module attribute block; refusing default-path fallback")
    explicit_paths = []
    for attribute in block.group(1).splitlines():
        if not attribute.strip() or attribute.lstrip().startswith("//"):
            continue
        name = re.match(r'\s*#\[\s*([A-Za-z_][A-Za-z0-9_:]*)', attribute)
        require(name is not None, "Unrecognized module attribute")
        if name.group(1) == "path":
            path_attribute = re.fullmatch(r'\s*#\[\s*path\s*=\s*"([^"\n]+)"\s*\]\s*', attribute)
            require(path_attribute is not None, "Unsupported explicit module path attribute")
            explicit_paths.append(path_attribute.group(1))
        elif name.group(1) == "cfg_attr":
            require(re.search(r'\bpath\s*=', attribute) is None,
                    "Conditional module path needs review; refusing default-path fallback")
    require(len(explicit_paths) <= 1, "Ambiguous explicit module paths")
    explicit = explicit_paths[0] if explicit_paths else None
    if explicit:
        require("\\" not in explicit, "Rust string escapes in module paths require review")
        child = PurePosixPath(explicit)
        require(not child.is_absolute() and ".." not in child.parts, "Unreviewed module path traversal")
        candidates = [str(parent / child)]
    else:
        filename = PurePosixPath(caller)
        base = parent if crate_root or filename.name in ("lib.rs", "mod.rs") else parent / filename.stem
        candidates = [str(base / f"{module}.rs"), str(base / module / "mod.rs")]
    declared = [path for path in candidates if path in files]
    require(len(declared) == 1, f"Declared module {module} missing or ambiguous in checksum manifest")
    return declared[0]


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
        self.cached_app_metadata = None
        self.core_seed = None
        self.cached_macro_source_bytes = 0
        self.initial_offline_value = os.environ.get("CARGO_NET_OFFLINE")
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
            source_snapshot: tuple[Path, dict] | None = None, quiet_stdout: bool = False,
            timeout_seconds: float | None = None) -> Path:
        self.serial += 1
        stem = self.logs / f"{self.serial:03d}-{label}"
        stdout_path, stderr_path = Path(f"{stem}.stdout"), Path(f"{stem}.stderr")
        receipt_path = Path(f"{stem}.json")
        env = dict(self.env)
        if target_dir is not None:
            env["CARGO_TARGET_DIR"] = str(target_dir)
        record = {"argv": argv, "cwd": str(cwd), "build_environment": self.build_env,
                  "target_dir": str(target_dir) if target_dir else None,
                  "integrity_root": str(source_snapshot[0]) if source_snapshot else None,
                  "timeout_seconds": timeout_seconds,
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
        cleanup_errors = []
        kill_authority = False
        custody_lost = False
        leader_exit_observed = False
        require(hasattr(os, "WNOWAIT") and hasattr(os, "waitid"), "Linux WNOWAIT custody is required")
        require(signal.getsignal(signal.SIGCHLD) == signal.SIG_DFL,
                "Unexpected SIGCHLD handler could consume child identity")
        with stdout_path.open("wb") as out, stderr_path.open("wb") as err:
            process = None
            threads = []

            def observe_once():
                nonlocal kill_authority, custody_lost, leader_exit_observed
                try:
                    information = os.waitid(os.P_PID, process.pid, os.WEXITED | os.WNOHANG | os.WNOWAIT)
                except ChildProcessError:
                    kill_authority = False
                    custody_lost = True
                    raise QualificationError("Direct-child custody lost (ECHILD); numeric signaling revoked")
                if information is not None:
                    leader_exit_observed = True
                return information

            def observe_for(seconds):
                deadline = None if seconds is None else time.monotonic() + seconds
                while True:
                    information = observe_once()
                    if information is not None:
                        return information
                    if deadline is not None and time.monotonic() >= deadline:
                        raise subprocess.TimeoutExpired(argv, seconds)
                    time.sleep(0.05 if deadline is None else min(0.05, max(0, deadline - time.monotonic())))

            def signal_owned_group(signum):
                # WNOWAIT retains the direct leader's PID/PGID identity even after
                # exit. No consuming wait/poll occurs while this authority exists.
                if not kill_authority or process is None:
                    return
                try:
                    observe_once()
                except QualificationError as error:
                    cleanup_errors.append(str(error))
                    return
                try:
                    os.killpg(process.pid, signum)
                except ProcessLookupError:
                    pass
                except OSError as error:
                    cleanup_errors.append(f"Owned process-group signal failed: {error}")

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
                kill_authority = True
                threads = [threading.Thread(target=spool, args=(process.stdout, out, not quiet_stdout), daemon=True),
                           threading.Thread(target=spool, args=(process.stderr, err, True), daemon=True)]
                for thread in threads:
                    thread.start()
                observe_for(timeout_seconds)
            except BaseException as error:
                failure = error
                interrupted = isinstance(error, KeyboardInterrupt)
                if process is not None and kill_authority:
                    signal_owned_group(signal.SIGTERM)
                    try:
                        observe_for(5)
                    except subprocess.TimeoutExpired:
                        signal_owned_group(signal.SIGKILL)
                        try:
                            observe_for(2)
                        except (subprocess.TimeoutExpired, QualificationError) as cleanup_error:
                            cleanup_errors.append(str(cleanup_error))
                    except QualificationError as cleanup_error:
                        cleanup_errors.append(str(cleanup_error))
            finally:
                # A terminated Cargo parent may leave Git descendants holding
                # pipes open. The leader remains unreaped throughout cleanup.
                if failure is not None:
                    signal_owned_group(signal.SIGKILL)
                drain_deadline = time.monotonic() + 5
                for thread in threads:
                    if thread.ident is not None:
                        thread.join(timeout=max(0, drain_deadline - time.monotonic()))
                if any(thread.is_alive() for thread in threads):
                    signal_owned_group(signal.SIGKILL)
                    final_drain_deadline = time.monotonic() + 2
                    for thread in threads:
                        if thread.ident is not None:
                            thread.join(timeout=max(0, final_drain_deadline - time.monotonic()))
                    if any(thread.is_alive() for thread in threads):
                        stream_errors.append("Output pipes did not close within the bounded cleanup interval")
                        if failure is None:
                            failure = QualificationError("Bounded output drain failed; raw logs may be incomplete")
                # All group signaling is now permanently closed. Only after
                # revocation may Popen consume/reap the leader. The strong local
                # reference also prevents destructor polling before revocation.
                kill_authority = False
                if process is not None and not custody_lost:
                    try:
                        returncode = process.wait(timeout=2)
                    except (subprocess.TimeoutExpired, ChildProcessError) as cleanup_error:
                        cleanup_errors.append(f"Bounded final reap failed: {cleanup_error}")
                if cleanup_errors and failure is None:
                    failure = QualificationError("Child lifecycle cleanup failed; see cleanup_errors receipt")
        integrity_error = None
        try:
            intact = source_snapshot is None or fingerprint(source_snapshot[0]) == source_snapshot[1]
        except Exception as error:
            intact = False
            integrity_error = repr(error)
        record.update({"returncode": returncode, "elapsed_seconds": time.monotonic() - started,
                       "spawned": process is not None,
                       "child_reaped": process is not None and returncode is not None,
                       "leader_exit_observed_wnowait": leader_exit_observed,
                       "child_custody_lost": custody_lost,
                       "numeric_signal_authority_revoked": not kill_authority,
                       "ended_utc": time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime()),
                       "interrupted": interrupted, "source_and_lock_unchanged": intact,
                       "timed_out": isinstance(failure, subprocess.TimeoutExpired),
                       "integrity_error": integrity_error,
                       "stream_errors": stream_errors,
                       "cleanup_errors": cleanup_errors,
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

    def stage_core(self, *, metadata: dict | None = None, include_fonts: bool = True) -> Path:
        # This selects app workspace members, not a dependency-only -p/feature combination.
        before = {name: verify_blob(self.repo / name, expected) for name, expected in APP_BLOBS.items()}
        if metadata is None:
            path = self.run("app-source-metadata", CARGO + ["metadata", "--manifest-path",
                            str(self.repo / "Cargo.toml"), "--locked", "--format-version", "1",
                            "--filter-platform", TARGET, "--all-features"], self.repo, quiet_stdout=True)
            metadata = read_metadata(path)
        for name, data in before.items():
            require(regular_bytes(self.repo / name) == data, "App input changed during metadata")
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
        for name, expected, size in FONTS if include_fonts else ():
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
        self.core_seed = root
        return root

    def audit_graph(self, metadata: dict, root: Path, lane: str, *, proposal: bool = False) -> dict:
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
            "evidence_scope": "unaccepted-lock-proposal" if proposal else "strict-locked-qualification",
            "lock_git_blob": blob_id(lock_path.read_bytes()),
            "lock_sha256": hashlib.sha256(lock_path.read_bytes()).hexdigest(),
            "selected_features": sorted(active),
            "packages": [{"id": pid, "name": packages[pid]["name"], "version": packages[pid]["version"],
                          "source": packages[pid]["source"], "features": nodes[pid]["features"],
                          "deps": nodes[pid]["deps"]} for pid in sorted(reachable)],
        }
        output_name = ("core-lock-proposal" if core else "vendor-lock-proposal") if proposal else lane
        (self.root / f"{output_name}-resolved.json").write_text(json.dumps(receipt, indent=2) + "\n")
        # Compact complete graph remains in the existing job log; no new upload action is required.
        marker = "PROPOSED_RESOLVED_GRAPH " if proposal else "RESOLVED_GRAPH "
        print(marker + json.dumps(receipt, separators=(",", ":")), flush=True)
        return receipt

    def propose_vendor_lock(self, seed: Path) -> None:
        """Only the proposal's Cargo.lock may change. Its bytes are NEVER accepted here."""
        original = self.source_snapshots[str(seed)]
        require(fingerprint(seed) == original, "Immutable vendor seed changed before proposal")
        proposal = self.root / "vendor-lock-proposal"
        require(not proposal.exists(), "Refusing an existing proposal directory")
        shutil.copytree(seed, proposal, symlinks=True)
        proposal_sources = {name: value for name, value in fingerprint(proposal).items()
                            if name != "Cargo.lock"}
        seed_bytes = verify_blob(seed / "Cargo.lock", APP_BLOBS["Cargo.lock"])
        require(regular_bytes(proposal / "Cargo.lock") == seed_bytes, "Proposal seed copy differs")
        self.env["CARGO_NET_OFFLINE"] = "true"
        self.build_env["CARGO_NET_OFFLINE"] = "true"
        metadata_path = None
        resolution_error = None
        try:
            metadata_path = self.run("vendor-lock-proposal-offline-metadata", CARGO + [
                "metadata", "--manifest-path", str(proposal / "Cargo.toml"),
                "--offline", "--format-version", "1", "--filter-platform", TARGET, *UNIT_FEATURES,
            ], proposal, target_dir=self.root / "target/vendor-lock-proposal-unused",
                source_snapshot=(seed, original), quiet_stdout=True)
        except Exception as error:
            # Preserve candidate evidence even when offline resolution cannot finish.
            # There is deliberately no unlocked-online or download fallback.
            resolution_error = error
        require(fingerprint(seed) == original, "Immutable vendor seed changed during proposal")
        require({name: value for name, value in fingerprint(proposal).items() if name != "Cargo.lock"}
                == proposal_sources, "Proposal resolution changed more than its own lock")
        for name, expected in APP_BLOBS.items():
            verify_blob(self.repo / name, expected)
        candidate_bytes = regular_bytes(proposal / "Cargo.lock")
        before = tomllib.loads(seed_bytes.decode("utf-8"))
        after = tomllib.loads(candidate_bytes.decode("utf-8"))
        verify_publishable_lock(before)
        verify_publishable_lock(after)
        diff = lock_diff(before, after)
        receipt = {
            "mode": "vendor-lock-proposal-only", "candidate_accepted": False,
            "qualification_pass": False, "build_test_or_check_invocations": 0,
            "seed_git_blob": blob_id(seed_bytes), "candidate_git_blob": blob_id(candidate_bytes),
            "seed_sha256": hashlib.sha256(seed_bytes).hexdigest(),
            "candidate_sha256": hashlib.sha256(candidate_bytes).hexdigest(),
            "seed_bytes": len(seed_bytes), "candidate_bytes": len(candidate_bytes),
            "seed_path": str(seed / "Cargo.lock"), "candidate_path": str(proposal / "Cargo.lock"),
            "manifest_sha256": hashlib.sha256(regular_bytes(proposal / "Cargo.toml")).hexdigest(),
            "source_tree": VENDOR_TREE, "patches_unchanged": PATCHES,
            "resolution_error": str(resolution_error) if resolution_error else None,
            "package_and_patch_diff": diff,
        }
        (self.root / "vendor-lock-proposal-receipt.json").write_text(json.dumps(receipt, indent=2) + "\n")
        for label, data in (("SEED", seed_bytes), ("CANDIDATE", candidate_bytes)):
            print(f"VENDOR_LOCK_{label}_BEGIN sha256={hashlib.sha256(data).hexdigest()} bytes={len(data)}", flush=True)
            sys.stdout.write(data.decode("utf-8"))
            if not data.endswith(b"\n"):
                sys.stdout.write("\n")
            print(f"VENDOR_LOCK_{label}_END", flush=True)
        print("VENDOR_LOCK_PROPOSAL_DIFF " + json.dumps(receipt, separators=(",", ":")), flush=True)
        self.receipt["vendor_lock_proposal"] = {
            key: receipt[key] for key in ("mode", "candidate_accepted", "qualification_pass",
                                         "seed_sha256", "candidate_sha256", "resolution_error")
        }
        if resolution_error is not None:
            raise resolution_error
        # Audit only. A candidate graph passing these assertions is still NOT an accepted lock.
        self.audit_graph(read_metadata(metadata_path), proposal, "vendor-unit", proposal=True)
        self.status["vendor-lock-review"] = {
            "state": "blocked", "prerequisite": "independent review and explicit candidate acceptance",
        }
        self.save()
        print("VENDOR_LOCK_PROPOSAL_REQUIRES_REVIEW: no compiler/test/check will use this candidate", flush=True)

    def inspect_cached_macro(self, seed: Path, *, package_name: str = MACRO_PACKAGE,
                             package_version: str = MACRO_VERSION, package_checksum: str = MACRO_CHECKSUM,
                             resolved_metadata: dict | None = None, selection_lock: Path | None = None,
                             selection_lock_blob: str | None = None) -> None:
        """Read only the already-resolved package and explicitly declared wrapper paths."""
        lock_path = selection_lock or self.repo / "Cargo.lock"
        lock_blob = selection_lock_blob or APP_BLOBS["Cargo.lock"]
        source_lock = tomllib.loads(verify_blob(lock_path, lock_blob).decode())
        locked = [package for package in source_lock["package"] if package["name"] == package_name]
        require(len(locked) == 1 and locked[0]["version"] == package_version
                and locked[0]["source"] == REGISTRY_SOURCE
                and locked[0]["checksum"] == package_checksum, "Selected macro package/checksum differs from admitted input")
        self.env["CARGO_NET_OFFLINE"] = "true"
        self.build_env["CARGO_NET_OFFLINE"] = "true"
        if resolved_metadata is None:
            metadata_path = self.run("cached-macro-offline-locked-metadata", CARGO + [
                "metadata", "--manifest-path", str(self.repo / "Cargo.toml"), "--offline", "--locked",
                "--format-version", "1", "--filter-platform", TARGET, "--all-features",
            ], self.repo, source_snapshot=(seed, self.source_snapshots[str(seed)]), quiet_stdout=True)
            metadata = read_metadata(metadata_path)
            self.cached_app_metadata = metadata
        else:
            metadata = resolved_metadata
        for name, expected in APP_BLOBS.items():
            verify_blob(self.repo / name, expected)
        packages = [p for p in metadata["packages"] if p["name"] == package_name]
        require(len(packages) == 1 and packages[0]["version"] == package_version
                and packages[0]["source"] == REGISTRY_SOURCE, "Offline metadata macro identity mismatch")
        package = packages[0]
        manifest = Path(package["manifest_path"])
        require(manifest.name == "Cargo.toml" and not manifest.is_symlink(), "Unexpected macro manifest path")
        root = manifest.parent.resolve(strict=True)
        cargo_home = Path(self.receipt["cargo_home"])
        require(root.is_relative_to(cargo_home / "registry/src"), "Metadata path is outside the admitted registry source cache")
        require(manifest == root / "Cargo.toml", "Metadata manifest has a noncanonical parent")
        archive_bytes, files, archive_provenance = pinned_cached_archive(
            cargo_home, root, package_name, package_version, package_checksum)
        records = []
        total_bytes = 0
        receipt = {
            "package": package_name, "version": package_version, "source": REGISTRY_SOURCE,
            "lock_package_checksum": package_checksum, "metadata_package_id": package["id"],
            "selection_lock": str(lock_path), "selection_lock_git_blob": lock_blob,
            "selection_context": "original core own-lock" if selection_lock else "application lock",
            "metadata_manifest_path": str(manifest), "canonical_package_root": str(root),
            "registry_archive": archive_provenance,
            "verification_scope": "selected regular archive members bound to original locked compressed-archive SHA256 and byte-compared with metadata-selected source-cache files",
            "registry_archive_acquired": False, "registry_archive_independently_hashed": True,
            "file_limit_bytes": 131072, "total_limit_bytes": 393216,
            "total_budget_shared_across_cached_packages": True,
            "source_files": records,
        }
        print("CACHED_MACRO_PROVENANCE " + json.dumps(receipt, separators=(",", ":")), flush=True)

        def read_source(relative: str, reason: str, *, optional: bool = False) -> str | None:
            nonlocal total_bytes
            path = PurePosixPath(relative)
            require(not path.is_absolute() and ".." not in path.parts, "Unsafe explicit macro source path")
            require(str(path) == relative and "\\" not in relative, "Noncanonical macro source path")
            local_relative = PurePosixPath(root.relative_to(cargo_home).as_posix(), relative)
            if optional and relative not in files:
                require(bounded_cache_bytes(cargo_home, local_relative, 131072, optional=True) is None,
                        "Source-cache optional file is absent from authenticated archive")
                record = {"package": package_name, "path": relative, "reason": reason, "state": "optional-absent"}
                records.append(record)
                print("CACHED_MACRO_SOURCE_OPTIONAL_ABSENT " + json.dumps(record), flush=True)
                return None
            require(relative in files, f"Explicit macro source missing from authenticated archive: {relative}")
            offset, length = files[relative]
            require(length <= 131072, "Selected archive member exceeds source-file bound")
            require(self.cached_macro_source_bytes + length <= 393216, "Shared macro-source total diagnostic bound exceeded")
            data = archive_bytes[offset:offset + length]
            require(bounded_cache_bytes(cargo_home, local_relative, 131072) == data,
                    f"Metadata-selected cached source differs from authenticated archive: {relative}")
            digest = hashlib.sha256(data).hexdigest()
            text = data.decode("utf-8")
            total_bytes += len(data)
            self.cached_macro_source_bytes += len(data)
            record = {"package": package_name, "path": relative, "reason": reason, "state": "locked-archive-and-source-cache-validated",
                      "bytes": len(data), "sha256": digest}
            records.append(record)
            print("CACHED_MACRO_SOURCE_BEGIN " + json.dumps(record, separators=(",", ":")), flush=True)
            sys.stdout.write(text)
            if not data.endswith(b"\n"):
                sys.stdout.write("\n")
            print("CACHED_MACRO_SOURCE_END " + package_name + "/" + relative, flush=True)
            return text

        def finish_source_receipt():
            receipt["total_source_bytes"] = total_bytes
            receipt["shared_source_bytes_so_far"] = self.cached_macro_source_bytes
            output = "cached-macro-source-receipt.json" if package_name == MACRO_PACKAGE else "cached-proptest-macro-source-receipt.json"
            (self.root / output).write_text(json.dumps(receipt, indent=2) + "\n")
            print("CACHED_MACRO_SOURCE_RESULT " + json.dumps(receipt, separators=(",", ":")), flush=True)

        normalized = read_source("Cargo.toml", "normalized resolved package manifest")
        manifest_data = tomllib.loads(normalized)
        require(manifest_data["package"]["name"] == package_name
                and manifest_data["package"]["version"] == package_version, "Cached normalized manifest identity mismatch")
        library_name = manifest_data.get("lib", {}).get("path", "src/lib.rs")
        require(isinstance(library_name, str), "Malformed declared macro library path")
        library_path = PurePosixPath(library_name)
        require(not library_path.is_absolute() and ".." not in library_path.parts,
                "Absolute/traversing macro library path refused")
        library_name = str(library_path)
        library_targets = [target for target in package["targets"] if "proc-macro" in target["kind"]]
        require(len(library_targets) == 1
                and Path(library_targets[0]["src_path"]).resolve() == root.joinpath(*library_path.parts).resolve()
                and root.joinpath(*library_path.parts).resolve().is_relative_to(root),
                "Manifest library target differs from exact metadata target or escapes package")
        if package_name == MACRO_PACKAGE:
            read_source("Cargo.toml.orig", "optional original package manifest", optional=True)
            read_source(".cargo_vcs_info.json", "optional package VCS provenance", optional=True)
        library = read_source(library_name, "exact manifest-declared/metadata-confirmed procedural-macro library entrypoint")
        declared_property = re.search(r"(?m)^[ \t]*(?:pub(?:\([^)]*\))?[ \t]+)?mod[ \t]+property_test[ \t]*;", library)
        already_read_property = None
        property_source = None
        if declared_property:
            already_read_property = declared_module_path(library, library_name, "property_test", files, crate_root=True)
            property_source = read_source(already_read_property, "explicitly declared property_test implementation module")
            receipt["property_test_location"] = already_read_property
        if package_name == "proptest-macro":
            require(already_read_property is not None, "No explicit proptest property_test module; further source scope not inferred")
            children = []
            for child in ("options", "codegen"):
                child_path = declared_module_path(property_source, already_read_property, child, files)
                read_source(child_path, f"root-approved declared proptest property-test {child} child")
                children.append(child_path)
            receipt["approved_property_children"] = children
            finish_source_receipt()
            return
        references = rust_wrapper_references(library, "property_test")
        receipt["entrypoints_inspected"] = ["property_test"]
        # Some published packages rename the original function and introduce an
        # outer crate-path-rewriting facade. Follow that alias only when the
        # exported wrapper explicitly calls it, and only inside the same lib file.
        if re.search(r"\b__gpui_pre_property_test\s*\(", rust_wrapper_body(library, "property_test")):
            references |= rust_wrapper_references(library, "__gpui_pre_property_test")
            receipt["entrypoints_inspected"].append("__gpui_pre_property_test")
        primary = {(module, function) for module, function in references if module != "gpui_pre_facade_paths"}
        if declared_property:
            module, function = "property_test", "(declared implementation module)"
            require(all(name == "property_test" for name, _ in primary), "Ambiguous property-test implementation references")
        elif primary:
            require(len(primary) == 1, "Ambiguous property-test implementation references")
            module, function = next(iter(primary))
        else:
            module = None
        if module is not None:
            relative = declared_module_path(library, library_name, module, files, crate_root=True)
            if relative != already_read_property:
                read_source(relative, f"library declares property-test implementation {module}::{function}")
            receipt["property_test_location"] = relative
        else:
            receipt["property_test_location"] = library_name + " (no external implementation module followed)"
        facade = {(name, called) for name, called in references if name == "gpui_pre_facade_paths"}
        if facade:
            require(len(facade) == 1, "Ambiguous source facade calls")
            helper_module, helper_function = next(iter(facade))
            helper_path = declared_module_path(library, library_name, helper_module, files, crate_root=True)
            require(helper_path == "src/gpui_pre_facade_paths.rs", "Unreviewed facade-helper path")
            read_source(helper_path, f"one additional facade: exported/explicit alias wrapper calls {helper_module}::{helper_function}")
            receipt["additional_facade"] = {"path": helper_path, "called_function": helper_function}
            # Exactly one additional local facade/helper is allowed. Never recurse further.
        finish_source_receipt()

    def inspect_cached_proptest_macro(self) -> None:
        """Original core lock selects 0.6.0; app's different 0.5.0 is not substituted."""
        require(self.cached_app_metadata is not None, "Original core source requires successful cached app metadata")
        seed = self.core_seed or self.stage_core(metadata=self.cached_app_metadata, include_fonts=False)
        self.env["CARGO_NET_OFFLINE"] = "true"
        self.build_env["CARGO_NET_OFFLINE"] = "true"
        path = self.run("cached-proptest-original-core-offline-locked-metadata", CARGO + [
            "metadata", "--manifest-path", str(seed / "crates/gpui/Cargo.toml"),
            "--locked", "--offline", "--format-version", "1", "--filter-platform", TARGET, *CORE_FEATURES,
        ], seed, source_snapshot=(seed, self.source_snapshots[str(seed)]), quiet_stdout=True)
        metadata = read_metadata(path)
        self.audit_graph(metadata, seed, "core")
        self.inspect_cached_macro(seed, package_name="proptest-macro", package_version="0.6.0",
                                  package_checksum=PROPTEST_MACRO_CHECKSUM, resolved_metadata=metadata,
                                  selection_lock=seed / "crates/gpui/Cargo.lock",
                                  selection_lock_blob=CORE_BLOBS["Cargo.lock"])

    def propose_core_lock(self) -> None:
        """Separate, explicitly online proposal; only the reviewed manifest/lock copy changes."""
        require(self.cached_app_metadata is not None,
                "Core proposal requires successful app offline locked metadata; no source-search fallback")
        require(self.initial_offline_value not in ("true", "1"),
                "Inherited offline restriction requires review; it will not be overridden")
        seed = self.core_seed or self.stage_core(metadata=self.cached_app_metadata, include_fonts=False)
        original = self.source_snapshots[str(seed)]
        proposal = self.root / "core-lock-proposal"
        require(not proposal.exists(), "Refusing existing core proposal directory")
        shutil.copytree(seed, proposal, symlinks=True)
        manifest_path = proposal / "crates/gpui/Cargo.toml"
        original_manifest = verify_blob(manifest_path, CORE_BLOBS["Cargo.toml"])
        anchor = b"[build-dependencies.embed-resource]\n"
        addition = (f'[dev-dependencies.util_macros]\ngit = "https://github.com/zed-industries/zed"\n'
                    f'rev = "{ZED}"\n\n').encode("utf-8")
        require(original_manifest.count(anchor) == 1, "Core manifest insertion anchor changed")
        proposed_manifest = original_manifest.replace(anchor, addition + anchor)
        require(blob_id(proposed_manifest) == CORE_PROPOSED_MANIFEST_BLOB, "Manifest differs from exact reviewed four-line patch")
        manifest_path.write_bytes(proposed_manifest)
        proposal_sources = {name: value for name, value in fingerprint(proposal).items()
                            if name != "crates/gpui/Cargo.lock"}
        seed_bytes = verify_blob(seed / "crates/gpui/Cargo.lock", CORE_BLOBS["Cargo.lock"])
        scope = {
            "mode": "explicit-online-core-lock-proposal", "candidate_accepted": False,
            "git_repository": "https://github.com/zed-industries/zed", "git_revision": ZED,
            "permitted_git_packages": sorted(ZED_HELPER_MANIFESTS),
            "registry_scope": "ordinary crates.io metadata/source needed for the reviewed util_macros -> perf -> collections -> gpui_util closure and existing exact core graph",
            "package_code_execution": False, "compilation_tests_checks": False,
            "configuration_security_runner_changes": False,
            "reviewed_manifest_patch_sha256": CORE_MANIFEST_PATCH_SHA256,
            "command_timeout_seconds": 120,
            "maximum_parent_and_pipe_cleanup_seconds": 16,
            "enclosing_step_timeout_minutes": 5, "unchanged_job_timeout_minutes": 30,
        }
        self.receipt["core_proposal_acquisition_scope"] = scope
        print("CORE_LOCK_PROPOSAL_ACQUISITION_SCOPE " + json.dumps(scope, separators=(",", ":")), flush=True)
        # This explicit online lane is separate from the offline vendor/macro lanes.
        # Change only this subprocess environment; no Cargo/global/security config is edited.
        self.env["CARGO_NET_OFFLINE"] = "false"
        self.build_env["CARGO_NET_OFFLINE"] = "false"
        prior_retry = self.env.get("CARGO_NET_RETRY")
        self.env["CARGO_NET_RETRY"] = "0"
        self.build_env["CARGO_NET_RETRY"] = "0"
        disk_before = shutil.disk_usage(self.root).free
        acquisition_started = time.monotonic()
        metadata_path = None
        resolution_error = None
        try:
            metadata_path = self.run("core-lock-proposal-online-metadata", CARGO + [
                "metadata", "--manifest-path", str(manifest_path), "--format-version", "1",
                "--filter-platform", TARGET, *CORE_FEATURES,
            ], proposal, target_dir=self.root / "target/core-lock-proposal-unused",
                source_snapshot=(seed, original), quiet_stdout=True, timeout_seconds=120)
        except Exception as error:
            resolution_error = error
        finally:
            self.env["CARGO_NET_OFFLINE"] = "true"
            self.build_env["CARGO_NET_OFFLINE"] = "true"
            if prior_retry is None:
                self.env.pop("CARGO_NET_RETRY", None)
            else:
                self.env["CARGO_NET_RETRY"] = prior_retry
            self.build_env.pop("CARGO_NET_RETRY", None)
            disk_after = shutil.disk_usage(self.root).free
            resource_receipt = {"disk_free_before": disk_before, "disk_free_after": disk_after,
                                "observed_disk_delta_bytes": disk_before - disk_after,
                                "elapsed_seconds": time.monotonic() - acquisition_started}
            self.receipt["core_proposal_acquisition_resources"] = resource_receipt
            print("CORE_LOCK_PROPOSAL_RESOURCE_RESULT " + json.dumps(resource_receipt), flush=True)
        require(fingerprint(seed) == original, "Immutable original core stage changed")
        require({name: value for name, value in fingerprint(proposal).items() if name != "crates/gpui/Cargo.lock"}
                == proposal_sources, "Core proposal changed outside reviewed manifest/own lock")
        for name, expected in APP_BLOBS.items():
            verify_blob(self.repo / name, expected)
        candidate_bytes = regular_bytes(proposal / "crates/gpui/Cargo.lock")
        before, after = tomllib.loads(seed_bytes.decode()), tomllib.loads(candidate_bytes.decode())
        verify_publishable_lock(before)
        verify_publishable_lock(after)
        receipt = {
            "acquisition_scope": scope, "candidate_accepted": False, "qualification_pass": False,
            "seed_git_blob": blob_id(seed_bytes), "candidate_git_blob": blob_id(candidate_bytes),
            "seed_sha256": hashlib.sha256(seed_bytes).hexdigest(),
            "candidate_sha256": hashlib.sha256(candidate_bytes).hexdigest(),
            "manifest_before_git_blob": CORE_BLOBS["Cargo.toml"],
            "manifest_after_git_blob": blob_id(proposed_manifest),
            "manifest_after_sha256": hashlib.sha256(proposed_manifest).hexdigest(),
            "manifest_addition": addition.decode(), "source_tree": CORE_TREE,
            "resolution_error": str(resolution_error) if resolution_error else None,
            "package_and_patch_diff": lock_diff(before, after),
        }
        for label, data in (("SEED", seed_bytes), ("CANDIDATE", candidate_bytes)):
            print(f"CORE_LOCK_{label}_BEGIN sha256={hashlib.sha256(data).hexdigest()} bytes={len(data)}", flush=True)
            sys.stdout.write(data.decode())
            if not data.endswith(b"\n"):
                sys.stdout.write("\n")
            print(f"CORE_LOCK_{label}_END", flush=True)
        print("CORE_LOCK_PROPOSAL_DIFF " + json.dumps(receipt, separators=(",", ":")), flush=True)
        (self.root / "core-lock-proposal-receipt.json").write_text(json.dumps(receipt, indent=2) + "\n")
        if resolution_error is not None:
            raise resolution_error
        metadata = read_metadata(metadata_path)
        graph = self.audit_graph(metadata, proposal, "core", proposal=True)
        git_packages = [package for package in metadata["packages"]
                        if package["source"] and package["source"].startswith("git+")]
        expected_source = f"git+https://github.com/zed-industries/zed?rev={ZED}#{ZED}"
        require({p["name"] for p in git_packages} == set(ZED_HELPER_MANIFESTS) and len(git_packages) == 4,
                "Unexpected Git package closure; proposal not accepted")
        require(all(p["version"] == "0.1.0" and p["source"] == expected_source for p in git_packages),
                "Git helper package source/version differs from reviewed closure")
        packages = {p["id"]: p for p in metadata["packages"]}
        nodes = {node["id"]: node for node in metadata["resolve"]["nodes"]}
        core_id = metadata["workspace_members"][0]
        util = next(p for p in git_packages if p["name"] == "util_macros")
        edge = [dep for dep in nodes[core_id]["deps"] if dep["pkg"] == util["id"]]
        require(len(edge) == 1 and edge[0]["dep_kinds"] and all(kind["kind"] == "dev" for kind in edge[0]["dep_kinds"]),
                "util_macros is not exclusively the reviewed core dev-dependency")
        perf = next(p for p in git_packages if p["name"] == "perf")
        require(not {"default", "cli"} & set(nodes[perf["id"]]["features"]), "Unreviewed perf CLI/default features")
        syn_edges = [dep for dep in nodes[util["id"]]["deps"] if packages[dep["pkg"]]["name"] == "syn"]
        require(len(syn_edges) == 1 and packages[syn_edges[0]["pkg"]]["version"].startswith("2."), "Expected syn 2 helper edge")
        require({"full", "extra-traits", "clone-impls", "parsing", "printing"}
                <= set(nodes[syn_edges[0]["pkg"]]["features"]), "Reviewed syn helper features missing")
        util_manifest = Path(util["manifest_path"]).resolve(strict=True)
        git_root = Path(self.git("zed-source-root", util_manifest.parent, "rev-parse", "--show-toplevel").decode().strip()).resolve(strict=True)
        require(self.git("zed-source-head", git_root, "rev-parse", "HEAD").decode().strip() == ZED,
                "Acquired official Git checkout is not the full admitted revision")
        workspace_manifest = verify_blob(git_root / "Cargo.toml", "b3d85a3bee127534672d2de08fba9272c7c7693d")
        manifest_evidence = {"workspace": {"path": "Cargo.toml", "git_blob": "b3d85a3bee127534672d2de08fba9272c7c7693d",
                                             "sha256": hashlib.sha256(workspace_manifest).hexdigest()}}
        for package in git_packages:
            relative, expected = ZED_HELPER_MANIFESTS[package["name"]]
            require(Path(package["manifest_path"]).resolve() == (git_root / relative).resolve(), "Unexpected helper manifest path")
            data = verify_blob(git_root / relative, expected)
            manifest_evidence[package["name"]] = {"path": relative, "git_blob": expected,
                                                   "sha256": hashlib.sha256(data).hexdigest()}
        receipt["helper_manifest_evidence"] = manifest_evidence
        receipt["resolved_packages"] = len(graph["packages"])
        (self.root / "core-lock-proposal-receipt.json").write_text(json.dumps(receipt, indent=2) + "\n")
        print("CORE_LOCK_PROPOSAL_SOURCE_EVIDENCE " + json.dumps(manifest_evidence, separators=(",", ":")), flush=True)
        self.status["core-lock-review"] = {"state": "blocked", "prerequisite": "independent review and explicit acceptance; property macro error remains separate"}
        self.save()
        print("CORE_LOCK_PROPOSAL_REQUIRES_REVIEW: no compiler/test/check uses this candidate", flush=True)

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

    def execute(self, *, proposal_only: bool = False, core_proposal: bool = False) -> int:
        self.receipt["mode"] = ("prerequisite-evidence-with-online-core-proposal" if core_proposal
                                else "vendor-lock-proposal-only" if proposal_only else "strict-qualification")
        valid, _ = self.stage("inputs", self.validate_inputs)
        compiler, _ = self.stage("toolchain", self.validate_toolchain) if valid else (False, None)
        if not valid or not compiler:
            self.block(["core", "vendor-unit", "vendor-wayland", "vendor-x11"], "inputs/toolchain")
            return 1
        vendor_ok, vendor = self.stage("vendor-source", self.stage_vendor)
        if proposal_only:
            if vendor_ok:
                macro_ok, _ = self.stage("cached-macro-source", lambda: self.inspect_cached_macro(vendor))
                generated, _ = self.stage("vendor-lock-proposal", lambda: self.propose_vendor_lock(vendor))
                core_generated = True
                if core_proposal:
                    core_generated, _ = self.stage("core-lock-proposal", self.propose_core_lock)
                # Run after any separately authorized online core proposal so
                # legitimate cache population may help, but select ONLY the
                # unchanged original lock and stay strictly offline here.
                proptest_ok, _ = self.stage("cached-proptest-macro-source", self.inspect_cached_proptest_macro)
            else:
                macro_ok = False
                generated = False
                core_generated = False
                proptest_ok = False
                self.block(["cached-macro-source", "vendor-lock-proposal", "cached-proptest-macro-source"], "vendor-source")
                if core_proposal:
                    self.block(["core-lock-proposal"], "vendor-source")
            self.receipt["qualification_pass"] = False
            self.save()
            print("VENDOR_LOCK_PROPOSAL_RESULT " + json.dumps(self.status, sort_keys=True), flush=True)
            # Review-ready proposal is deliberately nonzero, never a green qualification job.
            return 2 if generated and macro_ok and core_generated and proptest_ok else 1
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
    parser.add_argument("--vendor-lock-proposal-only", action="store_true",
                        help="Offline disposable-lock plus bounded cached-macro-source diagnostics; never compiles/tests or accepts its candidate")
    parser.add_argument("--include-core-lock-proposal", action="store_true",
                        help="Explicitly add online official-Zed/crates.io acquisition for the reviewed unaccepted core dev-dependency lock proposal")
    args = parser.parse_args()
    if args.include_core_lock_proposal and not args.vendor_lock_proposal_only:
        parser.error("--include-core-lock-proposal requires the non-compiling proposal-only mode")
    harness = None

    def interrupted(signum, frame):
        raise KeyboardInterrupt

    signal.signal(signal.SIGTERM, interrupted)
    try:
        harness = Harness(Path.cwd().resolve(strict=True), args.output.absolute())
        return harness.execute(proposal_only=args.vendor_lock_proposal_only, core_proposal=args.include_core_lock_proposal)
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
