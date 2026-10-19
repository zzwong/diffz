#!/usr/bin/env python3
"""Qualify the existing Linux Bazel graph against a cache on two real hosts.

This records observed native inputs; it does not make the graph hermetic.
See docs/bazel-cache-qualification.md for the service and host prerequisites.
"""

import argparse
from contextlib import contextmanager
import fcntl
import hashlib
import json
import os
from pathlib import Path
import platform
import re
import shutil
import socket
import subprocess
import sys
import time
from urllib.parse import unquote, urlsplit


SCHEMA = 1
ROOT = Path(__file__).resolve().parent.parent
TEST_ROOT = "/var/tmp/diffz-bazel-qualification-tests"
NATIVE_PACKAGES = (
    "fontconfig", "freetype2", "xkbcommon", "xkbcommon-x11",
    "wayland-client", "wayland-cursor", "x11", "openssl",
)
FORBIDDEN_ENV = (
    "BAZEL_RUST_STDLIB_LINKFLAGS", "CFLAGS", "CXXFLAGS", "CPPFLAGS",
    "LDFLAGS", "CPATH", "C_INCLUDE_PATH", "CPLUS_INCLUDE_PATH",
    "LIBRARY_PATH", "LD_LIBRARY_PATH", "PKG_CONFIG_PATH",
    "PKG_CONFIG_LIBDIR", "PKG_CONFIG_SYSROOT_DIR", "CMAKE_PREFIX_PATH",
    "CMAKE_TOOLCHAIN_FILE", "RUSTFLAGS", "CARGO_ENCODED_RUSTFLAGS",
)


class QualificationError(Exception):
    pass


def digest(value):
    return hashlib.sha256(json.dumps(value, sort_keys=True, separators=(",", ":")).encode()).hexdigest()


def file_digest(path):
    hasher = hashlib.sha256()
    with Path(path).open("rb") as stream:
        for block in iter(lambda: stream.read(1024 * 1024), b""):
            hasher.update(block)
    return hasher.hexdigest()


def write_json(path, value):
    Path(path).write_text(json.dumps(value, indent=2, sort_keys=True) + "\n")


def capture(argv, *, cwd=ROOT, env=None):
    """Read-only diagnostic commands, never shell strings."""
    try:
        result = subprocess.run(argv, cwd=cwd, env=env, capture_output=True,
                                text=True, timeout=30, check=False)
        return {"exit_code": result.returncode, "stdout": result.stdout.strip(),
                "stderr": result.stderr.strip()}
    except (OSError, subprocess.TimeoutExpired) as error:
        return {"exit_code": -1, "stdout": "", "stderr": str(error)}


def source_identity(root):
    head = capture(["git", "rev-parse", "HEAD"], cwd=root)
    status = capture(["git", "status", "--porcelain", "--untracked-files=normal"], cwd=root)
    if head["exit_code"] or status["exit_code"]:
        raise QualificationError("Run from a Git checkout of diffz.")
    if status["stdout"]:
        raise QualificationError("Qualification requires a clean checkout, including untracked files.")
    return {"commit": head["stdout"], "clean": True}


def executable_identity(path, env):
    executable = Path(path).resolve()
    if not executable.is_file() or not os.access(executable, os.X_OK):
        raise QualificationError(f"Missing executable: {path}")
    version = capture([str(executable), "--version"], env=env)
    return {"path": str(executable), "sha256": file_digest(executable), "version": version}


def bazel_binary(root):
    expected = (root / ".bazelversion").read_text().strip()
    configured = os.environ.get("BAZEL")
    if configured:
        found = shutil.which(configured)
        candidate = Path(found) if found else Path(configured)
        if not candidate.is_absolute():
            candidate = root / candidate
    else:
        candidate = root / f"target/bazel-bootstrap/bazel-{expected}-linux-x86_64"
    if not candidate.is_file() or not os.access(candidate, os.X_OK):
        raise QualificationError("Bazel is not installed for the trial. Run scripts/bazel-trial.sh --version first, or set BAZEL to the pinned executable.")
    return candidate.resolve(), expected


