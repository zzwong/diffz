#!/usr/bin/env python3
"""Measure artifact-cold Cargo/Bazel builds without touching ordinary build output."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import shutil
import subprocess
import tempfile
import time

ROOT = Path(__file__).resolve().parent.parent


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--iterations", type=int, default=3)
    parser.add_argument("--jobs", type=int, default=16)
    parser.add_argument("--tool", choices=("cargo", "bazel", "both"), default="both")
    parser.add_argument("--bazel-config", action="append", default=[])
    parser.add_argument("--bazel-arg", action="append", default=[])
    parser.add_argument("--cargo-arg", action="append", default=[])
    parser.add_argument("--cargo-profile", default="dev")
    parser.add_argument("--repository-cache", type=Path,
                        help="Optional shared Bazel download cache; compiled action caches stay disabled")
    args = parser.parse_args()
    if args.iterations < 1 or args.jobs < 1:
        parser.error("--iterations and --jobs must be positive")
    evidence = args.output.resolve()
    evidence.mkdir(parents=True, exist_ok=False)
    # Every invocation owns fresh build directories. Downloads and JVM startup
    # are timed separately; clean removes artifacts before each measured build.
    (ROOT / "target").mkdir(exist_ok=True)
    artifacts = Path(tempfile.mkdtemp(prefix="cold-build-", dir=ROOT / "target"))
    tools = [args.tool] if args.tool != "both" else ["cargo", "bazel"]
    bazel = ["bash", "scripts/bazel-trial.sh"]
    repository_flags = (["--repository_cache=" + str(args.repository_cache.resolve())]
                        if args.repository_cache else [])
    flags = ([f"--config={x}" for x in args.bazel_config]
             + [f"--jobs={args.jobs}", f"--local_resources=cpu={args.jobs}"]
             + args.bazel_arg + repository_flags + ["--disk_cache=", "--remote_cache="])
    cargo = ["cargo"] + args.cargo_arg
    cargo_profile = [] if args.cargo_profile == "dev" else ["--profile=" + args.cargo_profile]
    source_hashes = {str(p.relative_to(ROOT)): hashlib.sha256(p.read_bytes()).hexdigest()
                     for p in [ROOT / "Cargo.toml", ROOT / "Cargo.lock", ROOT / "MODULE.bazel",
                               ROOT / ".bazelrc", ROOT / "rust-toolchain.toml", ROOT / ".cargo/config.toml"]
                     if p.exists()}
    metadata = dict(jobs=args.jobs, iterations=args.iterations, tools=tools, cargo_profile=args.cargo_profile,
                    artifacts=str(artifacts), source_hashes=source_hashes,
                    native_compilers={"CC": "/usr/bin/gcc", "CXX": "/usr/bin/g++", "CCACHE_DISABLE": "1"},
                    compiler_environment={k: v for k, v in os.environ.items()
                                          if k.startswith(("RUSTFLAGS", "CARGO_ENCODED_RUSTFLAGS", "CARGO_PROFILE_", "RUSTC_BOOTSTRAP"))},
                    definition="Artifact-cold; dependencies fetched, Bazel JVM warm; compiler wrappers disabled")
    (evidence / "setup.json").write_text(json.dumps(metadata, indent=2) + "\n")
    with tempfile.TemporaryDirectory(prefix="diffz-cold-", dir=os.environ.get("TMPDIR", "/var/tmp")) as scratch:
        env = {**os.environ, "TMPDIR": scratch, "RUSTC_WRAPPER": "",
               "RUSTC_WORKSPACE_WRAPPER": "", "CARGO_INCREMENTAL": "1",
               "CARGO_TARGET_DIR": str(artifacts / "cargo"),
               "DIFFZ_BAZEL_OUTPUT_ROOT": str(artifacts / "bazel"),
               "CC": "/usr/bin/gcc", "CXX": "/usr/bin/g++", "CCACHE_DISABLE": "1"}

        def run(name, command, check=True):
            start = time.monotonic()
            with (evidence / (name + ".log")).open("w") as log:
                log.write(repr(command) + "\n")
                log.flush()
                result = subprocess.run(command, cwd=ROOT, env=env, stdout=log, stderr=subprocess.STDOUT)
            row = dict(name=name, command=command, seconds=round(time.monotonic() - start, 4),
                       exit_code=result.returncode)
            with (evidence / "measurements.jsonl").open("a") as log:
                log.write(json.dumps(row) + "\n")
            print(json.dumps(row), flush=True)
            if check and result.returncode:
                raise RuntimeError(f"{name} failed; see {evidence / (name + '.log')}")

        try:
            run("rustc-version", ["rustc", "-Vv"])
            run("cargo-version", ["cargo", "-V"])
            run("git-head", ["git", "rev-parse", "HEAD"])
            run("git-state", ["git", "status", "--short"])
            if "cargo" in tools:
                run("cargo-fetch", cargo + ["fetch", "--locked"])
            if "bazel" in tools:
                run("bazel-version", bazel + ["--version"])
                run("bazel-fetch", bazel + ["fetch", "//:all_build"] + repository_flags)
                run("bazel-analysis", bazel + ["build", "//:all_build", "--nobuild"] + flags)
            for i in range(1, args.iterations + 1):
                for tool in tools if i % 2 else list(reversed(tools)):
                    if tool == "cargo":
                        run(f"cargo-clean-{i}", cargo + ["clean", "--target-dir", env["CARGO_TARGET_DIR"]])
                        run(f"cargo-cold-{i}", cargo + ["build", "--locked", "--workspace", "--all-features",
                            "--all-targets", "--timings", f"-j{args.jobs}"] + cargo_profile)
                        shutil.copyfile(artifacts / "cargo/cargo-timings/cargo-timing.html",
                                        evidence / f"cargo-timing-{i}.html")
                    else:
                        run(f"bazel-clean-{i}", bazel + ["clean"])
                        run(f"bazel-cold-{i}", bazel + ["build", "//:all_build",
                            "--profile=" + str(evidence / f"bazel-profile-{i}.json.gz")] + flags)
            if "cargo" in tools:
                run("cargo-tests", cargo + ["test", "--locked", "--workspace", "--all-features",
                    "--lib", "--bins", "--tests", f"-j{args.jobs}"] + cargo_profile)
            if "bazel" in tools:
                run("bazel-tests", bazel + ["test", "//:tests", "--cache_test_results=no"] + flags)
        finally:
            if "bazel" in tools:
                run("bazel-shutdown", bazel + ["shutdown"], check=False)
    print(f"Evidence: {evidence}; retained build/download artifacts: {artifacts}")


if __name__ == "__main__":
    main()
