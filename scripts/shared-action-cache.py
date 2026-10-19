#!/usr/bin/env python3
"""Instrumented localhost HTTP cache for experiments, not a public cache service.

Implements Bazel's opaque /ac and /cas GET/PUT protocol and serves /artifacts
for a Cargo archive comparison. Data stays under the supplied worktree target/.
"""
import argparse
import hashlib
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
import json
import os
from pathlib import Path
import re
import tempfile
import threading
import time


class CacheServer(ThreadingHTTPServer):
    daemon_threads = True

    def __init__(self, root, port=0, mib_per_second=0, latency_ms=0):
        if mib_per_second < 0 or latency_ms < 0:
            raise ValueError("Network shaping values must be nonnegative")
        self.root = Path(root).resolve()
        self.root.mkdir(parents=True, exist_ok=True)
        self.lock = threading.Lock()
        self.idle = threading.Condition(self.lock)
        self.active = 0
        self.rate_lock = threading.Lock()
        self.next_send = time.monotonic()
        self.rate = mib_per_second * 1024 * 1024
        self.latency = latency_ms / 1000
        self.records = []
        super().__init__(("127.0.0.1", port), Handler)

    def begin(self):
        with self.idle:
            self.active += 1

    def finish(self, record):
        with self.idle:
            self.records.append(record)
            self.active -= 1
            self.idle.notify_all()

    def snapshot(self):
        with self.idle:
            if not self.idle.wait_for(lambda: self.active == 0, timeout=30):
                raise RuntimeError("HTTP requests did not drain")
            return list(self.records)

    def throttle(self, size):
        if self.rate:
            with self.rate_lock:
                now = time.monotonic()
                self.next_send = max(now, self.next_send) + size / self.rate
                delay = self.next_send - now
            time.sleep(delay)


class Handler(BaseHTTPRequestHandler):
    protocol_version = "HTTP/1.1"

    def log_message(self, *args):
        pass

    def path_file(self):
        if not re.fullmatch(r"/(?:ac|cas)/[0-9a-f]{64}|/artifacts/[a-zA-Z0-9_.-]+", self.path):
            return None
        return self.server.root / self.path.lstrip("/")

    def do_GET(self):
        self.server.begin()
        started = time.monotonic()
        count, status = 0, 404
        path = self.path_file()
        time.sleep(self.server.latency)
        try:
            if path is None or not path.is_file():
                self.send_error(404)
                return
            status = 200
            with path.open("rb") as stream:
                self.send_response(200)
                self.send_header("Content-Length", str(os.fstat(stream.fileno()).st_size))
                self.end_headers()
                while block := stream.read(256 * 1024):
                    self.server.throttle(len(block))
                    self.wfile.write(block)
                    count += len(block)
        except (BrokenPipeError, ConnectionResetError):
            status = 499
        except Exception:
            status = 500
            raise
        finally:
            self.server.finish(dict(method="GET", path=self.path, status=status,
                bytes=count, start=started, end=time.monotonic()))

    def do_PUT(self):
        self.server.begin()
        started = time.monotonic()
        path = self.path_file()
        temporary = None
        count, digest, status = 0, hashlib.sha256(), 500
        try:
            try:
                size = int(self.headers.get("Content-Length", "-1"))
            except ValueError:
                size = -1
            if path is None or not self.path.startswith(("/ac/", "/cas/")) or size < 0:
                status = 400
                self.send_error(400)
                return
            path.parent.mkdir(parents=True, exist_ok=True)
            with tempfile.NamedTemporaryFile(dir=path.parent, delete=False) as stream:
                temporary = Path(stream.name)
                while count < size:
                    block = self.rfile.read(min(256 * 1024, size - count))
                    if not block:
                        raise IOError("Truncated upload")
                    stream.write(block)
                    digest.update(block)
                    count += len(block)
            if self.path.startswith("/cas/") and digest.hexdigest() != path.name:
                status = 400
                self.send_error(400, "CAS digest mismatch")
                return
            temporary.replace(path)
            status = 200
            self.send_response(200)
            self.send_header("Content-Length", "0")
            self.end_headers()
        finally:
            if temporary:
                temporary.unlink(missing_ok=True)
            self.server.finish(dict(method="PUT", path=self.path, status=status,
                bytes=count, start=started, end=time.monotonic()))


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--root", type=Path, required=True)
    parser.add_argument("--port", type=int, default=8080)
    parser.add_argument("--mib-per-second", type=float, default=0)
    parser.add_argument("--latency-ms", type=float, default=0)
    parser.add_argument("--requests", type=Path, required=True)
    args = parser.parse_args()
    server = CacheServer(args.root, args.port, args.mib_per_second, args.latency_ms)
    print(f"http://127.0.0.1:{server.server_port}", flush=True)
    try:
        server.serve_forever()
    except KeyboardInterrupt:
        pass
    finally:
        server.server_close()
        args.requests.parent.mkdir(parents=True, exist_ok=True)
        args.requests.write_text(json.dumps(server.snapshot(), indent=2) + "\n")


if __name__ == "__main__":
    main()
