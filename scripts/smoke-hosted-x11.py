#!/usr/bin/env python3
"""Bounded GitHub-hosted Xvfb handoff smoke.

No installation, Cargo invocation, existing-display connection, WM, or focus
claim. The private process-tree guardian is sampled, not a security sandbox.
SIGKILL/kernel loss cannot be cleaned up by a dead Python process; the existing
hosted job's lifecycle remains the final boundary. Do not run outside admission.
"""
import argparse
import ctypes
import hashlib
import json
import os
from pathlib import Path
import re
import resource
import selectors
import shutil
import signal
import socket
import stat
import struct
import subprocess
import sys
import tempfile
import time

SELF = Path(__file__).resolve()
OBSERVER = SELF.with_name("observe-x11-handoff.py")
OBSERVER_SHA = "846eb669480471e4a16bf92278367a9312e295e0fbd0f82cbf3f14825f106ab1"
WORK_SECONDS, TOTAL_SECONDS, SAMPLE_SECONDS = 230, 240, 0.1
DISPLAY_START_SECONDS = 30
RSS_CAP, RESOURCE_FLOOR = 2 * 1024**3, 2_700_000_000
LOG_CAP, DATA_CAP, EVIDENCE_CAP = 8 * 1024**2, 64 * 1024**2, 12 * 1024**2
PACKAGES = ("xvfb", "xauth", "libx11-6", "libegl1", "libegl-mesa0", "libglvnd0",
            "libgl1-mesa-dri", "libgbm1", "mesa-libgallium", "libllvm20", "libvulkan1")
STOP = None


def need(condition, message):
    if not condition:
        raise RuntimeError(message)


def digest(path):
    with Path(path).open("rb") as stream:
        return hashlib.file_digest(stream, "sha256").hexdigest()


def write_json(path, value):
    with Path(path).open("x") as stream:
        json.dump(value, stream, indent=2, sort_keys=True)
        stream.write("\n")


def metadata_command(argv, cwd=None):
    # No secrets, auth cookies, inherited display, proxy, or loader settings.
    return subprocess.run(argv, cwd=cwd, env={"PATH": "/usr/bin:/bin", "LC_ALL": "C"},
                          stdin=subprocess.DEVNULL, stdout=subprocess.PIPE,
                          stderr=subprocess.PIPE, check=True, timeout=10).stdout.decode().strip()


def hosted_inputs():
    need(os.environ.get("GITHUB_ACTIONS") == "true" and
         os.environ.get("RUNNER_ENVIRONMENT") == "github-hosted" and
         os.environ.get("RUNNER_OS") == "Linux", "requires the existing GitHub-hosted Linux job")
    need(os.uname().machine == "x86_64" and os.getuid() != 0 and
         os.getuid() == os.geteuid(), "requires unprivileged x86-64 Linux")
    os_release = dict(line.split("=", 1) for line in Path("/etc/os-release").read_text().splitlines()
                      if "=" in line)
    need(os_release.get("ID") == "ubuntu" and os_release.get("VERSION_ID") == '"24.04"',
         "requires the existing Ubuntu 24.04 job")
    workspace = Path(os.environ["GITHUB_WORKSPACE"]).resolve(strict=True)
    temporary = Path(os.environ["RUNNER_TEMP"]).resolve(strict=True)
    need(temporary.is_dir() and temporary.stat().st_uid == os.getuid(), "job temp is not owned")
    need(sys.flags.isolated and sys.flags.no_site and sys.flags.dont_write_bytecode,
         "invoke with Python -I -S -B")
    return workspace, temporary


def source_identity(workspace):
    checkout = metadata_command(["git", "rev-parse", "HEAD"], workspace)
    head = os.environ["DIFFZ_EVENT_HEAD"]
    need(re.fullmatch(r"[0-9a-f]{40}", head) and re.fullmatch(r"[0-9a-f]{40}", checkout),
         "invalid commit identity")
    commit = metadata_command(["git", "cat-file", "-p", checkout], workspace)
    parents = re.findall(r"^parent ([0-9a-f]{40})$", commit.split("\n\n", 1)[0], re.M)
    need(checkout == head or head in parents, "checkout is neither event head nor its direct PR merge")
    need(not metadata_command(["git", "diff", "--name-only", "HEAD", "--"], workspace),
         "tracked checkout changed since checkout")
    return {"checkout_sha": checkout, "event_head_sha": head,
            "checkout_tree": metadata_command(["git", "rev-parse", "HEAD^{tree}"], workspace),
            "checkout_is_raw_head": checkout == head,
            "cargo_lock_sha256": digest(workspace / "Cargo.lock")}


