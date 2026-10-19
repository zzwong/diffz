#!/usr/bin/env python3
"""Compare the experimental Linux Bazel build with Cargo; retain raw evidence."""
import argparse
import json
import os
from pathlib import Path
import shutil
import statistics
import subprocess
import tempfile
import time

ROOT = Path(__file__).resolve().parent.parent
JOBS = 16
PROBES = {
    "app": "crates/app/src/main.rs",
    "core": "crates/core/src/lib.rs",
    "ui": "crates/ui/src/lib.rs",
    "test": "crates/adapters/tests/store_retention.rs",
}
NORMALIZE = [
    "--config", "profile.dev.codegen-units=16",
    "--config", "profile.dev.package.gpui-kit-assets.debug-assertions=false",
    "--config", "profile.dev.package.rust-embed.debug-assertions=false",
    "--config", "profile.dev.overflow-checks=true",
]


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output", type=Path, required=True,
                        help="New evidence directory; existing directories are refused")
    parser.add_argument("--iterations", type=int, default=3)
    parser.add_argument("--bazel-config", action="append", default=[],
                        help="Bazel build/test config to compare; repeat to combine configs")
    parser.add_argument("--bazel-driver", type=Path, default=Path("scripts/bazel-trial.sh"),
                        help="Trial driver; use scripts/bazel-worker-trial.sh for the worker experiment")
    parser.add_argument("--disk-cache-trial", action="store_true",
                        help="Also time repeated visits to a known core edit with a local action cache")
    args = parser.parse_args()
    if args.iterations < 1:
        parser.error("--iterations must be positive")
    evidence = args.output.resolve()
    evidence.mkdir(parents=True, exist_ok=False)
    rows = []
    env = {**os.environ, "RUSTC_WRAPPER": "", "RUSTC_WORKSPACE_WRAPPER": "",
           "CC": "/usr/bin/gcc", "CXX": "/usr/bin/g++"}
    bazel = ["bash", str(args.bazel_driver)]

    def run(name, command, check=True):
        if command[:len(bazel)] == bazel and command[len(bazel)] in ("build", "test"):
            command = (command[:len(bazel) + 1]
                       + ["--config=" + config for config in args.bazel_config]
                       + command[len(bazel) + 1:])
        start = time.monotonic()
        with (evidence / (name + ".log")).open("w") as log:
            log.write("$ " + repr(command) + "\n")
            log.flush()
            result = subprocess.run(command, cwd=ROOT, env=env, stdout=log,
                                    stderr=subprocess.STDOUT)
        row = dict(name=name, command=command,
                   seconds=round(time.monotonic() - start, 4), exit_code=result.returncode)
        rows.append(row)
        with (evidence / "measurements.jsonl").open("a") as log:
            log.write(json.dumps(row) + "\n")
        print(json.dumps(row), flush=True)
        if check and result.returncode:
            raise RuntimeError(f"{name} failed; see {evidence / (name + '.log')}")

    def cargo(profile):
        normalized = profile == "cargo"
        env["CARGO_INCREMENTAL"] = "0" if normalized else "1"
        env["CARGO_TARGET_DIR"] = str(ROOT / "target" / (
            "cargo-bazel-trial" if normalized else "cargo-bazel-default"))
        base = ["cargo"] + (NORMALIZE if normalized else [])
        build = base + ["build", "--locked", "--workspace", "--all-features",
                        "--all-targets", "-j", str(JOBS)]
        tests = base + ["test", "--locked", "--workspace", "--all-features",
                        "--lib", "--bins", "--tests", "-j", str(JOBS)]
        return build, tests

    def edits(label, tools):
        source = ROOT / PROBES[label]
        original = source.read_bytes()
        probe = original
        try:
            for i in range(1, args.iterations + 1):
                order = tools if i % 2 else list(reversed(tools))
                for tool, command in order:
                    if source.read_bytes() != original:
                        raise RuntimeError(f"Concurrent edit detected in {source}")
                    probe = original + f"\n// Bazel/Cargo {label} edit comparison {i}.\n".encode()
                    source.write_bytes(probe)
                    run(f"{tool}-{label}-edit-{i}", command)
                    if source.read_bytes() != probe:
                        raise RuntimeError(f"Concurrent edit detected in {source}")
                    source.write_bytes(original)
                    run(f"{tool}-{label}-undo-{i}", command)
        finally:
            if source.read_bytes() == probe:
                source.write_bytes(original)

    def save_timings(profile):
        source = Path(env["CARGO_TARGET_DIR"]) / "cargo-timings/cargo-timing.html"
        if source.exists():
            shutil.copy(source, evidence / (profile + "-cold-timing.html"))

    with tempfile.TemporaryDirectory(prefix="diffz-bazel-bench-", dir=os.environ.get("TMPDIR", "/var/tmp")) as scratch:
        env["TMPDIR"] = scratch
        try:
            build, tests = cargo("cargo")
            run("cargo-fetch", ["cargo", "fetch", "--locked"])
            run("bazel-fetch", bazel + ["fetch", "//:all_build"])
            # Only the benchmark's own Cargo artifact directory is cleaned.
            run("cargo-clean", ["cargo", "clean", "--target-dir", env["CARGO_TARGET_DIR"]])
            run("cargo-cold-build", build + ["-vv", "--timings"])
            save_timings("cargo")
            run("cargo-tests-initial", tests)
            run("bazel-clean", bazel + ["clean"])
            run("bazel-cold-build", bazel + ["build", "//:all_build",
                "--profile=" + str(evidence / "bazel-cold-profile.json.gz")])
            run("bazel-tests-initial", bazel + ["test", "//:tests", "--cache_test_results=no"])
            for i in range(1, args.iterations + 1):
                for suffix, a, b in [
                    ("build", build, bazel + ["build", "//:all_build"]),
                    ("tests", tests, bazel + ["test", "//:tests", "--cache_test_results=no"]),
                ]:
                    tools = [("cargo", a), ("bazel", b)]
                    for label, command in tools if i % 2 else reversed(tools):
                        run(f"{label}-warm-{suffix}-{i}", command)
            run("bazel-cached-test-seed", bazel + ["test", "//:tests"])
            for i in range(1, args.iterations + 1):
                run(f"bazel-cached-tests-{i}", bazel + ["test", "//:tests"])
            for label in PROBES:
                edits(label, [("cargo", build), ("bazel", bazel + ["build", "//:all_build"])])
            run("cargo-tests-final", tests)
            run("bazel-tests-final", bazel + ["test", "//:tests", "--cache_test_results=no"])
            build, tests = cargo("cargo-default")
            run("cargo-default-clean", ["cargo", "clean", "--target-dir", env["CARGO_TARGET_DIR"]])
            run("cargo-default-cold-build", build + ["-vv", "--timings"])
            save_timings("cargo-default")
            for i in range(1, args.iterations + 1):
                run(f"cargo-default-warm-build-{i}", build)
                run(f"cargo-default-warm-tests-{i}", tests)
            for label in PROBES:
                edits(label, [("cargo-default", build)])
            run("cargo-default-tests-final", tests)
            # Real value changes complement the comment-based invalidation probes.
            for label, relative, old, new in [
                ("core-limit", "crates/core/src/patch.rs", b"max_files: 10_000,", b"max_files: 10_001,"),
                ("ui-scale", "crates/ui/src/magnify.rs", b"MAX_SCALE: f32 = 5.0;", b"MAX_SCALE: f32 = 5.25;"),
            ]:
                source = ROOT / relative
                original = source.read_bytes()
                if original.count(old) != 1:
                    raise RuntimeError(f"Semantic probe does not match {source}")
                probe = original.replace(old, new)
                try:
                    for tool in ["cargo-default", "cargo", "bazel"]:
                        if tool == "bazel":
                            command = bazel + ["build", "//:all_build"]
                        else:
                            command, _ = cargo(tool)
                        source.write_bytes(probe)
                        run(f"semantic-{tool}-{label}-edit", command)
                        source.write_bytes(original)
                        run(f"semantic-{tool}-{label}-undo", command)
                finally:
                    if source.read_bytes() == probe:
                        source.write_bytes(original)
            build, tests = cargo("cargo-default")
            run("cycle-cargo-seed", tests)
            run("cycle-bazel-seed", bazel + ["test", "//:tests"])
            for label, relative, old, template, count in [
                ("app-version", "crates/app/src/main.rs", b'"diffz {}"', '"diffz trial-%d {}"', 2),
                ("test-assertion", "crates/adapters/tests/store_retention.rs",
                 b"assert_eq!(store.prune(2).unwrap(), 2);",
                 'assert_eq!(store.prune(2).unwrap(), 2, "trial cycle %d");', 1),
            ]:
                source = ROOT / relative
                original = source.read_bytes()
                if original.count(old) != count:
                    raise RuntimeError(f"Test-cycle probe does not match {source}")
                probe = original
                try:
                    for i in range(1, args.iterations + 1):
                        order = [("cargo", tests, build), ("bazel", bazel + ["test", "//:tests"], bazel + ["build", "//:all_build"])]
                        for tool, command, undo in order if i % 2 else reversed(order):
                            if source.read_bytes() != original:
                                raise RuntimeError(f"Concurrent edit detected in {source}")
                            probe = original.replace(old, (template % i).encode())
                            source.write_bytes(probe)
                            run(f"cycle-{tool}-{label}-{i}", command)
                            if source.read_bytes() != probe:
                                raise RuntimeError(f"Concurrent edit detected in {source}")
                            source.write_bytes(original)
                            run(f"cycle-{tool}-{label}-restore-{i}", undo)
                finally:
                    if source.read_bytes() == probe:
                        source.write_bytes(original)
            run("cycle-cargo-final", tests)
            run("cycle-bazel-final", bazel + ["test", "//:tests", "--cache_test_results=no"])
            if args.disk_cache_trial:
                command = bazel + ["build", "//:all_build", "--disk_cache=" + str(ROOT / "target/bazel-action-cache")]
                source = ROOT / PROBES["core"]
                original = source.read_bytes()
                probe = original + b"\n// Known core branch for the local action-cache trial.\n"
                try:
                    source.write_bytes(probe)
                    run("disk-cache-seed-edit", command)
                    source.write_bytes(original)
                    run("disk-cache-seed-undo", command)
                    for i in range(1, args.iterations + 1):
                        source.write_bytes(probe)
                        run(f"disk-cache-known-core-edit-{i}", command)
                        source.write_bytes(original)
                        run(f"disk-cache-core-undo-{i}", command)
                finally:
                    if source.read_bytes() == probe:
                        source.write_bytes(original)
                run("disk-cache-tests-final", bazel + ["test", "//:tests", "--cache_test_results=no", "--disk_cache=" + str(ROOT / "target/bazel-action-cache")])
            source = ROOT / "target/bazel-testlogs"
            if source.exists():
                for log in source.glob("*/test.log"):
                    dest = evidence / "bazel-testlogs" / log.parent.name
                    dest.mkdir(parents=True, exist_ok=True)
                    shutil.copy(log, dest / "test.log")
        finally:
            run("bazel-shutdown", bazel + ["shutdown"], check=False)
    groups = {}
    for row in rows:
        name = row["name"].rsplit("-", 1)[0] if row["name"].rsplit("-", 1)[-1].isdigit() else row["name"]
        groups.setdefault(name, []).append(row)
    summary = {name: {"median_seconds": statistics.median(r["seconds"] for r in values),
                      "all_passed": all(r["exit_code"] == 0 for r in values)}
               for name, values in groups.items()}
    (evidence / "summary.json").write_text(json.dumps(summary, indent=2) + "\n")


if __name__ == "__main__":
    main()