def collect_identity(args, root=ROOT):
    if (platform.system(), platform.machine()) != ("Linux", "x86_64"):
        raise QualificationError("The current graph is qualified only for Linux x86_64.")
    forbidden = [key for key in FORBIDDEN_ENV if os.environ.get(key) or
                 (key == "BAZEL_RUST_STDLIB_LINKFLAGS" and key in os.environ)]
    if forbidden:
        raise QualificationError("Unset ambient native overrides: " + ", ".join(forbidden))
    source = source_identity(root)
    rc = (root / ".bazelrc").read_text()
    values = {}
    for name, pattern in (
        ("cc", r"^build --repo_env=CC=(.+)$"),
        ("cxx", r"^build --repo_env=CXX=(.+)$"),
        ("action_path", r"^build --action_env=PATH=(.+)$"),
    ):
        matches = re.findall(pattern, rc, re.MULTILINE)
        if len(matches) != 1:
            raise QualificationError(f"Expected one explicit {name} in .bazelrc; update the qualifier for the new graph.")
        values[name] = matches[0]
    env = {**os.environ, "LC_ALL": "C", "PATH": values["action_path"],
           "CC": values["cc"], "CXX": values["cxx"]}
    tools = {}
    for name, requested in (("cc", values["cc"]), ("cxx", values["cxx"]),
                            *((name, name) for name in ("ar", "ld", "pkg-config", "cmake", "make", "perl"))):
        found = shutil.which(requested, path=values["action_path"])
        if not found:
            raise QualificationError(f"Required native tool is absent from the action PATH: {requested}")
        tools[name] = executable_identity(found, env)
    compiler = {}
    for option in ("-dumpmachine", "-dumpfullversion", "-print-sysroot", "-print-search-dirs"):
        compiler[option] = capture([values["cc"], option], env=env)
    libraries = {}
    for name, driver in (("libgcc_s.so.1", values["cc"]), ("libc.so.6", values["cc"]),
                         ("libstdc++.so", values["cxx"])):
        query = capture([driver, "-print-file-name=" + name], env=env)
        resolved = Path(query["stdout"])
        if query["exit_code"] or not resolved.is_file():
            raise QualificationError(f"Cannot identify native runtime library: {name}")
        libraries[name] = {"path": str(resolved.resolve()), "sha256": file_digest(resolved)}
    inventory = {}
    for name, command in (
        ("dpkg", ["dpkg-query", "-W", "-f=${binary:Package}\t${Version}\t${Architecture}\n"]),
        ("rpm", ["rpm", "-qa", "--qf", "%{NAME}\t%{VERSION}-%{RELEASE}\t%{ARCH}\n"]),
        ("pacman", ["pacman", "-Q"]),
        ("brew", ["brew", "list", "--versions"]),
    ):
        if shutil.which(command[0], path=values["action_path"]):
            row = capture(command, env=env)
            if row["exit_code"]:
                raise QualificationError(f"Native package inventory failed: {name}")
            inventory[name] = sorted(row["stdout"].splitlines())
    if not inventory:
        raise QualificationError("No supported native package inventory tool found (dpkg, rpm, pacman, brew).")
    packages = {name: capture(["pkg-config", "--modversion", "--cflags", "--libs", name], env=env)
                for name in NATIVE_PACKAGES}
    linker_paths = re.findall(r"^build:lld --linkopt=-B(.+)$", rc, re.MULTILINE)
    if len(linker_paths) != 1 or "build --config=lld" not in rc or "build:lld --linkopt=-fuse-ld=lld" not in rc:
        raise QualificationError("Expected the pinned Rust-sysroot LLD config; update the qualifier for the new linker.")
    module = (root / "MODULE.bazel").read_text()
    linker_archives = dict(re.findall(r'"((?:rustc|llvm-tools)-[^"\n]+)": "([a-f0-9]{64})"', module))
    if len(linker_archives) != 2:
        raise QualificationError("Expected pinned rustc and LLVM-tools archives for the selected linker.")
    extra_files = {}
    for path in args.native_file:
        path = path.resolve(strict=True)
        if not path.is_file():
            raise QualificationError("--native-file must name a regular file.")
        extra_files[str(path)] = file_digest(path)
    native = {
        "declared_environment_id": args.native_environment_id,
        "system": platform.system(), "machine": platform.machine(),
        "os_release": Path("/etc/os-release").read_text(),
        "configured_environment": values, "tools": tools, "compiler": compiler,
        "runtime_libraries": libraries, "package_inventory": inventory,
        "pkg_config": packages, "extra_files": extra_files,
        "selected_linker": {"flavor": "lld", "search_path": linker_paths[0],
                            "toolchain_archives": linker_archives},
    }
    binary, expected = bazel_binary(root)
    actual = capture([str(binary), "--version"], env=env)
    if actual["exit_code"] or actual["stdout"] != "bazel " + expected:
        raise QualificationError(f"Bazel must match .bazelversion ({expected}).")
    machine_id = Path("/etc/machine-id")
    if not machine_id.is_file() or not machine_id.read_text().strip():
        raise QualificationError("A nonempty /etc/machine-id is required to distinguish observed hosts.")
    return {
        "schema": SCHEMA, "source": source, "native": native,
        "native_sha256": digest(native),
        "host": {"label": args.host_label, "machine_id_sha256": file_digest(machine_id),
                 "kernel": platform.release()},
        "bazel": {"version": expected, "sha256": file_digest(binary)},
        "graph": {name: file_digest(root / name) for name in
                  (".bazelrc", "MODULE.bazel", "MODULE.bazel.lock", "Cargo.lock", "scripts/bazel-trial.sh")},
    }, binary


