#!/usr/bin/env python3
"""Measure isolated cold builds and optional Cargo desktop/reuse scenarios."""
import argparse
from contextlib import contextmanager
from datetime import date
import hashlib
import json
import os
from pathlib import Path
import platform
import re
import shutil
import subprocess
import sys
import tempfile
import time

ROOT = Path(__file__).resolve().parent.parent
SCHEMA_VERSION = 1
CHECKSUM_NIGHTLY = date(2026, 9, 2)
ERROR_SOURCE = "crates/app/src/main.rs"
ERROR_MARKER = "diffz-benchmark-deliberate-compile-error"
ERROR_EDIT = ('\ncompile_error!("' + ERROR_MARKER + '");\n').encode()
PROBES = (
    ("core", "crates/core/src/patch.rs", b"max_files: 10_000,", 1),
    ("ui", "crates/ui/src/magnify.rs", b"MAX_SCALE: f32 = 5.0;", 1),
    ("app", "crates/app/src/main.rs", b'"diffz {}"', 2),
)


def parse_args(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output", type=Path)
    parser.add_argument("--preflight", action="store_true",
                        help="Check tools and controls, without downloading or building")
    parser.add_argument("--iterations", type=int, default=3)
    parser.add_argument("--jobs", type=int, default=16)
    parser.add_argument("--tool", choices=("cargo", "bazel", "both"), default="both")
    parser.add_argument("--bazel-config", action="append", default=[])
    parser.add_argument("--bazel-arg", action="append", default=[])
    parser.add_argument("--cargo-arg", action="append", default=[])
    parser.add_argument("--cargo-profile", default="dev")
    parser.add_argument("--cargo-compare-profile",
                        help="Alternate this profile with --cargo-profile using isolated outputs")
    parser.add_argument("--cargo-incremental", choices=("on", "off"), default="on",
                        help="Explicit Cargo incremental setting for every variant (default: on)")
    parser.add_argument("--cargo-workload", choices=("workspace", "desktop"), default="workspace",
                        help="workspace: all features/targets; desktop: -p diffz --bin diffz")
    parser.add_argument("--cargo-toolchain", help="An already installed rustup toolchain")
    parser.add_argument("--cargo-fingerprint", choices=("default", "mtime", "content", "both"),
                        default="default", help="Explicit modes require one dated nightly >= 2026-09-02")
    parser.add_argument("--cargo-scenario", action="append", default=[],
                        choices=("noop", "edit", "fresh-worktree", "error-repair"),
                        help="Additional Cargo phases after each artifact-cold build; repeatable")
    parser.add_argument("--cc", default="/usr/bin/clang" if platform.system() == "Darwin" else "/usr/bin/gcc")
    parser.add_argument("--cxx", default="/usr/bin/clang++" if platform.system() == "Darwin" else "/usr/bin/g++")
    parser.add_argument("--repository-cache", type=Path,
                        help="Optional shared Bazel download cache; compiled action caches stay disabled")
    args = parser.parse_args(argv)
    if args.iterations < 1 or args.jobs < 1:
        parser.error("--iterations and --jobs must be positive")
    if not args.output and not args.preflight:
        parser.error("--output is required unless --preflight is selected")
    if (args.cargo_workload == "desktop" or args.cargo_scenario or args.cargo_fingerprint != "default") and args.tool != "cargo":
        parser.error("desktop, reuse scenarios and fingerprint trials require --tool cargo")
    if args.cargo_compare_profile is not None:
        if args.tool != "cargo" or args.cargo_fingerprint != "default":
            parser.error("profile comparisons require --tool cargo and --cargo-fingerprint default; measure fingerprint changes separately")
        if not args.cargo_profile.strip() or not args.cargo_compare_profile.strip():
            parser.error("profile comparisons require two nonempty profile names")
        if args.cargo_profile == args.cargo_compare_profile:
            parser.error("profile comparisons require two different profiles")
    if args.cargo_fingerprint != "default":
        match = re.fullmatch(r"nightly-(\d{4}-\d{2}-\d{2})", args.cargo_toolchain or "")
        try:
            nightly = date.fromisoformat(match[1]) if match else None
        except ValueError:
            nightly = None
        if nightly is None or nightly < CHECKSUM_NIGHTLY:
            parser.error("fingerprint trials require --cargo-toolchain nightly-YYYY-MM-DD >= 2026-09-02; use the same nightly for both modes")
    # Arbitrary global config can replace rustc, the fingerprint algorithm, or
    # owned output paths. Keep it for legacy screens, not controlled reuse trials.
    if (args.cargo_fingerprint != "default" or args.cargo_scenario or args.cargo_compare_profile is not None) and args.cargo_arg:
        parser.error("--cargo-arg is not supported in fingerprint/reuse/profile trials; select a tracked --cargo-profile")
    args.cargo_scenario = list(dict.fromkeys(args.cargo_scenario))
    return args


def tool_command(args, tool):
    return (["rustup", "run", args.cargo_toolchain, tool] if args.cargo_toolchain else [tool])


def command_output(command):
    result = subprocess.run(command, cwd=ROOT, text=True, stdout=subprocess.PIPE, stderr=subprocess.STDOUT)
    if result.returncode:
        raise RuntimeError(f"Preflight failed: {command!r}\n{result.stdout.strip()}")
    return result.stdout.strip()


def preflight(args):
    needed = ["git", args.cc, args.cxx]
    if args.tool != "bazel":
        needed += ["rustup"] if args.cargo_toolchain else ["cargo", "rustc"]
    if args.tool != "cargo":
        needed += ["bash"]
        if os.environ.get("BAZEL"):
            needed += [os.environ["BAZEL"]]
        elif platform.system() != "Linux" or platform.machine() != "x86_64":
            raise RuntimeError("The Bazel trial requires x86_64 Linux unless BAZEL names a supported executable; use --tool cargo for native macOS")
        elif not (ROOT / "target/bazel-bootstrap/bazel-8.4.2-linux-x86_64").is_file():
            needed += ["curl", "sha256sum"]
    missing = [tool for tool in needed if not shutil.which(tool)]
    if missing:
        raise RuntimeError("Missing required executable(s): " + ", ".join(missing) + ". Install them before running this trial.")
    if args.tool != "cargo" and not (ROOT / "scripts/bazel-trial.sh").is_file():
        raise RuntimeError("Missing scripts/bazel-trial.sh; run from a complete diffz checkout")
    revision = command_output(["git", "rev-parse", "HEAD"])
    state = command_output(["git", "status", "--porcelain", "--untracked-files=normal"])
    snapshot = bool(args.cargo_scenario) or args.cargo_fingerprint != "default" or args.cargo_compare_profile is not None
    if snapshot and state:
        raise RuntimeError("Fingerprint/reuse/profile trials require a clean, committed checkout so every Git worktree has identical inputs")
    inherited = inherited_cargo_config_hashes(ROOT)
    scratch_parent = Path(os.environ.get("TMPDIR", "/var/tmp")).resolve()
    if snapshot and inherited_cargo_config_hashes(scratch_parent / "diffz-preflight" / "seed") != inherited:
        raise RuntimeError("TMPDIR and the checkout inherit different Cargo configuration; move shared config to CARGO_HOME before creating source worktrees")
    checks = {}
    rustc_path = None
    if args.tool != "bazel":
        if args.cargo_toolchain:
            # rustup run and which fail if absent; neither installs a toolchain.
            rustc_path = command_output(["rustup", "which", "--toolchain", args.cargo_toolchain, "rustc"])
        elif shutil.which("rustup"):
            # Avoid a proxy's implicit download when the repository toolchain
            # is missing, including during --preflight.
            rustc_path = command_output(["rustup", "which", "rustc"])
            command_output(["rustup", "which", "cargo"])
        for tool in ("rustc", "cargo"):
            command = tool_command(args, tool) + (["-Vv"] if tool == "rustc" else ["-V"])
            checks[tool] = dict(command=command, output=command_output(command))
        if args.cargo_fingerprint != "default":
            if any(os.environ.get(key) for key in ("RUSTC", "RUSTC_BOOTSTRAP")):
                raise RuntimeError("Unset RUSTC and RUSTC_BOOTSTRAP for a matched nightly fingerprint trial")
            if any("-nightly" not in checks[tool]["output"] for tool in ("rustc", "cargo")):
                raise RuntimeError("Both Cargo and rustc must report nightly for the selected fingerprint toolchain")
    for tool, path in (("cc", args.cc), ("cxx", args.cxx)):
        command = [path, "--version"]
        checks[tool] = dict(command=command, output=command_output(command))
    if "edit" in args.cargo_scenario:
        for _, relative, old, count in PROBES:
            if (ROOT / relative).read_bytes().count(old) != count:
                raise RuntimeError(f"Semantic probe no longer matches {relative}; update the probe before benchmarking")
    return dict(revision=revision, git_state=state, source_snapshot=snapshot,
                rustc_path=rustc_path, tool_versions=checks,
                inherited_cargo_config_hashes=inherited)


def source_hashes(root):
    paths = [root / name for name in ("Cargo.toml", "Cargo.lock", "MODULE.bazel", ".bazelrc",
                                      "rust-toolchain.toml", ".cargo/config.toml", ".cargo/config")]
    return {str(path.relative_to(root)): hashlib.sha256(path.read_bytes()).hexdigest()
            for path in paths if path.is_file()}


def inherited_cargo_config_hashes(root):
    cargo_home = Path(os.environ.get("CARGO_HOME", Path.home() / ".cargo"))
    if not cargo_home.is_absolute():
        cargo_home = ROOT / cargo_home
    directories = [parent / ".cargo" for parent in root.parents] + [cargo_home]
    return {str(path.resolve()): hashlib.sha256(path.read_bytes()).hexdigest()
            for directory in directories for path in (directory / "config", directory / "config.toml")
            if path.is_file()}


@contextmanager
def compile_error(root):
    """Inject only into the driver's disposable worktree; restore exact bytes."""
    source = root / ERROR_SOURCE
    if root.resolve() == ROOT.resolve() or source.is_symlink() or not source.resolve().is_relative_to(root.resolve()):
        raise RuntimeError("Compile-error probe requires an owned source worktree")
    original = source.read_bytes()
    if ERROR_MARKER.encode() in original:
        raise RuntimeError("Compile-error marker already exists in source")
    changed = original + ERROR_EDIT
    if source.read_bytes() != original:
        raise RuntimeError(f"Source changed concurrently; refusing to overwrite {source}")
    source.write_bytes(changed)
    try:
        yield dict(path=ERROR_SOURCE, appended=ERROR_EDIT.decode(),
                   original_sha256=hashlib.sha256(original).hexdigest(),
                   changed_sha256=hashlib.sha256(changed).hexdigest())
    finally:
        if source.read_bytes() != changed:
            raise RuntimeError(f"Source changed concurrently; refusing to overwrite {source}")
        source.write_bytes(original)


@contextmanager
def semantic_edit(root, probe, iteration):
    kind, relative, old, count = probe
    source = root / relative
    original = source.read_bytes()
    if original.count(old) != count:
        raise RuntimeError(f"Semantic probe no longer matches {source}")
    new = {"core": f"max_files: {41719 + iteration:_},".encode(),
           "ui": f"MAX_SCALE: f32 = {5.375 + iteration / 32:.5f};".encode(),
           "app": f'"diffz trial-{iteration} {{}}"'.encode()}[kind]
    changed = original.replace(old, new)
    if kind == "ui":
        changed = changed.replace(b"assert_eq!(window.magnification().scale, 4.0);",
                                  b"assert_eq!(window.magnification().scale, MAX_SCALE * 0.8);")
    if source.read_bytes() != original:
        raise RuntimeError(f"Source changed concurrently; refusing to overwrite {source}")
    source.write_bytes(changed)
    try:
        yield
    finally:
        if source.read_bytes() != changed:
            raise RuntimeError(f"Source changed concurrently; refusing to overwrite {source}")
        source.write_bytes(original)


def main(argv=None):
    args = parse_args(argv)
    try:
        controls = preflight(args)
    except (OSError, RuntimeError) as error:
        raise SystemExit(str(error)) from error
    if args.preflight:
        print(json.dumps(dict(schema_version=SCHEMA_VERSION, status="preflight-ok", **controls), indent=2))
        return
    evidence = args.output.resolve()
    evidence.mkdir(parents=True, exist_ok=False)
    # Only these newly allocated output directories can ever be cleaned.
    (ROOT / "target").mkdir(exist_ok=True)
    artifacts = Path(tempfile.mkdtemp(prefix="cold-build-", dir=ROOT / "target"))
    tools = [args.tool] if args.tool != "both" else ["cargo", "bazel"]
    modes = ["mtime", "content"] if args.cargo_fingerprint == "both" else [args.cargo_fingerprint]
    profiles = [args.cargo_profile]
    if args.cargo_compare_profile is not None:
        profiles.append(args.cargo_compare_profile)
    # Numeric labels keep user-supplied profile names out of filesystem paths.
    # Profile and fingerprint comparisons are separate experiments.
    variants = [(profile, mode, f"cargo-profile-{index + 1}" if len(profiles) > 1
                 else "cargo" if mode == "default" else "cargo-" + mode)
                for index, profile in enumerate(profiles) for mode in modes]
    bazel = ["bash", "scripts/bazel-trial.sh"]
    repository_flags = (["--repository_cache=" + str(args.repository_cache.resolve())]
                        if args.repository_cache else [])
    flags = ([f"--config={x}" for x in args.bazel_config]
             + [f"--jobs={args.jobs}", f"--local_resources=cpu={args.jobs}"]
             + args.bazel_arg + repository_flags + ["--disk_cache=", "--remote_cache="])
    cargo_base = tool_command(args, "cargo") + args.cargo_arg
    workload = (["-p", "diffz", "--bin", "diffz"] if args.cargo_workload == "desktop"
                else ["--workspace", "--all-features", "--all-targets"])
    # Source worktrees live outside the checkout so ancestor Cargo config cannot
    # be discovered a second time. Git creates their normal fresh file mtimes.
    scratch = Path(tempfile.mkdtemp(prefix="diffz-cold-", dir=os.environ.get("TMPDIR", "/var/tmp"))).resolve()
    (scratch / "tmp").mkdir()
    base_env = {**os.environ, "TMPDIR": str(scratch / "tmp"), "RUSTC_WRAPPER": "",
                "RUSTC_WORKSPACE_WRAPPER": "", "CARGO_INCREMENTAL": "1" if args.cargo_incremental == "on" else "0",
                "CARGO_TARGET_DIR": str(artifacts / "cargo"),
                "CARGO_BUILD_BUILD_DIR": str(artifacts / "cargo"),
                "DIFFZ_BAZEL_OUTPUT_ROOT": str(artifacts / "bazel"),
                "CC": args.cc, "CXX": args.cxx, "CCACHE_DISABLE": "1"}
    if "CARGO_HOME" in base_env:
        base_env["CARGO_HOME"] = str((ROOT / base_env["CARGO_HOME"]).resolve())
    if controls["rustc_path"]:
        base_env["RUSTC"] = controls["rustc_path"]
    metadata = dict(schema_version=SCHEMA_VERSION, jobs=args.jobs, iterations=args.iterations, tools=tools, cargo_profile=args.cargo_profile,
                    cargo_profiles=profiles, cargo_incremental=base_env["CARGO_INCREMENTAL"],
                    cargo_variants=[dict(profile=profile, fingerprint=mode, label=label)
                                    for profile, mode, label in variants],
                    cargo_workload=args.cargo_workload, cargo_toolchain=args.cargo_toolchain,
                    cargo_fingerprints=modes, cargo_scenarios=args.cargo_scenario,
                    artifacts=str(artifacts), source_scratch=str(scratch), source_hashes=source_hashes(ROOT),
                    native_compilers={key: base_env[key] for key in ("CC", "CXX", "CCACHE_DISABLE")},
                    compiler_environment={key: value for key, value in os.environ.items()
                                          if key in ("RUSTFLAGS", "CARGO_ENCODED_RUSTFLAGS", "CARGO_BUILD_TARGET",
                                                     "CFLAGS", "CXXFLAGS", "CPPFLAGS", "LDFLAGS", "RUSTC_BOOTSTRAP",
                                                     "CARGO_BUILD_RUSTFLAGS", "CARGO_BUILD_FINGERPRINT",
                                                     "CARGO_UNSTABLE_CHECKSUM_FRESHNESS")
                                          or key.startswith("CARGO_PROFILE_")
                                          or (key.startswith("CARGO_TARGET_") and key.endswith(("_RUSTFLAGS", "_LINKER")))},
                    host=dict(system=platform.system(), release=platform.release(), machine=platform.machine(),
                              logical_cpus=os.cpu_count()), **controls,
                    definition="Artifact-cold; dependencies fetched, Bazel JVM warm; wrappers disabled. Reuse builds and artifact copies are separate phases.")
    (evidence / "setup.json").write_text(json.dumps(metadata, indent=2) + "\n")

    def record(row):
        row["schema_version"] = SCHEMA_VERSION
        with (evidence / "measurements.jsonl").open("a") as log:
            log.write(json.dumps(row) + "\n")
        print(json.dumps(row), flush=True)

    def run(name, command, *, cwd=ROOT, env=base_env, phase="setup", check=True,
            cargo_profile=None, scenario=None, iteration=None):
        start = time.monotonic()
        with (evidence / (name + ".log")).open("w") as log:
            log.write(f"cwd={cwd}\n{command!r}\n")
            log.flush()
            try:
                result = subprocess.run(command, cwd=cwd, env=env, stdout=log, stderr=subprocess.STDOUT)
                code = result.returncode
            except OSError as error:
                log.write(str(error) + "\n")
                code = 127
        row = dict(name=name, phase=phase, command=command, cwd=str(cwd),
                   target_dir=env.get("CARGO_TARGET_DIR"), build_dir=env.get("CARGO_BUILD_BUILD_DIR"),
                   fingerprint=env.get("CARGO_BUILD_FINGERPRINT", "default"),
                   cargo_profile=cargo_profile,
                   cargo_incremental=env["CARGO_INCREMENTAL"] if cargo_profile is not None else None,
                   scenario=scenario, iteration=iteration,
                   seconds=round(time.monotonic() - start, 4), exit_code=code)
        record(row)
        if check and code:
            raise RuntimeError(f"{name} failed; see {evidence / (name + '.log')}")
        return row

    def cargo_env(mode, target):
        env = {**base_env, "CARGO_TARGET_DIR": str(target), "CARGO_BUILD_BUILD_DIR": str(target)}
        if mode != "default":
            env["CARGO_BUILD_FINGERPRINT"] = mode
        return env

    def cargo_command(mode, target):
        # Explicit configuration wins over inherited Cargo config and over
        # legacy --cargo-arg values. Cleaning cannot reach ordinary output.
        owned = ["--config", "build.build-dir=" + json.dumps(str(target))]
        return cargo_base + owned + (["-Zchecksum-freshness", "--config", f'build.fingerprint="{mode}"']
                             if mode != "default" else [])

    def profile_flags(profile):
        # Preserve legacy dev/test selection for single-profile invocations.
        # A comparison explicitly tests each named profile, including dev.
        return [] if profile == "dev" and len(profiles) == 1 else ["--profile=" + profile]

    def build(name, command, cwd, env, profile, *, scenario, iteration, expected_error=False):
        row = run(name, command + ["build", "--locked", "--timings", f"-j{args.jobs}",
                            "--target-dir", env["CARGO_TARGET_DIR"]] + workload + profile_flags(profile),
            cwd=cwd, env=env, phase="build", cargo_profile=profile, scenario=scenario, iteration=iteration,
            check=not expected_error)
        if expected_error:
            # Cargo uses 101 for compilation failure. A missing tool, signal,
            # unrelated failure or unexpected success does not qualify.
            diagnostic = "error: " + ERROR_MARKER
            log_text = (evidence / (name + ".log")).read_text(errors="replace")
            log_text = re.sub(r"\x1b\[[0-9;]*m", "", log_text)
            matched = diagnostic in log_text
            valid = row["exit_code"] == 101 and matched
            record(dict(name=name + "-validation", phase="validation", scenario=scenario,
                        cargo_profile=profile, iteration=iteration, build_exit_code=row["exit_code"],
                        expected_exit_code=101, diagnostic_matched=matched, exit_code=0 if valid else 1))
            if not valid:
                raise RuntimeError(f"{name} did not produce the expected compile error; see {evidence / (name + '.log')}")
            # Cargo may leave the previous successful timing HTML behind.
            # Never label that stale report as this failed build's timing.
            return
        timing = Path(env["CARGO_TARGET_DIR"]) / "cargo-timings/cargo-timing.html"
        shutil.copyfile(timing, evidence / (name + "-timing.html"))

    def restore(name, target, restored, profile, iteration):
        start = time.monotonic()
        error = None
        try:
            # Preserve artifact timestamps, never source timestamps. Copy
            # symlinks as links instead of following external directories.
            shutil.copytree(target, restored, symlinks=True, copy_function=shutil.copy2)
        except OSError as failure:
            error = str(failure)
            raise
        finally:
            row = dict(name=name, phase="restore", operation="copytree", source=str(target), destination=str(restored),
                       cargo_profile=profile, cargo_incremental=base_env["CARGO_INCREMENTAL"],
                       scenario="fresh-worktree-restore", iteration=iteration,
                       seconds=round(time.monotonic() - start, 4), exit_code=1 if error else 0, error=error)
            (evidence / (name + ".log")).write_text(json.dumps(row, indent=2) + "\n")
            record(row)

    worktrees = []

    def add_worktree(name):
        path = scratch / name
        # Git can leave a registered checkout when a post-checkout hook fails.
        # Include partial creations in non-forced cleanup too.
        worktrees.append(path)
        run(name + "-checkout", ["git", "worktree", "add", "--detach", str(path), controls["revision"]])
        if inherited_cargo_config_hashes(path) != controls["inherited_cargo_config_hashes"]:
            raise RuntimeError("Source worktree inherits different Cargo configuration; use a TMPDIR with the same ancestor config, or move shared config to CARGO_HOME")
        if source_hashes(path) != metadata["source_hashes"]:
            raise RuntimeError("Source worktree configuration differs from the preflight checkout")
        return path

    completed = False
    retained_worktrees = []
    try:
        for tool, details in controls["tool_versions"].items():
            (evidence / (tool + "-version.log")).write_text(repr(details["command"]) + "\n" + details["output"] + "\n")
        source = add_worktree("seed") if controls["source_snapshot"] else ROOT
        if "cargo" in tools:
            run("cargo-fetch", cargo_base + ["fetch", "--locked"], cwd=source)
        if "bazel" in tools:
            run("bazel-version", bazel + ["--version"])
            run("bazel-fetch", bazel + ["fetch", "//:all_build"] + repository_flags)
            run("bazel-analysis", bazel + ["build", "//:all_build", "--nobuild"] + flags)
        for i in range(1, args.iterations + 1):
            for tool in tools if i % 2 else list(reversed(tools)):
                if tool == "bazel":
                    run(f"bazel-clean-{i}", bazel + ["clean"])
                    run(f"bazel-cold-{i}", bazel + ["build", "//:all_build",
                        "--profile=" + str(evidence / f"bazel-profile-{i}.json.gz")] + flags, phase="build")
                    continue
                for profile, mode, label in variants if i % 2 else list(reversed(variants)):
                    target = artifacts / label
                    env = cargo_env(mode, target)
                    command = cargo_command(mode, target)
                    run(f"{label}-clean-{i}", command + ["clean", "--target-dir", str(target)], cwd=source, env=env,
                        cargo_profile=profile, scenario="clean", iteration=i)
                    build(f"{label}-cold-{i}", command, source, env, profile, scenario="cold", iteration=i)
                    if mode == "default" and len(profiles) == 1:
                        shutil.copyfile(evidence / f"{label}-cold-{i}-timing.html", evidence / f"cargo-timing-{i}.html")
                    if "noop" in args.cargo_scenario:
                        build(f"{label}-noop-{i}", command, source, env, profile, scenario="noop", iteration=i)
                    if "fresh-worktree" in args.cargo_scenario:
                        fresh_label = f"{label}-fresh-{i}"
                        reader = add_worktree(fresh_label)
                        restored = artifacts / fresh_label
                        restore(fresh_label + "-restore", target, restored, profile, i)
                        build(fresh_label, cargo_command(mode, restored), reader, cargo_env(mode, restored), profile,
                              scenario="fresh-worktree", iteration=i)
                    if "edit" in args.cargo_scenario:
                        for probe in PROBES:
                            with semantic_edit(source, probe, i):
                                build(f"{label}-edit-{probe[0]}-{i}", command, source, env, profile,
                                      scenario="edit-" + probe[0], iteration=i)
                            build(f"{label}-undo-{probe[0]}-{i}", command, source, env, profile,
                                  scenario="undo-" + probe[0], iteration=i)
                    if "error-repair" in args.cargo_scenario:
                        if source not in worktrees:
                            raise RuntimeError("Compile-error probe requires an owned source worktree")
                        name = f"{label}-error-{i}"
                        with compile_error(source) as edit:
                            (evidence / (name + "-source.json")).write_text(json.dumps(edit, indent=2) + "\n")
                            build(name, command, source, env, profile,
                                  scenario="compile-error", iteration=i, expected_error=True)
                        build(f"{label}-repair-{i}", command, source, env, profile,
                              scenario="error-repair", iteration=i)
        if "cargo" in tools:
            for profile, mode, label in variants:
                verification_profile = "test" if profile == "dev" and len(profiles) == 1 else profile
                run(label + "-tests", cargo_command(mode, artifacts / label) + ["test", "--locked", "--workspace", "--all-features",
                    "--lib", "--bins", "--tests", f"-j{args.jobs}", "--target-dir", str(artifacts / label)] + profile_flags(profile),
                    cwd=source, env=cargo_env(mode, artifacts / label), phase="verification",
                    cargo_profile=verification_profile, scenario="verification")
        if "bazel" in tools:
            run("bazel-tests", bazel + ["test", "//:tests", "--cache_test_results=no"] + flags, phase="verification")
        completed = True
    finally:
        primary_failure = sys.exc_info()[0] is not None
        cleanup_errors = []

        def cleanup(name, command):
            try:
                result = run(name, command, check=False, phase="cleanup")
                if not result["exit_code"]:
                    return True
                cleanup_errors.append(f"{name} exited {result['exit_code']}")
            except Exception as error:
                cleanup_errors.append(f"{name}: {error}")
            return False

        if "bazel" in tools:
            cleanup("bazel-shutdown", bazel + ["shutdown"])
        for path in reversed(worktrees):
            if not cleanup(path.name + "-remove", ["git", "worktree", "remove", str(path)]):
                retained_worktrees.append(str(path))
        if not retained_worktrees and not cleanup_errors:
            try:
                shutil.rmtree(scratch)
            except OSError as error:
                cleanup_errors.append(f"scratch cleanup: {error}")
        try:
            completion = dict(schema_version=SCHEMA_VERSION, completed=completed and not cleanup_errors,
                              retained_worktrees=retained_worktrees, cleanup_errors=cleanup_errors,
                              source_hashes_unchanged=source_hashes(ROOT) == metadata["source_hashes"])
            (evidence / "completion.json").write_text(json.dumps(completion, indent=2) + "\n")
        except OSError as error:
            if not primary_failure:
                raise
            print(f"Could not record completion: {error}", file=sys.stderr)
    if cleanup_errors:
        raise RuntimeError(f"Cleanup failed; see {evidence / 'completion.json'}")
    print(f"Evidence: {evidence}; retained build/download artifacts: {artifacts}")


if __name__ == "__main__":
    main()