def record_build():
    workspace, temporary = hosted_inputs()
    binary = workspace / "target/debug/diffz"
    need(binary.is_file() and not binary.is_symlink() and os.access(binary, os.X_OK),
         "cargo test did not produce an ordinary executable; do not rebuild here")
    write_json(temporary / "diffz-native-build.json", {
        **source_identity(workspace), "binary_sha256": digest(binary),
        "binary_bytes": binary.stat().st_size,
        "build_command": "cargo test --locked --workspace --all-features",
        "run_id": os.environ["GITHUB_RUN_ID"], "run_attempt": os.environ["GITHUB_RUN_ATTEMPT"]})


def process_table():
    rows = {}
    for path in Path("/proc").iterdir():
        if path.name.isdecimal():
            try:
                fields = (path / "stat").read_text().rsplit(")", 1)[1].split()
                rows[int(path.name)] = {"ppid": int(fields[1]), "start_ticks": int(fields[19]),
                    "state": fields[0], "uid": path.stat().st_uid,
                    "rss": int(fields[21]) * os.sysconf("SC_PAGE_SIZE")}
            except (FileNotFoundError, ProcessLookupError):
                pass
    return rows


def descendants(rows):
    found = {os.getpid()}
    while True:
        extended = found | {pid for pid, row in rows.items() if row["ppid"] in found}
        if extended == found:
            return {pid: rows[pid] for pid in found if pid != os.getpid() and pid in rows}
        found = extended


def namespace_ids():
    return {name: os.readlink("/proc/self/ns/" + name) for name in ("pid", "mnt", "net")}


def resources(root):
    available = next(int(line.split()[1]) * 1024 for line in Path("/proc/meminfo").read_text().splitlines()
                     if line.startswith("MemAvailable:"))
    return {"mem_available": available, "disk_free": shutil.disk_usage(root).free}


def prctl(option, value):
    libc = ctypes.CDLL("libc.so.6", use_errno=True)
    libc.prctl.argtypes = [ctypes.c_int, ctypes.c_ulong, ctypes.c_ulong, ctypes.c_ulong, ctypes.c_ulong]
    libc.prctl.restype = ctypes.c_int
    need(libc.prctl(option, value, 0, 0, 0) == 0, "prctl failed")


def child_setup(death_signal=signal.SIGKILL):
    parent = os.getppid()
    prctl(1, death_signal)  # Direct-child protection, not a tree guarantee.
    if parent == 1 or os.getppid() != parent:
        os._exit(125)
    resource.setrlimit(resource.RLIMIT_CORE, (0, 0))
    resource.setrlimit(resource.RLIMIT_FSIZE, (LOG_CAP, LOG_CAP))


def auth_bytes(cookie, number):
    # FamilyWild matches this private record; it does NOT bypass cookie auth.
    fields = (b"", number.encode("ascii"), b"MIT-MAGIC-COOKIE-1", cookie)
    return struct.pack("!H", 65535) + b"".join(struct.pack("!H", len(x)) + x for x in fields)


def authenticate(endpoint, cookie, server_pid, server_start):
    def handshake(key):
        with socket.socket(socket.AF_UNIX, socket.SOCK_STREAM) as connection:
            connection.settimeout(2)
            connection.connect(str(endpoint))
            peer_pid, peer_uid, _ = struct.unpack("3i", connection.getsockopt(socket.SOL_SOCKET,
                                                        socket.SO_PEERCRED, struct.calcsize("3i")))
            need(peer_pid == server_pid and peer_uid == os.getuid(), "X socket peer is not owned Xvfb")
            protocol = b"MIT-MAGIC-COOKIE-1"
            packet = struct.pack("<BBHHHHH", ord("l"), 0, 11, 0, len(protocol), len(key), 0)
            for value in (protocol, key):
                packet += value + b"\0" * (-len(value) % 4)
            connection.sendall(packet)
            answer = b""
            while len(answer) < 8:
                part = connection.recv(8 - len(answer))
                need(part, "incomplete X11 authentication response")
                answer += part
            return answer[0]
    need(handshake(b"\0" * 16) == 0, "private X server accepted a wrong cookie")
    need(handshake(cookie) == 1, "private X server rejected its cookie")
    need(process_table().get(server_pid, {}).get("start_ticks") == server_start,
         "X server identity changed during authenticated handshake")