def validate_endpoint(endpoint):
    parsed = urlsplit(endpoint)
    if parsed.scheme not in ("grpc", "grpcs") or not parsed.hostname:
        raise QualificationError("Use the existing cache's grpc:// or grpcs:// endpoint.")
    if parsed.username or parsed.password or parsed.query or parsed.fragment or parsed.path not in ("", "/"):
        raise QualificationError("The endpoint must contain only a scheme and host/port; use external credentials.")
    return endpoint.rstrip("/")


def validate_identity_pair(producer, consumer):
    for identity in (producer, consumer):
        if identity.get("schema") != SCHEMA or digest(identity["native"]) != identity["native_sha256"]:
            raise QualificationError("Invalid identity evidence or native fingerprint.")
    for key in ("source", "native_sha256", "bazel", "graph"):
        if producer[key] != consumer[key]:
            raise QualificationError(f"Producer/consumer mismatch: {key}. No cross-host qualification.")
    if producer["host"]["machine_id_sha256"] == consumer["host"]["machine_id_sha256"]:
        raise QualificationError("Producer and consumer have the same observed machine ID; use two hosts.")
    if producer["host"]["label"] == consumer["host"]["label"]:
        raise QualificationError("Use distinct, recognizable host labels.")


def read_events(path):
    try:
        return [json.loads(line) for line in Path(path).read_text().splitlines() if line.strip()]
    except (OSError, ValueError) as error:
        raise QualificationError(f"Missing or invalid Bazel event evidence: {path}") from error


def require_successful_finish(events):
    finished = [event["finished"] for event in events if "finished" in event]
    if len(finished) != 1 or "exitCode" not in finished[0] or int(finished[0]["exitCode"].get("code", 0)) != 0:
        raise QualificationError("Build event stream lacks a successful final exit code.")


def build_evidence(events):
    require_successful_finish(events)
    summaries = [event["buildMetrics"]["actionSummary"] for event in events
                 if "actionSummary" in event.get("buildMetrics", {})]
    if len(summaries) != 1:
        raise QualificationError("Build event stream lacks a successful build and action metrics.")
    runners = {row["name"]: int(row.get("count", 0)) for row in summaries[0].get("runnerCount", [])}
    if not runners:
        raise QualificationError("Build event stream has no runner counts; cannot prove remote reuse.")
    return {"runners": runners, "remote_cache_hits": runners.get("remote cache hit", 0),
            "other_execution": {key: value for key, value in runners.items()
                                if key not in ("internal", "total", "remote cache hit") and value}}


def test_evidence(events):
    require_successful_finish(events)
    results = [event["testResult"] for event in events if "testResult" in event]
    summaries = {event["id"]["testSummary"]["label"]: event["testSummary"] for event in events
                 if "testSummary" in event and "testSummary" in event.get("id", {})}
    if not results or not summaries:
        raise QualificationError("No executed test evidence was reported.")
    for row in results:
        if row.get("status") != "PASSED" or row.get("cachedLocally") or row.get("executionInfo", {}).get("cachedRemotely"):
            raise QualificationError("A test failed or reused a cached result.")
    if any(row.get("overallStatus") != "PASSED" or int(row.get("totalNumCached", 0)) for row in summaries.values()):
        raise QualificationError("A test summary failed or contains cached results.")
    return {"labels": sorted(summaries), "executed_results": len(results)}


def binary_from_events(events, output_root):
    candidates = set()
    for event in events:
        for item in event.get("namedSetOfFiles", {}).get("files", []):
            uri = urlsplit(item.get("uri", ""))
            if item.get("name") == "diffz" and uri.scheme == "file" and uri.netloc in ("", "localhost"):
                path = Path(unquote(uri.path)).resolve()
                if path.is_relative_to(output_root.resolve()) and path.is_file():
                    candidates.add(path)
    if len(candidates) != 1:
        raise QualificationError("Expected one materialized diffz binary within this run's output root.")
    return candidates.pop()


