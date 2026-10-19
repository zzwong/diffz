#!/usr/bin/env python3
"""Compare HTTP Bazel action reuse and Cargo artifact restoration across checkouts.

Run against two prepared, identical checkouts. Both must have fetched external
dependencies; those downloads are reported separately by this driver. Builds
are serialized. The HTTP cache is localhost-only and stops when the driver exits.
"""
import argparse
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import shutil
import subprocess
import tempfile
import threading
import time


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--writer", type=Path, required=True)
    parser.add_argument("--reader", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--iterations", type=int, default=2)
    parser.add_argument("--reuse-seed", action="store_true", help="Reuse completed seed with a new evidence directory")
    parser.add_argument("--seed-change-reason", help="Explicitly record intentional source differences from a reused seed")
    parser.add_argument("--consumer-tag", default="shared-cache-trial")
    parser.add_argument("--repo-contents-cache", type=Path)
    parser.add_argument("--skip-edits", action="store_true")
    parser.add_argument("--download-outputs", choices=("toplevel", "all"), default="toplevel")
    parser.add_argument("--network", action="append", choices=("loopback", "100mib-5ms"))
    parser.add_argument("--checkout-timestamps", choices=("fresh", "preserved"), default="fresh",
                        help="Fresh mimics Git checkout mtimes; preserved is an optimistic Cargo archive control")
    parser.add_argument("--edit-kind", action="append", choices=("core", "ui", "app"))
    args = parser.parse_args()
    if os.environ.get("BAZEL_RUST_STDLIB_LINKFLAGS") is not None:
        parser.error("Unset BAZEL_RUST_STDLIB_LINKFLAGS: pinned rules_rust does not track it for repo cache keys")
    writer, reader, evidence = args.writer.resolve(), args.reader.resolve(), args.output.resolve()
    if writer == reader or args.iterations < 1 or not args.consumer_tag.replace("-", "").isalnum():
        parser.error("Need two different checkouts and positive iterations")
    evidence.mkdir(parents=True, exist_ok=False)
    def fingerprint(root):
        files = subprocess.check_output(["git", "ls-files", "--cached", "--others", "--exclude-standard"],
                                        cwd=root, text=True).splitlines()
        return {p: hashlib.sha256((root / p).read_bytes()).hexdigest() for p in files if (root / p).is_file()}
    source_hashes = fingerprint(writer)
    if source_hashes != fingerprint(reader):
        raise RuntimeError("Writer/reader source trees differ; prepare identical checkouts first")
    heads = [subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=p, text=True).strip() for p in (writer, reader)]
    if heads[0] != heads[1]:
        raise RuntimeError("Writer/reader HEADs differ")
    owned = writer / "target/shared-cache-trial"
    artifacts = reader / "target" / args.consumer_tag
    owned.mkdir(parents=True, exist_ok=True)
    artifacts.mkdir(parents=True, exist_ok=True)
    seed_fingerprint = owned / "seed-fingerprint.json"
    seed_differences = []
    if args.reuse_seed:
        if not seed_fingerprint.is_file():
            raise RuntimeError("Missing seed source fingerprint")
        previous = json.loads(seed_fingerprint.read_text())
        seed_differences = [p for p in sorted(previous.keys() | source_hashes.keys()) if previous.get(p) != source_hashes.get(p)]
        if seed_differences and not args.seed_change_reason:
            raise RuntimeError("Seed source changed; record why using --seed-change-reason: " + repr(seed_differences))
    spec = importlib.util.spec_from_file_location("shared_cache", Path(__file__).with_name("shared-action-cache.py"))
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    server = module.CacheServer(owned / "http")
    thread = threading.Thread(target=server.serve_forever, daemon=True)
    endpoint = f"http://127.0.0.1:{server.server_port}"
    flags = ["--config=pipeline", "--config=native2", "--config=buildtools0",
             "--jobs=32", "--local_resources=cpu=32", "--disk_cache=",
             "--remote_cache=" + endpoint, "--remote_download_outputs=" + args.download_outputs,
             "--repository_cache=" + str(writer / "target/bazel/cache/repos/v1")]
    if args.repo_contents_cache:
        flags += ["--repo_contents_cache=" + str(args.repo_contents_cache.resolve())]
    baseline = {p: (reader / p).read_bytes() for p in ["crates/core/src/patch.rs",
        "crates/ui/src/magnify.rs", "crates/app/src/main.rs"]}
    for p, content in baseline.items():
        if (writer / p).read_bytes() != content:
            raise RuntimeError(f"Source mismatch: {p}")
    (evidence / "definition.json").write_text(json.dumps(dict(writer=str(writer), reader=str(reader),
        flags=flags, cargo_profile="fast-dev", jobs=32, iterations=args.iterations, head=heads[0],
        full_source_sha256=source_hashes,
        seed_differences=seed_differences, seed_change_reason=args.seed_change_reason,
        checkout_timestamps=args.checkout_timestamps,
        source_hashes={p: hashlib.sha256(b).hexdigest() for p, b in baseline.items()},
        network="localhost HTTP; also simulated aggregate 100 MiB/s GET bodies and 5 ms per GET; uploads unshaped",
        profile_difference="Cargo workspace CGU256/incremental; Bazel workspace CGU16/no stable rustc incremental; same earlier measured configurations",
        exclusions="OS page caches warm; repository/toolchain downloads are setup, not action-cache bytes; same host native tools",
        test_policy="Cached tests measured separately; edit cycles force test execution"), indent=2) + "\n")
    scratch = tempfile.TemporaryDirectory(prefix="diffz-shared-cache-", dir=os.environ.get("TMPDIR", "/var/tmp"))
    test_root = Path(os.environ.get("TMPDIR", "/var/tmp")) / "diffz-shared-cache-tests"
    env = {**os.environ, "TMPDIR": scratch.name, "CC": "/usr/bin/gcc", "CXX": "/usr/bin/g++",
           "CCACHE_DISABLE": "1", "RUSTC_WRAPPER": "", "RUSTC_WORKSPACE_WRAPPER": "",
           "CARGO_INCREMENTAL": "1", "DIFFZ_BAZEL_CACHE": "0",
           "DIFFZ_BAZEL_TEST_TMP_ROOT": str(test_root),
           "DIFFZ_BAZEL_REMOTE_CACHE_READ_ONLY": "0"}
    # Don't let a developer's endpoint override this isolated experiment.
    env.pop("DIFFZ_BAZEL_REMOTE_CACHE", None)
    env.pop("DIFFZ_BAZEL_REPO_CONTENTS_CACHE", None)
    servers = set()

    def run(name, command, cwd, additions=None, check=True):
        index = len(server.snapshot())
        started = time.monotonic()
        with (evidence / (name + ".log")).open("w") as log:
            log.write(repr(command) + "\n")
            log.flush()
            result = subprocess.run(command, cwd=cwd, env={**env, **(additions or {})},
                                    stdout=log, stderr=subprocess.STDOUT)
        elapsed = time.monotonic() - started
        records = server.snapshot()[index:]
        gets = [r for r in records if r["method"] == "GET"]
        # Wall-clock union of concurrent HTTP request spans, not a sum that
        # double-counts parallel downloads. Includes latency and server throttling.
        union, end = 0.0, 0.0
        for row in sorted(gets, key=lambda r: r["start"]):
            union += max(0.0, row["end"] - max(end, row["start"]))
            end = max(end, row["end"])
        row = dict(name=name, seconds=round(elapsed, 4), exit_code=result.returncode,
            cwd=str(cwd), command=command, download_bytes=sum(r["bytes"] for r in gets),
            upload_bytes=sum(r["bytes"] for r in records if r["method"] == "PUT"),
            http_gets=len(gets), http_hits=sum(r["status"] == 200 for r in gets),
            http_misses=sum(r["status"] == 404 for r in gets),
            http_busy_wall_seconds=round(union, 4))
        row["http_by_route"] = {route: dict(
            download_bytes=sum(r["bytes"] for r in gets if r["path"].startswith("/" + route + "/")),
            hits=sum(r["status"] == 200 for r in gets if r["path"].startswith("/" + route + "/")),
            misses=sum(r["status"] == 404 for r in gets if r["path"].startswith("/" + route + "/")))
            for route in ("ac", "cas", "artifacts")}
        with (evidence / "measurements.jsonl").open("a") as out:
            out.write(json.dumps(row) + "\n")
        with (evidence / "requests.jsonl").open("a") as out:
            for record in records:
                out.write(json.dumps({"sample": name, **record}) + "\n")
        print(json.dumps(row), flush=True)
        if check and result.returncode:
            raise RuntimeError(f"{name} failed: {evidence / (name + '.log')}")
        return row

    def bazel(name, root, output, operation, targets=None, extra=None, write=False):
        additions = {"DIFFZ_BAZEL_OUTPUT_ROOT": str(output)}
        if operation == "shutdown":
            return run(name, ["bash", "scripts/bazel-trial.sh", "shutdown"], root, additions, check=False)
        servers.add((root, output))
        command = ["bash", "scripts/bazel-trial.sh", operation] + (targets or []) + flags + [
            "--remote_upload_local_results=" + str(write).lower()]
        if operation in ("build", "test"):
            command += ["--profile=" + str(evidence / (name + ".profile.json.gz"))]
        return run(name, command + (extra or []), root, additions)

    def cargo(name, root, output, operation="build"):
        command = ["cargo", operation, "--locked", "--offline", "--workspace", "--all-features",
                   "--profile=fast-dev", "-j32"]
        command += ["--all-targets"] if operation == "build" else ["--lib", "--bins", "--tests"]
        return run(name, command, root, {"CARGO_TARGET_DIR": str(output)})

    def remove_output(path):
        # Only this driver's artifact directories may be removed. Bazel output
        # directories may be read-only; never chmod files or follow symlinks.
        path = Path(path)
        if path.parent != artifacts or not path.name.startswith(("bazel-", "cargo-")):
            raise RuntimeError(f"Refusing cleanup outside owned outputs: {path}")
        if path.is_symlink():
            raise RuntimeError(f"Refusing to remove symlink: {path}")
        if path.exists():
            for directory, dirs, _ in os.walk(path, followlinks=False):
                if not Path(directory).is_symlink():
                    os.chmod(directory, 0o755)
            shutil.rmtree(path)

    def restore_cargo(label, output):
        remove_output(output)
        output.mkdir()
        download = artifacts / (label + ".tar.zst")
        run(label + "-download", ["curl", "--fail", "--silent", "--show-error", endpoint + "/artifacts/cargo-fast-dev.tar.zst",
            "--output", str(download)], reader)
        run(label + "-extract", ["tar", "--zstd", "-xf", str(download), "-C", str(output)], reader)
        download.unlink()
        cargo(label + "-build", reader, output)

    seed_bazel = owned / "writer-bazel"
    seed_cargo = owned / "cargo-seed"
    archive = owned / "http/artifacts/cargo-fast-dev.tar.zst"
    thread.start()
    try:
        run("versions", ["rustc", "-Vv"], writer)
        run("cargo-fetch", ["cargo", "fetch", "--locked"], writer)
        servers.add((writer, seed_bazel))
        run("bazel-fetch-writer", ["bash", "scripts/bazel-trial.sh", "fetch", "//:all_build",
            "--repository_cache=" + str(writer / "target/bazel/cache/repos/v1")], writer,
            {"DIFFZ_BAZEL_OUTPUT_ROOT": str(seed_bazel)})
        if not args.reuse_seed:
            if seed_cargo.exists() or archive.exists() or (owned / "http/ac").exists():
                raise RuntimeError("Seed artifacts exist; use --resume only after a complete successful seed")
            cargo("cargo-seed-build", writer, seed_cargo)
            cargo("cargo-seed-tests", writer, seed_cargo, "test")
            archive.parent.mkdir(parents=True, exist_ok=True)
            run("cargo-archive-create", ["tar", "--use-compress-program=zstd -T4 -3", "-cf", str(archive),
                "-C", str(seed_cargo), "."], writer)
            bazel("bazel-seed-build", writer, seed_bazel, "build", ["//:all_build"], write=True)
            bazel("bazel-seed-tests", writer, seed_bazel, "test", ["//:tests"], write=True)
            (owned / "seed-complete").write_text("completed\n")
            seed_fingerprint.write_text(json.dumps(source_hashes, indent=2) + "\n")
        elif not (owned / "seed-complete").exists():
            raise RuntimeError("No completed seed to resume")
        if args.reuse_seed:
            # Test temporary-directory environment participates in test keys.
            # Refresh coverage for this declared environment before consumers.
            bazel("bazel-seed-tests-refresh", writer, seed_bazel, "test", ["//:tests"], write=True)
        bazel("shutdown-writer-after-seed", writer, seed_bazel, "shutdown")
        servers.discard((writer, seed_bazel))
        # A separate, empty output_user_root also counts startup/analysis and
        # repository materialization in each fresh consumer measurement.
        for network, rate, latency in [("loopback", 0, 0), ("100mib-5ms", 100, 5)]:
            if args.network and network not in args.network:
                continue
            with server.rate_lock:
                server.rate = rate * 1024 * 1024
                server.latency = latency / 1000
                server.next_send = time.monotonic()
            for i in range(1, args.iterations + 1):
                # A real git checkout gives files new mtimes. Cargo's artifact
                # freshness checks can invalidate path crates even when bytes
                # match; Bazel hashes content. Preserve timestamps only as an
                # explicitly named optimistic Cargo control.
                if args.checkout_timestamps == "fresh":
                    now = time.time_ns()
                    if now <= archive.stat().st_mtime_ns:
                        raise RuntimeError("Checkout timestamp must be newer than the seeded archive")
                    for relative in source_hashes:
                        os.utime(reader / relative, ns=(now, now), follow_symlinks=False)
                    with (evidence / "checkout-timestamps.jsonl").open("a") as out:
                        out.write(json.dumps(dict(network=network, iteration=i, source_touch_ns=now,
                            source_files=len(source_hashes), archive_mtime_ns=archive.stat().st_mtime_ns)) + "\n")
                boutput = artifacts / f"bazel-{network}-{i}"
                coutput = artifacts / f"cargo-{network}-{i}"
                if boutput.exists() or coutput.exists():
                    raise RuntimeError("Fresh output directory already exists")
                tools = ["bazel", "cargo"] if i % 2 else ["cargo", "bazel"]
                for tool in tools:
                    label = f"fresh-{network}-{i}-{tool}"
                    if tool == "bazel":
                        result = bazel(label, reader, boutput, "build", ["//:all_build"])
                        if result["http_misses"] or not result["http_hits"]:
                            raise RuntimeError("Fresh baseline did not fully hit the seeded HTTP cache")
                        bazel(label + "-tests-cached", reader, boutput, "test", ["//:tests"])
                        bazel(label + "-tests-execute", reader, boutput, "test", ["//:tests"], ["--cache_test_results=no"])
                    else:
                        restore_cargo(label, coutput)
                        cargo(label + "-tests", reader, coutput, "test")
                if network == "loopback" and i == 1:
                    # Best-case same-host Cargo artifact reuse, no archive.
                    cargo("cargo-shared-target-cross-root-build", reader, seed_cargo)
                    cargo("cargo-shared-target-cross-root-tests", reader, seed_cargo, "test")
                if network == "100mib-5ms" and i == 1 and not args.skip_edits:
                    for kind, relative, old, new in [
                        ("core", "crates/core/src/patch.rs", b"max_files: 10_000,", b"max_files: 41_719,"),
                        ("ui", "crates/ui/src/magnify.rs", b"MAX_SCALE: f32 = 5.0;", b"MAX_SCALE: f32 = 5.375;"),
                        ("app", "crates/app/src/main.rs", b'"diffz {}"', b'"diffz cache-trial-41719 {}"'),
                    ]:
                        if args.edit_kind and kind not in args.edit_kind:
                            continue
                        source = reader / relative
                        original = baseline[relative]
                        if old not in original:
                            raise RuntimeError("Semantic probe no longer matches")
                        for iteration in range(1, args.iterations + 1):
                            # Each sample has a new semantic input, not a replay
                            # of a content hash uploaded by an earlier build.
                            value = new.replace(b"41_719", f"{41719 + iteration:_}".encode()).replace(
                                b"5.375", f"{5.375 + iteration / 32:.5f}".encode()).replace(
                                b"41719", str(41719 + iteration).encode())
                            changed = original.replace(old, value)
                            if kind == "ui":
                                changed = changed.replace(b"assert_eq!(window.magnification().scale, 4.0);",
                                    b"assert_eq!(window.magnification().scale, MAX_SCALE * 0.8);")
                            if source.read_bytes() != original:
                                raise RuntimeError("Concurrent source edit")
                            source.write_bytes(changed)
                            try:
                                for tool in (["cargo", "bazel"] if iteration % 2 else ["bazel", "cargo"]):
                                    label = f"edit-{kind}-{iteration}-{tool}"
                                    if tool == "bazel":
                                        bazel(label, reader, boutput, "build", ["//:all_build"])
                                        bazel(label + "-tests", reader, boutput, "test", ["//:tests"], ["--cache_test_results=no"])
                                    else:
                                        cargo(label, reader, coutput)
                                        cargo(label + "-tests", reader, coutput, "test")
                            finally:
                                if source.read_bytes() != changed:
                                    raise RuntimeError("Source changed concurrently; refusing to overwrite")
                                source.write_bytes(original)
                            cargo(f"undo-{kind}-{iteration}-cargo", reader, coutput)
                            bazel(f"undo-{kind}-{iteration}-bazel", reader, boutput, "build", ["//:all_build"])
                bazel(f"shutdown-{network}-{i}", reader, boutput, "shutdown")
                servers.discard((reader, boutput))
                # Keep one consumer pair for follow-up inspection, reclaim the rest.
                if not (network == "100mib-5ms" and i == 1):
                    remove_output(boutput)
                    remove_output(coutput)
        (evidence / "completion.json").write_text(json.dumps(dict(archive_bytes=archive.stat().st_size,
            sources_restored=all((reader / p).read_bytes() == b for p, b in baseline.items()),
            full_tree_restored=fingerprint(reader) == source_hashes,
            retained_artifacts=[str(owned), str(artifacts)]), indent=2) + "\n")
    finally:
        for root, output in servers:
            bazel("shutdown-final-" + output.name, root, output, "shutdown")
        server.shutdown()
        server.server_close()
        thread.join(timeout=5)
        scratch.cleanup()
        if test_root.exists():
            try:
                test_root.rmdir()
            except OSError:
                pass  # Another concurrent test may still own a child.


if __name__ == "__main__":
    main()