def unix_listeners():
    result = []
    for line in Path("/proc/net/unix").read_text().splitlines()[1:]:
        fields = line.split(maxsplit=7)
        if int(fields[3], 16) & 0x10000:
            result.append({"inode": fields[6], "path": fields[7] if len(fields) > 7 else ""})
    return result


def server_listeners(pid, start, endpoint):
    inodes = set()
    for fd in Path("/proc", str(pid), "fd").iterdir():
        try:
            match = re.fullmatch(r"socket:\[([0-9]+)\]", os.readlink(fd))
            if match:
                inodes.add(match.group(1))
        except FileNotFoundError:
            pass
    tcp = []
    for protocol in ("tcp", "tcp6"):
        path = Path("/proc/net") / protocol
        if not path.exists():
            continue
        for line in path.read_text().splitlines()[1:]:
            fields = line.split()
            if fields[3] == "0A" and fields[9] in inodes:
                tcp.append({"protocol": protocol, "inode": fields[9]})
    unix = [entry for entry in unix_listeners() if entry["inode"] in inodes]
    need(not tcp and unix and all(item["path"] in (str(endpoint), "@" + str(endpoint)) for item in unix),
         "unexpected owned Xvfb listener")
    need(process_table().get(pid, {}).get("start_ticks") == start, "Xvfb identity changed across listeners")
    return {"tcp": tcp, "unix": unix}


def wait_display_number(read_fd, server_handle, work_deadline):
    """Read one bounded displayfd frame without reaping or signalling Xvfb.

    Startup shares the original work budget; it cannot extend the guardian's
    deadline. Keep the pipe nonblocking even after readiness, and accumulate
    fragmented writes rather than treating one read as a complete response.
    """
    started = time.monotonic()
    deadline = min(started + DISPLAY_START_SECONDS, work_deadline)
    answer = bytearray()
    reason = "deadline"
    try:
        os.set_blocking(read_fd, False)
        with selectors.DefaultSelector() as selector:
            selector.register(read_fd, selectors.EVENT_READ, "display")
            selector.register(server_handle, selectors.EVENT_READ, "exit")
            while True:
                remaining = deadline - time.monotonic()
                need(remaining > 0, "Xvfb displayfd deadline")
                events = selector.select(remaining)
                # Never accept a late reply, including a delayed selector return.
                need(time.monotonic() < deadline, "Xvfb displayfd deadline")
                if any(key.data == "exit" for key, _ in events):
                    reason = "server-exit"
                    raise RuntimeError("Xvfb exited before display readiness")
                if not events:
                    continue
                try:
                    chunk = os.read(read_fd, 7 - len(answer))
                except BlockingIOError:
                    continue
                if not chunk:
                    reason = "eof"
                    raise RuntimeError("Xvfb displayfd EOF before readiness")
                answer.extend(chunk)
                if len(answer) > 6 or answer == b"\n" or not re.fullmatch(rb"[0-9]{0,5}\n?", answer):
                    reason = "malformed"
                    raise RuntimeError("Xvfb displayfd malformed response")
                if answer.endswith(b"\n"):
                    need(time.monotonic() < deadline, "Xvfb displayfd deadline")
                    reason = "ready"
                    return bytes(answer)
    finally:
        # Safe bounded metadata only; the existing worker.log is retained on
        # failure. Do not expose auth cookies or invent successful readiness.
        print(json.dumps({"xvfb_startup": reason, "response_bytes": len(answer),
                          "elapsed_seconds": time.monotonic() - started,
                          "startup_limit_seconds": DISPLAY_START_SECONDS}), flush=True)