def materialized_linker(identity, output_root, env):
    relative = Path(identity["native"]["selected_linker"]["search_path"])
    if relative.is_absolute() or ".." in relative.parts:
        raise QualificationError("Configured linker must be relative to the execution root.")
    candidates = [root / relative / "ld.lld" for root in output_root.glob("*/execroot/*")
                  if (root / relative / "ld.lld").is_file()]
    if len(candidates) != 1:
        raise QualificationError("Cannot identify the materialized LLD selected by .bazelrc.")
    selected = candidates[0]
    rust_lld = selected.parent.parent / "rust-lld"
    if not rust_lld.is_file():
        raise QualificationError("The pinned Rust linker implementation is missing.")
    version = capture([str(selected), "--version"], env=env)
    if version["exit_code"]:
        raise QualificationError("The selected LLD failed --version.")
    return {"ld_lld_sha256": file_digest(selected), "rust_lld_sha256": file_digest(rust_lld),
            "version": version["stdout"]}


@contextmanager
def local_test_lock():
    """Serialize only forced test execution; keep the action-visible path stable."""
    lock = Path(TEST_ROOT + ".lock")
    with lock.open("a") as stream:
        started = time.monotonic()
        fcntl.flock(stream.fileno(), fcntl.LOCK_EX)
        try:
            yield time.monotonic() - started
        finally:
            fcntl.flock(stream.fileno(), fcntl.LOCK_UN)


def make_flags(args, identity, phase, output):
    native_hash = identity["native_sha256"]
    producer_build = args.command == "producer" and phase == "build"
    flags = ["--config=fast-dev", "--disk_cache=", "--remote_executor=",
             "--remote_cache=" + args.cache_endpoint,
             "--remote_instance_name=diffz-qualification/" + native_hash,
             "--remote_cache_compression", "--remote_download_outputs=all",
             "--remote_cache_async=false",
             "--remote_upload_local_results=" + str(producer_build).lower(),
             "--remote_accept_cached=" + str(not producer_build).lower(),
             "--action_env=DIFFZ_BAZEL_NATIVE_ENV_SHA256=" + native_hash,
             "--repo_env=DIFFZ_BAZEL_NATIVE_ENV_SHA256=" + native_hash,
             "--lockfile_mode=error", "--symlink_prefix=/", "--color=no", "--curses=no",
             "--build_event_json_file_path_conversion=false",
             "--build_event_json_file=" + str(output / (phase + ".bep.jsonl")),
             "--profile=" + str(output / (phase + ".profile.json.gz"))]
    if args.repo_contents_cache:
        flags.append("--repo_contents_cache=" + str(args.repo_contents_cache))
    if args.credential_helper:
        flags.append("--credential_helper=" + urlsplit(args.cache_endpoint).hostname + "=" + str(args.credential_helper))
    if phase == "test":
        flags.extend(("--cache_test_results=no", "--test_strategy=local"))
    return flags


def run_logged(name, argv, output, env, root):
    started = time.monotonic()
    with (output / (name + ".log")).open("w") as stream:
        if name in ("version", "inspect"):
            with (output / (name + ".stdout")).open("w") as stdout:
                process = subprocess.run(argv, cwd=root, env=env, stdout=stdout, stderr=stream, check=False)
        else:
            process = subprocess.run(argv, cwd=root, env=env, stdout=stream, stderr=subprocess.STDOUT, check=False)
    row = {"phase": name, "argv": argv, "exit_code": process.returncode,
           "seconds": time.monotonic() - started}
    with (output / "commands.jsonl").open("a") as stream:
        stream.write(json.dumps(row) + "\n")
    if process.returncode:
        raise QualificationError(f"{name} failed; inspect {output / (name + '.log')}")
    return row


def load_complete(path):
    report = json.loads(Path(path).read_text())
    if report.get("schema") != SCHEMA or report.get("status") != "complete":
        raise QualificationError("Expected a completed qualification run report.")
    return report


