#!/usr/bin/env python3
"""Trial bazel-remote's compressed gRPC cache with measured TCP payload bytes."""
import argparse
from concurrent.futures import ThreadPoolExecutor
import hashlib
import http.client
import importlib.util
import json
import os
from pathlib import Path
import socket
import subprocess
import tempfile
import threading
import time
import urllib.request


def load(name, path):
    spec = importlib.util.spec_from_file_location(name, path)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


def port():
    with socket.socket() as sock:
        sock.bind(("127.0.0.1", 0))
        return sock.getsockname()[1]


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--writer", type=Path, required=True)
    parser.add_argument("--reader", type=Path, required=True)
    parser.add_argument("--store", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    writer, reader, store, evidence = [p.resolve() for p in (args.writer, args.reader, args.store, args.output)]
    if writer == reader or store in (writer, reader):
        parser.error("Use distinct writer, reader, and cache-host worktrees")
    evidence.mkdir(parents=True, exist_ok=False)
    binary = writer / "target/bazel-remote-bootstrap/bazel-remote-2.6.3-linux-amd64"
    if hashlib.sha256(binary.read_bytes()).hexdigest() != "65427624c288e29b17c7b0e189cf1d03ae1d8745354d9effc432f75c4361159a":
        raise RuntimeError("Unverified bazel-remote executable")
    if "BAZEL_RUST_STDLIB_LINKFLAGS" in os.environ:
        raise RuntimeError("Unset untracked BAZEL_RUST_STDLIB_LINKFLAGS")
    source = reader / "crates/core/src/patch.rs"
    original = source.read_bytes()
    if original != (writer / "crates/core/src/patch.rs").read_bytes():
        raise RuntimeError("Core source differs")
    outputs = reader / "target/grpc-cache-trial"
    outputs.mkdir(exist_ok=False)
    http_port, grpc_port = port(), port()
    cache = store / "target/bazel-remote"
    cache.mkdir(parents=True, exist_ok=True)
    log = (evidence / "server.log").open("w")
    command = [str(binary), "--dir=" + str(cache), "--max_size=10",
        f"--http_address=127.0.0.1:{http_port}", f"--grpc_address=127.0.0.1:{grpc_port}",
        "--storage_mode=zstd", "--enable_endpoint_metrics"]
    backend = None
    proxy = None
    connections = []
    servers = set()
    scratch = tempfile.TemporaryDirectory(prefix="diffz-grpc-cache-", dir=os.environ.get("TMPDIR", "/var/tmp"))
    env = {**os.environ, "TMPDIR": scratch.name, "CC": "/usr/bin/gcc", "CXX": "/usr/bin/g++",
        "CCACHE_DISABLE": "1", "RUSTC_WRAPPER": "", "RUSTC_WORKSPACE_WRAPPER": "",
        "DIFFZ_BAZEL_CACHE": "0", "DIFFZ_BAZEL_REMOTE_CACHE_READ_ONLY": "0",
        "DIFFZ_BAZEL_TEST_TMP_ROOT": str(Path(os.environ.get("TMPDIR", "/var/tmp")) / "diffz-shared-cache-tests")}
    env.pop("DIFFZ_BAZEL_REMOTE_CACHE", None)
    env.pop("DIFFZ_BAZEL_REPO_CONTENTS_CACHE", None)

    def run(name, command, output, cwd=None, additions=None):
        before = proxy.snapshot()
        started = time.monotonic()
        with (evidence / (name + ".log")).open("w") as stream:
            stream.write(repr(command) + "\n")
            stream.flush()
            result = subprocess.run(command, cwd=cwd or reader,
                                    env={**env, "DIFFZ_BAZEL_OUTPUT_ROOT": str(output), **(additions or {})},
                                    stdout=stream, stderr=subprocess.STDOUT)
        elapsed = time.monotonic() - started
        after = proxy.snapshot()
        row = dict(name=name, seconds=elapsed, exit_code=result.returncode, command=command,
            wire_metered=any(f"grpc://127.0.0.1:{proxy.port}" in x for x in command),
            download_wire_bytes=after["down_bytes"] - before["down_bytes"],
            upload_wire_bytes=after["up_bytes"] - before["up_bytes"])
        with (evidence / "measurements.jsonl").open("a") as out:
            out.write(json.dumps(row) + "\n")
        print(json.dumps(row), flush=True)
        if result.returncode:
            raise RuntimeError(f"{name} failed")
        return row

    try:
        backend = subprocess.Popen(command, stdout=log, stderr=subprocess.STDOUT)
        for _ in range(100):
            if backend.poll() is not None:
                raise RuntimeError("bazel-remote failed to start")
            try:
                with urllib.request.urlopen(f"http://127.0.0.1:{http_port}/status", timeout=1):
                    break
            except OSError:
                time.sleep(0.1)
        else:
            raise RuntimeError("bazel-remote startup timed out")
        # Import the existing seed without any compilation. CAS precedes AC so
        # the official server can validate all referenced blobs.
        local = threading.local()
        lock = threading.Lock()
        def upload(path):
            if not hasattr(local, "connection"):
                local.connection = http.client.HTTPConnection("127.0.0.1", http_port, timeout=120)
                with lock:
                    connections.append(local.connection)
            with path.open("rb") as body:
                local.connection.request("PUT", "/" + path.parent.name + "/" + path.name,
                    body=body, headers={"Content-Length": str(path.stat().st_size)})
                response = local.connection.getresponse()
                data = response.read()
                if response.status not in (200, 204):
                    raise RuntimeError(f"Import {path}: {response.status} {data[:200]!r}")
            return path.stat().st_size
        import_start, imported = time.monotonic(), {}
        for kind in ("cas", "ac"):
            files = list((writer / "target/shared-cache-trial/http" / kind).iterdir())
            with ThreadPoolExecutor(max_workers=16) as pool:
                sizes = list(pool.map(upload, files))
            imported[kind] = dict(files=len(files), bytes=sum(sizes))
        for connection in connections:
            connection.close()
        (evidence / "import.json").write_text(json.dumps(dict(seconds=time.monotonic() - import_start,
            imported=imported, server_command=command), indent=2) + "\n")
        wire = load("cache_wire", Path(__file__).with_name("cache-wire-proxy.py"))
        proxy = wire.WireProxy("127.0.0.1", grpc_port, mib_per_second=100)
        proxy.start()
        flags = ["--config=fast-dev", "--disk_cache=", f"--remote_cache=grpc://127.0.0.1:{proxy.port}",
            "--remote_upload_local_results=false",
            "--repository_cache=" + str(writer / "target/bazel/cache/repos/v1"),
            "--repo_contents_cache=" + str(store / "target/repositories")]
        (evidence / "definition.json").write_text(json.dumps(dict(flags=flags, server="bazel-remote 2.6.3",
            protocol="gRPC zstd; aggregate 100 MiB/s TCP downlink payload cap; loopback latency, no per-RPC delay",
            source_sha256=hashlib.sha256(original).hexdigest(), configs="same fast-dev alias as HTTP trials"), indent=2) + "\n")
        for mode in ("plain", "toplevel", "all"):
            output = outputs / ("bazel-" + mode)
            servers.add(output)
            def bazel(name, operation="build", extra=None):
                targets = ["//:tests"] if operation == "test" else ["//:all_build"]
                return run(mode + "-" + name, ["bash", "scripts/bazel-trial.sh", operation] + targets + flags + [
                    "--remote_download_outputs=" + ("toplevel" if mode == "plain" else mode),
                    "--remote_cache_compression=" + str(mode != "plain").lower(),
                    "--profile=" + str(evidence / (mode + "-" + name + ".profile.json.gz"))] + (extra or []), output)
            bazel("fresh")
            if mode == "plain":
                run(mode + "-shutdown", ["bash", "scripts/bazel-trial.sh", "shutdown"], output)
                servers.discard(output)
                continue
            bazel("cached-tests", "test")
            bazel("execute-tests", "test", ["--cache_test_results=no"])
            for i in (1, 2):
                changed = original.replace(b"max_files: 10_000,", f"max_files: {61400+i:_},".encode())
                if changed == original or source.read_bytes() != original:
                    raise RuntimeError("Semantic probe mismatch")
                source.write_bytes(changed)
                try:
                    bazel(f"core-edit-{i}")
                    bazel(f"core-edit-{i}-tests", "test", ["--cache_test_results=no"])
                finally:
                    if source.read_bytes() != changed:
                        raise RuntimeError("Concurrent source edit")
                    source.write_bytes(original)
                bazel(f"core-undo-{i}")
            run(mode + "-shutdown", ["bash", "scripts/bazel-trial.sh", "shutdown"], output)
            servers.discard(output)
        # Make the primary checkout useful immediately, and verify the local
        # disk tier can restore the graph without any running remote service.
        primary = Path(__file__).resolve().parent.parent
        ready = primary / "target/bazel"
        additions = {"DIFFZ_BAZEL_CACHE": "1", "DIFFZ_BAZEL_REMOTE_CACHE": f"grpc://127.0.0.1:{grpc_port}",
            "DIFFZ_BAZEL_REMOTE_CACHE_READ_ONLY": "1", "DIFFZ_BAZEL_REPO_CONTENTS_CACHE": str(store / "target/repositories")}
        servers.add(ready)
        run("primary-cache-populate", ["bash", "scripts/bazel-trial.sh", "build", "//:all_build",
            "--config=fast-dev", "--remote_cache_compression", "--remote_download_outputs=all"], ready, primary, additions)
        run("primary-shutdown", ["bash", "scripts/bazel-trial.sh", "shutdown"], ready, primary, additions)
        servers.discard(ready)
        disk_output = primary / "target/shared-cache-disk-verify"
        servers.add(disk_output)
        additions.pop("DIFFZ_BAZEL_REMOTE_CACHE")
        additions["DIFFZ_BAZEL_REMOTE_CACHE_READ_ONLY"] = "0"
        run("primary-disk-only-restore", ["bash", "scripts/bazel-trial.sh", "build", "//:all_build",
            "--config=fast-dev", "--remote_download_outputs=all"], disk_output, primary, additions)
        run("primary-disk-shutdown", ["bash", "scripts/bazel-trial.sh", "shutdown"], disk_output, primary, additions)
        servers.discard(disk_output)
        servers.add(ready)
        run("primary-local-warm-build", ["bash", "scripts/bazel-trial.sh", "build", "//:all_build",
            "--config=fast-dev", "--remote_download_outputs=all"], ready, primary, additions)
        run("primary-final-shutdown", ["bash", "scripts/bazel-trial.sh", "shutdown"], ready, primary, additions)
        servers.discard(ready)
        with urllib.request.urlopen(f"http://127.0.0.1:{http_port}/metrics") as response:
            (evidence / "server-metrics.txt").write_bytes(response.read())
        (evidence / "completion.json").write_text(json.dumps(dict(sources_restored=source.read_bytes() == original,
            outputs=str(outputs), cache=str(cache)), indent=2) + "\n")
    finally:
        for output in servers:
            root = primary if "primary" in locals() and output.parent == primary / "target" else reader
            subprocess.run(["bash", "scripts/bazel-trial.sh", "shutdown"], cwd=root,
                env={**env, "DIFFZ_BAZEL_OUTPUT_ROOT": str(output)}, stdout=subprocess.DEVNULL, stderr=subprocess.STDOUT)
        if proxy:
            proxy.stop()
        for connection in connections:
            connection.close()
        if backend:
            backend.terminate()
            try:
                backend.wait(timeout=10)
            except subprocess.TimeoutExpired:
                backend.kill()
                backend.wait()
        log.close()
        scratch.cleanup()


if __name__ == "__main__":
    main()