def worker(root, parent):
    need(os.getppid() == parent, "worker parent changed")
    preflight = json.loads((root / "preflight.json").read_text())
    need(root.resolve() == root and root.stat().st_uid == os.getuid() and
         stat.S_IMODE(root.stat().st_mode) == 0o700 and preflight["guardian_pid"] == parent and
         namespace_ids() == preflight["namespace_ids"],
         "invalid private root")
    need(digest(OBSERVER) == OBSERVER_SHA and digest(root / "diffz") == preflight["binary_sha256"] and
         digest(SELF) == preflight["guardian_sha256"] and
         digest(preflight["x11_library"]) == preflight["x11_sha256"] and
         digest("/usr/bin/Xvfb") == preflight["xvfb_sha256"], "worker inputs changed")
    need(time.monotonic() < preflight["work_deadline_monotonic"], "worker admission expired")
    for name in ("home", "tmp", "runtime", "config", "cache", "data", "state"):
        (root / name).mkdir(mode=0o700)
    env = {"PATH": "/usr/bin:/bin", "LC_ALL": "C", "HOME": str(root / "home"),
           "TMPDIR": str(root / "tmp"), "XDG_RUNTIME_DIR": str(root / "runtime"),
           "XDG_CONFIG_HOME": str(root / "config"), "XDG_CACHE_HOME": str(root / "cache"),
           "XDG_DATA_HOME": str(root / "data"), "XDG_STATE_HOME": str(root / "state"),
           "LIBGL_ALWAYS_SOFTWARE": "true", "GALLIUM_DRIVER": "llvmpipe", "EGL_PLATFORM": "x11",
           "EGL_LOG_LEVEL": "debug", "MESA_SHADER_CACHE_MAX_SIZE": "16M", "NO_AT_BRIDGE": "1"}
    # No Wayland, activation/startup token, D-Bus session, credentials, loader/ICD
    # redirection, inherited display, or generic WGPU_BACKEND in this environment.
    server, server_handle = None, None
    try:
        cookie = os.urandom(16)
        (root / "server.auth").write_bytes(auth_bytes(cookie, "0"))
        read_fd, write_fd = os.pipe()
        try:
            need(time.monotonic() < preflight["work_deadline_monotonic"], "display admission expired")
            with (root / "xvfb.log").open("xb") as log:
                server = subprocess.Popen(["/usr/bin/Xvfb", "-displayfd", str(write_fd),
                    "-screen", "0", "1280x800x24", "-dpi", "96", "-nolisten", "tcp", "-noreset",
                    "-auth", str(root / "server.auth")], pass_fds=(write_fd,), env=env, cwd=root,
                    stdin=subprocess.DEVNULL, stdout=log, stderr=log,
                    start_new_session=True, preexec_fn=lambda: child_setup(signal.SIGTERM))
            server_handle = os.pidfd_open(server.pid, 0)
            os.close(write_fd)
            write_fd = None
            answer = wait_display_number(read_fd, server_handle,
                                         preflight["work_deadline_monotonic"])
        finally:
            os.close(read_fd)
            if write_fd is not None:
                os.close(write_fd)
        need(re.fullmatch(rb"[0-9]{1,5}\n", answer) and server.poll() is None, "Xvfb did not become ready")
        number = answer[:-1].decode("ascii")
        endpoint = Path("/tmp/.X11-unix/X" + number)
        info = endpoint.lstat()
        need(stat.S_ISSOCK(info.st_mode) and info.st_uid == os.getuid(), "unexpected X socket")
        start = process_table()[server.pid]["start_ticks"]
        authenticate(endpoint, cookie, server.pid, start)
        listeners = server_listeners(server.pid, start, endpoint)
        (root / "client.auth").write_bytes(auth_bytes(cookie, number))
        del cookie
        env.update(DISPLAY=":" + number, XAUTHORITY=str(root / "client.auth"))
        write_json(root / "display-ready.json", {"display": env["DISPLAY"], "pid": server.pid,
            "start_ticks": start, "uid": os.getuid(), "socket": str(endpoint),
            "socket_device": info.st_dev, "socket_inode": info.st_ino,
            "authenticated_owned_peer": True, "wrong_cookie_rejected": True,
            "tcp_disabled_by_argv": True, "owned_listeners": listeners, "window_manager": None,
            "focus_verified": False, "renderer_verified": False})
        need(time.monotonic() < preflight["work_deadline_monotonic"], "observer admission expired")
        with (root / "observer.log").open("xb") as log:
            observer = subprocess.Popen([sys.executable, "-I", "-S", "-B", str(OBSERVER),
                "--run-root", str(root), "--binary", str(root / "diffz"),
                "--x11-library", preflight["x11_library"]], env=env, cwd=root,
                stdin=subprocess.DEVNULL, stdout=log, stderr=log,
                start_new_session=True, preexec_fn=child_setup)
            code = observer.wait()
        need(server.poll() is None, "Xvfb exited before observer completion")
        return code
    finally:
        if server is not None and server_handle is not None:
            try:
                try:
                    signal.pidfd_send_signal(server_handle, signal.SIGTERM, None, 0)
                except ProcessLookupError:
                    pass
                try:
                    server.wait(timeout=4)
                except subprocess.TimeoutExpired:
                    signal.pidfd_send_signal(server_handle, signal.SIGKILL, None, 0)
                    server.wait(timeout=4)
            finally:
                os.close(server_handle)