def compare_runs(producer, consumer):
    if producer.get("mode") != "producer" or consumer.get("mode") != "consumer":
        raise QualificationError("Comparison requires one producer and one consumer.")
    validate_identity_pair(producer["identity"], consumer["identity"])
    for key in ("cache_endpoint", "instance_name", "service_instance_isolation_acknowledged"):
        if not producer.get(key) or producer[key] != consumer.get(key):
            raise QualificationError(f"Cache configuration mismatch: {key}")
    if consumer["build"]["remote_cache_hits"] <= 0 or consumer["build"]["other_execution"]:
        raise QualificationError("Fresh consumer did not restore the full build from the remote cache.")
    for key in ("tests", "runtime", "materialized_linker"):
        if producer[key] != consumer[key]:
            raise QualificationError(f"Producer/consumer {key} differ.")
    return {"schema": SCHEMA, "status": "observed_two_host_cache_reuse",
            "native_sha256": producer["identity"]["native_sha256"],
            "commit": producer["identity"]["source"]["commit"],
            "producer_host": producer["identity"]["host"], "consumer_host": consumer["identity"]["host"],
            "consumer_build": consumer["build"], "tests": consumer["tests"],
            "limitations": ["Observed input coverage and declared environment ID are not a proof of hermeticity.",
                            "Distinct machine IDs require the operator to confirm two actual hosts.",
                            "No RBE, GUI, macOS, doctest or release qualification."]}


def prepare_output(path, root):
    path = path.resolve()
    if path.is_relative_to(root) and not path.is_relative_to(root / "target"):
        raise QualificationError("Keep evidence under target/ or outside the checkout.")
    path.mkdir(parents=True, exist_ok=False)
    return path


def execute(args, root=ROOT):
    output = prepare_output(args.output, root)
    report = {"schema": SCHEMA, "mode": args.command, "status": "failed"}
    env = None
    server_started = False
    comparison = None
    try:
        identity, binary = collect_identity(args, root)
        report["identity"] = identity
        write_json(output / "preflight.json", identity)
        if args.command == "preflight":
            report["status"] = "complete"
            return report
        args.cache_endpoint = validate_endpoint(args.cache_endpoint)
        if not args.confirm_instance_isolation:
            raise QualificationError("Confirm the service isolates instance names; bazel-remote requires --enable_ac_key_instance_mangling. See the qualification guide.")
        if args.credential_helper:
            args.credential_helper = args.credential_helper.resolve(strict=True)
            if not args.credential_helper.is_file() or not os.access(args.credential_helper, os.X_OK):
                raise QualificationError("--credential-helper must be an executable file.")
        if args.repo_contents_cache:
            args.repo_contents_cache = args.repo_contents_cache.resolve()
            if args.repo_contents_cache.is_relative_to(root) or args.repo_contents_cache.is_relative_to(output):
                raise QualificationError("Repository contents cache must be outside this checkout and evidence directory.")
        producer = None
        if args.command == "consumer":
            producer = load_complete(args.producer_report)
            if producer.get("mode") != "producer":
                raise QualificationError("--producer-report must reference a producer run.")
            validate_identity_pair(producer["identity"], identity)
            if producer.get("cache_endpoint") != args.cache_endpoint:
                raise QualificationError("The producer used a different endpoint.")
        report.update(cache_endpoint=args.cache_endpoint,
                      instance_name="diffz-qualification/" + identity["native_sha256"],
                      service_instance_isolation_acknowledged=True)
        output_root = output / "bazel-output"
        env = {**os.environ, "BAZEL": str(binary), "LC_ALL": "C", "CCACHE_DISABLE": "1",
               "CC": identity["native"]["configured_environment"]["cc"],
               "CXX": identity["native"]["configured_environment"]["cxx"],
               "RUSTC_WRAPPER": "", "RUSTC_WORKSPACE_WRAPPER": "",
               "DIFFZ_BAZEL_CACHE": "0", "DIFFZ_BAZEL_REMOTE_CACHE_READ_ONLY": "0",
               "DIFFZ_BAZEL_OUTPUT_ROOT": str(output_root),
               "DIFFZ_BAZEL_TEST_TMP_ROOT": TEST_ROOT}
        env.pop("DIFFZ_BAZEL_REMOTE_CACHE", None)
        env.pop("DIFFZ_BAZEL_REPO_CONTENTS_CACHE", None)
        driver = ["bash", str(root / "scripts/bazel-trial.sh")]
        for phase, target in (("build", "//:all_build"), ("test", "//:tests")):
            server_started = True
            argv = driver + [phase, target] + make_flags(args, identity, phase, output)
            if phase == "test":
                with local_test_lock() as wait_seconds:
                    report.setdefault("timings", {})["test_lock_wait"] = wait_seconds
                    row = run_logged(phase, argv, output, env, root)
            else:
                row = run_logged(phase, argv, output, env, root)
            events = read_events(output / (phase + ".bep.jsonl"))
            report["build" if phase == "build" else "tests"] = build_evidence(events) if phase == "build" else test_evidence(events)
            report.setdefault("timings", {})[phase] = row["seconds"]
            if phase == "build" and args.command == "consumer":
                if report["build"]["remote_cache_hits"] <= 0 or report["build"]["other_execution"]:
                    raise QualificationError("Consumer ran uncached build actions; inspect BEP runner counts. This is not full cache portability.")
        app = binary_from_events(read_events(output / "build.bep.jsonl"), output_root)
        report["materialized_linker"] = materialized_linker(identity, output_root, env)
        runtime = {"binary_sha256": file_digest(app)}
        runtime_env = dict(env)
        for key, child in (("XDG_CONFIG_HOME", "config"), ("XDG_DATA_HOME", "data"),
                           ("XDG_CACHE_HOME", "cache"), ("XDG_STATE_HOME", "xdg-state")):
            path = output / "runtime" / child
            path.mkdir(parents=True, exist_ok=False)
            runtime_env[key] = str(path)
        state = output / "runtime/state"
        for name, flags in (("version", ["--version"]),
                            ("inspect", ["--inspect", "--fixture", "F01", "--state-dir", str(state)])):
            run_logged(name, [str(app)] + flags, output, runtime_env, root)
            text = (output / (name + ".stdout")).read_text().strip()
            runtime[name] = json.loads(text) if name == "inspect" else text
        report["runtime"] = runtime
        if source_identity(root) != identity["source"]:
            raise QualificationError("Source changed while the qualification ran.")
        if producer:
            comparison = compare_runs(producer, report)
        report["status"] = "complete"
        return report
    except (QualificationError, OSError, ValueError, KeyError) as error:
        report["error"] = str(error)
        raise QualificationError(str(error)) from error
    finally:
        if server_started:
            shutdown = capture(["bash", str(root / "scripts/bazel-trial.sh"), "shutdown"], cwd=root, env=env)
            report["shutdown"] = shutdown
            if shutdown["exit_code"]:
                report["status"] = "failed"
                report["cleanup_error"] = "Bazel shutdown failed; inspect result.json and stop only this run's output root."
                report.setdefault("error", report["cleanup_error"])
        if report["status"] == "complete" and comparison is not None:
            write_json(output / "comparison.json", comparison)
        write_json(output / "result.json", report)


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    commands = parser.add_subparsers(dest="command", required=True)
    for mode in ("preflight", "producer", "consumer"):
        command = commands.add_parser(mode)
        command.add_argument("--output", type=Path, required=True, help="New evidence directory; never overwritten or removed")
        command.add_argument("--native-environment-id", required=True, help="Immutable image/native-environment identifier, identical on both hosts")
        command.add_argument("--host-label", default=socket.gethostname())
        command.add_argument("--native-file", type=Path, action="append", default=[], help="Additional native file whose path and bytes must match")
        if mode != "preflight":
            command.add_argument("--cache-endpoint", required=True)
            command.add_argument("--confirm-instance-isolation", action="store_true", help="Acknowledge verified service configuration; this does not probe it")
            command.add_argument("--repo-contents-cache", type=Path)
            command.add_argument("--credential-helper", type=Path, help="Existing executable Bazel credential helper, scoped to the cache hostname")
        if mode == "consumer":
            command.add_argument("--producer-report", type=Path, required=True)
    compare = commands.add_parser("compare")
    compare.add_argument("producer_report", type=Path)
    compare.add_argument("consumer_report", type=Path)
    args = parser.parse_args(argv)
    try:
        if args.command == "compare":
            result = compare_runs(load_complete(args.producer_report), load_complete(args.consumer_report))
        else:
            if not args.native_environment_id.strip():
                raise QualificationError("--native-environment-id cannot be empty.")
            result = execute(args)
            if result["status"] != "complete":
                raise QualificationError(result.get("error", "Qualification failed."))
        if args.command != "compare":
            result = {"mode": result["mode"], "status": result["status"],
                      "evidence": str(args.output.resolve()),
                      "native_sha256": result["identity"]["native_sha256"],
                      "timings": result.get("timings", {})}
        print(json.dumps(result, indent=2, sort_keys=True))
        return 0
    except (QualificationError, OSError, ValueError, KeyError) as error:
        print("Qualification failed: " + str(error), file=sys.stderr)
        return 2


if __name__ == "__main__":
    raise SystemExit(main())