def reap(rows, child):
    if child is not None:
        child.poll()
    for pid in rows:
        if child is None or pid != child.pid:
            try:
                os.waitpid(pid, os.WNOHANG)
            except (ChildProcessError, ProcessLookupError):
                pass


def kernel_children_reaped(child):
    # Let Popen retain its exact status before a wait-any drain of adopted children.
    if child is not None and child.poll() is None:
        return False
    while True:
        try:
            pid, _status = os.waitpid(-1, os.WNOHANG)
            if pid == 0:
                return False
        except ChildProcessError:
            return True


def signal_owned(rows, number):
    for pid, row in rows.items():
        try:
            handle = os.pidfd_open(pid, 0)
        except ProcessLookupError:
            continue
        try:
            current = descendants(process_table()).get(pid)
            if current is not None and current["start_ticks"] == row["start_ticks"] and current["uid"] == os.getuid():
                signal.pidfd_send_signal(handle, number, None, 0)
        except ProcessLookupError:
            pass
        finally:
            os.close(handle)


def observe_mappings(rows, binary, records):
    for pid, row in rows.items():
        try:
            proc = Path("/proc") / str(pid)
            if (proc / "exe").resolve() != binary:
                continue
            paths = set()
            for line in (proc / "maps").read_text().splitlines():
                fields = line.split(maxsplit=5)
                if len(fields) == 6 and re.search(r"/(libEGL|libgallium|libGL|libvulkan|libLLVM|swrast_dri)", fields[5]):
                    paths.add(fields[5])
            if process_table().get(pid, {}).get("start_ticks") == row["start_ticks"]:
                records[(pid, row["start_ticks"])] = sorted(paths)
        except (FileNotFoundError, ProcessLookupError):
            pass


def data_bytes(root):
    total = 0
    for directory, _, files in os.walk(root, followlinks=False):
        for name in files:
            path = Path(directory) / name
            if path == root / "diffz" or path.is_symlink():
                continue
            try:
                info = path.stat(follow_symlinks=False)
                if stat.S_ISREG(info.st_mode):
                    total += info.st_size
            except FileNotFoundError:
                pass
    return total


def export_evidence(root, evidence):
    # Only these files, never state databases, HOME, binary, environment or auth.
    allowed = {"preflight.json": 128 * 1024, "display-ready.json": 32 * 1024,
               "observer-result.json": 128 * 1024, "cleanup.json": 128 * 1024,
               "samples.jsonl": 1024 * 1024,
               **{n + ".png": 3 * 1024**2 for n in ("reference-F02", "before-F01", "after-F02")}}
    cookie_path = root / "server.auth"
    cookie = cookie_path.read_bytes()[-16:] if cookie_path.is_file() else b""
    total, manifest = 0, []
    for name, limit in allowed.items():
        source = root / name
        if not source.exists():
            continue
        need(source.is_file() and not source.is_symlink() and source.stat().st_size <= limit,
             "evidence file type/size invalid: " + name)
        payload = source.read_bytes()
        if name.endswith(".png"):
            need(payload.startswith(b"\x89PNG\r\n\x1a\n"), "invalid PNG")
        total += len(payload)
        need(total <= EVIDENCE_CAP, "evidence size cap")
        (evidence / name).write_bytes(payload)
        manifest.append({"file": name, "bytes": len(payload), "sha256": hashlib.sha256(payload).hexdigest()})
    for name in ("worker.log", "xvfb.log", "observer.log", "reference-state/diffz.log", "target-state/diffz.log"):
        source = root / name
        if not source.is_file() or source.is_symlink():
            continue
        with source.open("rb") as stream:
            payload = stream.read(128 * 1024)
        if cookie:
            for secret in (cookie, cookie.hex().encode(), cookie.hex().upper().encode()):
                payload = payload.replace(secret, b"[REDACTED]")
        total += len(payload)
        need(total <= EVIDENCE_CAP, "evidence size cap")
        destination = name.replace("/", "-")
        (evidence / destination).write_bytes(payload)
        manifest.append({"file": destination, "bytes": len(payload),
                         "truncated": source.stat().st_size > len(payload),
                         "sha256": hashlib.sha256(payload).hexdigest()})
    write_json(evidence / "manifest.json", {"files": manifest, "payload_bytes": total,
        "retention_days": 3, "content_text_independently_read": False,
        "focus_verified": False, "wayland_token_verified": False})


def guard():
    workspace, temporary = hosted_inputs()
    evidence = temporary / "diffz-x11-evidence"
    evidence.mkdir(mode=0o700)  # Never mix stale attempts; existing directory fails closed.
    root = Path(tempfile.mkdtemp(prefix="dz-", dir=temporary))
    child, peak, reason, returncode = None, 0, None, None
    history, mappings = {}, {}
    cleanup_delayed, cleanup_errors = False, []
    started = time.monotonic()
    try:
        need(len(os.fsencode(root / "reference-state/handoff.sock")) < 104, "job temp socket path too long")
        need(digest(OBSERVER) == OBSERVER_SHA, "frozen observer changed")
        build = json.loads((temporary / "diffz-native-build.json").read_text())
        for key, value in source_identity(workspace).items():
            need(build[key] == value, "build provenance changed: " + key)
        need(build["run_id"] == os.environ["GITHUB_RUN_ID"] and
             build["run_attempt"] == os.environ["GITHUB_RUN_ATTEMPT"], "build belongs to another attempt")
        source = workspace / "target/debug/diffz"
        need(source.is_file() and not source.is_symlink() and os.access(source, os.X_OK), "invalid test binary")
        need(digest(source) == build["binary_sha256"], "existing test binary changed; do not rebuild")
        need(all(Path(p).is_file() for p in ("/usr/bin/Xvfb", "/usr/lib/x86_64-linux-gnu/libEGL.so.1",
             "/usr/lib/x86_64-linux-gnu/libEGL_mesa.so.0", "/usr/share/glvnd/egl_vendor.d/50_mesa.json")),
             "missing supported Xvfb/EGL prerequisites; no install/retry fallback")
        library = Path("/usr/lib/x86_64-linux-gnu/libX11.so.6").resolve(strict=True)
        need(hasattr(os, "pidfd_open") and hasattr(signal, "pidfd_send_signal"), "PIDFD APIs required")
        check = os.pidfd_open(os.getpid(), 0)
        try:
            signal.pidfd_send_signal(check, 0, None, 0)
        finally:
            os.close(check)
        prctl(36, 1)  # PR_SET_CHILD_SUBREAPER: reap detached Diffz descendants.
        cpus = sorted(os.sched_getaffinity(0))[:2]
        need(len(cpus) == 2 and not descendants(process_table()), "fresh two-CPU process tree required")
        os.sched_setaffinity(0, cpus)
        initial = resources(temporary)
        need(min(initial.values()) >= RESOURCE_FLOOR, "insufficient memory/disk headroom")
        shutil.copyfile(source, root / "diffz")
        (root / "diffz").chmod(0o700)
        need(digest(root / "diffz") == build["binary_sha256"], "binary copy changed")
        need(min(resources(temporary).values()) >= RESOURCE_FLOOR, "insufficient headroom after copy")
        write_json(root / "preflight.json", {**build, "guardian_pid": os.getpid(),
            "observer_sha256": OBSERVER_SHA, "guardian_sha256": digest(SELF),
            "namespace_ids": namespace_ids(), "work_deadline_monotonic": started + WORK_SECONDS,
            "x11_library": str(library), "x11_sha256": digest(library),
            "xvfb_sha256": digest("/usr/bin/Xvfb"), "affinity": cpus, "resources": initial,
            "image": {key: os.environ.get(key) for key in ("ImageOS", "ImageVersion", "RUNNER_ARCH")},
            "packages": metadata_command(["dpkg-query", "-W", "-f=${Package}=${Version}\n", *PACKAGES]),
            "limits": {"work_seconds": WORK_SECONDS, "total_seconds": TOTAL_SECONDS,
                       "sample_seconds": SAMPLE_SECONDS, "tree_rss_bytes": RSS_CAP,
                       "memory_disk_floor_bytes": RESOURCE_FLOOR, "private_data_bytes": DATA_CAP},
            "software_adapter_verified": False, "focus_verified": False})
        need(not STOP and time.monotonic() - started < WORK_SECONDS, "preflight canceled or expired")
        with (root / "worker.log").open("xb") as log:
            child = subprocess.Popen([sys.executable, "-I", "-S", "-B", str(SELF), "--worker", str(root),
                "--parent", str(os.getpid())], env={"PATH": "/usr/bin:/bin", "LC_ALL": "C"}, cwd=root,
                stdin=subprocess.DEVNULL, stdout=log, stderr=log, start_new_session=True, preexec_fn=child_setup)
        with (root / "samples.jsonl").open("x") as samples:
            tick = 0
            while True:
                table = process_table()
                rows = descendants(table)
                for pid, row in rows.items():
                    history[(pid, row["start_ticks"])] = row
                reap(rows, child)
                rss = table[os.getpid()]["rss"] + sum(row["rss"] for row in rows.values())
                peak = max(peak, rss)
                state = resources(root)
                elapsed = time.monotonic() - started
                if tick % 10 == 0:
                    observe_mappings(rows, root / "diffz", mappings)
                    samples.write(json.dumps({"elapsed": round(elapsed, 2), "tree_rss": rss,
                                              "descendants": len(rows), **state}) + "\n")
                    samples.flush()
                returncode = child.poll()
                if STOP or elapsed >= WORK_SECONDS or rss > RSS_CAP or min(state.values()) < RESOURCE_FLOOR or data_bytes(root) > DATA_CAP:
                    reason = STOP or ("work-deadline" if elapsed >= WORK_SECONDS else "resource-limit")
                    break
                if returncode is not None:
                    reason = "observer-terminal"
                    break
                tick += 1
                time.sleep(SAMPLE_SECONDS)
    except Exception as error:
        reason = type(error).__name__ + ": " + str(error)
    finally:
        graceful = time.monotonic() + 2
        while True:
            try:
                rows = descendants(process_table())
                reap(rows, child)
                rows = descendants(process_table())
                if not rows and kernel_children_reaped(child):
                    break
                signal_owned(rows, signal.SIGTERM if time.monotonic() < graceful else signal.SIGKILL)
            except Exception as error:
                if str(error) not in cleanup_errors:
                    cleanup_errors.append(str(error))
            if time.monotonic() - started >= TOTAL_SECONDS and not cleanup_delayed:
                cleanup_delayed = True
                try:
                    write_json(evidence / "cleanup-blocked.json", {"phase_a_passed": False,
                               "reason": reason, "cleanup_errors": cleanup_errors})
                except Exception:
                    pass
            # Never report success or abandon owned descendants because cleanup
            # observation/signaling failed. Job timeout remains the final boundary.
            time.sleep(1 if cleanup_delayed else SAMPLE_SECONDS)
    passed = False
    result = {}
    guard_error = None
    try:
        elapsed = time.monotonic() - started
        observer = root / "observer-result.json"
        need(not observer.is_file() or observer.stat().st_size <= 128 * 1024, "observer receipt too large")
        result = json.loads(observer.read_text()) if observer.is_file() else {}
        passed = (reason == "observer-terminal" and returncode == 0 and result.get("phase_a_passed") is True
                  and result.get("same_pid_starttime_xid") is True and result.get("matches_independent_f02_render") is True
                  and not cleanup_errors and not cleanup_delayed and elapsed < TOTAL_SECONDS)
        # No global endpoint/lock unlink, including an owned server's leftovers.
        display = root / "display-ready.json"
        endpoint_absent, lock_absent, unix_absent = False, False, False
        if display.is_file():
            receipt = json.loads(display.read_text())
            endpoint_absent = not os.path.lexists(receipt["socket"])
            lock_absent = not os.path.lexists("/tmp/.X" + receipt["display"][1:] + "-lock")
            old_inodes = {entry["inode"] for entry in receipt["owned_listeners"]["unix"]}
            unix_absent = not any(entry["inode"] in old_inodes or entry["path"] in
                                 (receipt["socket"], "@" + receipt["socket"]) for entry in unix_listeners())
            passed = passed and receipt["authenticated_owned_peer"] and receipt["wrong_cookie_rejected"] and not receipt["owned_listeners"]["tcp"]
        passed = passed and endpoint_absent and lock_absent and unix_absent
        preflight_path = root / "preflight.json"
        input_checks = {}
        if preflight_path.is_file():
            preflight = json.loads(preflight_path.read_text())
            input_checks = {"guardian": digest(SELF) == preflight["guardian_sha256"],
                "observer": digest(OBSERVER) == OBSERVER_SHA,
                "copy": digest(root / "diffz") == preflight["binary_sha256"],
                "test_binary": digest(workspace / "target/debug/diffz") == preflight["binary_sha256"],
                "x11": digest(preflight["x11_library"]) == preflight["x11_sha256"],
                "xvfb": digest("/usr/bin/Xvfb") == preflight["xvfb_sha256"],
                "namespaces": namespace_ids() == preflight["namespace_ids"],
                "checkout": all(preflight[k] == v for k, v in source_identity(workspace).items())}
        passed = bool(passed and input_checks and all(input_checks.values()) and not STOP)
        write_json(root / "cleanup.json", {"observations_passed_before_export": passed, "reason": reason,
            "worker_exit": returncode, "elapsed_seconds": elapsed, "peak_tree_rss": peak,
            "descendants_remaining": descendants(process_table()), "endpoint_absent": endpoint_absent,
            "x_lock_absent": lock_absent, "unix_listeners_absent": unix_absent, "inputs_unchanged": input_checks,
            "cleanup_delayed": cleanup_delayed, "cleanup_errors": cleanup_errors,
            "process_history": [{"pid": pid, **row} for (pid, _), row in sorted(history.items())],
            "renderer_mappings": [{"pid": pid, "start_ticks": start, "paths": paths}
                                  for (pid, start), paths in sorted(mappings.items())],
            "software_adapter_verified": False, "focus_verified": False,
            "wayland_token_verified": False, "content_text_independently_read": False})
        export_evidence(root, evidence)
    except Exception as error:
        passed = False
        guard_error = type(error).__name__ + ": " + str(error)
        write_json(evidence / "guard-error.json", {"phase_a_passed": False,
                   "error": guard_error})
    finally:
        for name in ("server.auth", "client.auth"):
            path = root / name
            if path.is_file() and not path.is_symlink():
                path.unlink()
        # Only our own mkdtemp root, after all owned processes have been reaped.
        shutil.rmtree(root)
    elapsed = time.monotonic() - started
    final_resources = resources(temporary)
    passed = bool(passed and not STOP and elapsed < TOTAL_SECONDS and
                  min(final_resources.values()) >= RESOURCE_FLOOR and kernel_children_reaped(child))
    write_json(evidence / "final.json", {"phase_a_passed": passed, "elapsed_seconds": elapsed,
        "resources": final_resources, "private_root_removed": not root.exists(),
        "focus_verified": False, "software_adapter_verified": False,
        "content_text_independently_read": False})
    print(json.dumps({"phase_a_passed": passed, "evidence": str(evidence),
                      "reason": reason, "worker_exit": returncode,
                      "observer_error": result.get("error"), "guard_error": guard_error,
                      "focus_verified": False, "software_adapter_verified": False}))
    return 0 if passed else 1



def stopped(number, _frame):
    global STOP
    STOP = "signal-" + str(number)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--record-build", action="store_true")
    parser.add_argument("--worker", type=Path)
    parser.add_argument("--parent", type=int)
    args = parser.parse_args()
    os.umask(0o077)
    if args.record_build:
        record_build()
        return 0
    if args.worker:
        def interrupted(number, _frame):
            raise InterruptedError("worker signal-" + str(number))
        for number in (signal.SIGTERM, signal.SIGINT, signal.SIGHUP, signal.SIGQUIT):
            signal.signal(number, interrupted)
        return worker(args.worker, args.parent)
    for number in (signal.SIGTERM, signal.SIGINT, signal.SIGHUP, signal.SIGQUIT):
        signal.signal(number, stopped)
    return guard()


if __name__ == "__main__":
    raise SystemExit(main())
