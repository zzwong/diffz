#!/usr/bin/env python3
"""Install one pinned host Kache atomically, with a lock shared by worktrees."""

import argparse
import fcntl
import os
from pathlib import Path
import re
import stat
import subprocess
import sys
import tempfile


def fail(message):
    raise RuntimeError(message)


def verify_binary(binary, version, *, owned=False):
    if owned and (binary.is_symlink() or binary.parent.is_symlink()):
        fail(f"refusing symlink in managed Kache installation: {binary}")
    if not binary.is_file() or not os.access(binary, os.X_OK):
        fail(f"Kache executable is missing or not executable: {binary}")
    result = subprocess.run(
        [str(binary), "--version"], capture_output=True, text=True, timeout=15
    )
    if result.returncode or result.stdout.strip() != f"kache {version}":
        fail(f"expected kache {version} at {binary}; existing files were not replaced")


def validate_install(root, version):
    if root.is_symlink() or not root.is_dir():
        fail(f"refusing non-directory or symlink Kache root: {root}")
    verify_binary(root / "bin" / "kache", version, owned=True)


def bootstrap(args):
    if not re.fullmatch(r"[0-9]+\.[0-9]+\.[0-9]+(?:[-+][A-Za-z0-9.-]+)?", args.version):
        fail("Kache version must be an exact version")
    requested_root = Path(os.path.abspath(args.root))
    binary = Path(os.path.abspath(args.bin))

    # An explicit external executable is read-only: never install into its root.
    if binary != requested_root / "bin" / "kache":
        verify_binary(binary, args.version)
        return 0

    if not re.fullmatch(r"[A-Za-z0-9_]+(?:-[A-Za-z0-9_]+)+", args.host):
        fail("cannot determine the Rust host; check the pinned rustc on PATH")
    if requested_root.is_symlink():
        fail(f"refusing symlink Kache root: {requested_root}")
    # Resolve parent aliases before choosing the lock, including an XDG symlink.
    root = requested_root.parent.resolve() / requested_root.name
    if root == root.parent:
        fail("the filesystem root cannot be a Kache installation")
    root.parent.mkdir(parents=True, exist_ok=True)
    lock_path = root.parent / f".{root.name}.install.lock"
    flags = os.O_CREAT | os.O_RDWR | os.O_NOFOLLOW
    lock_fd = os.open(lock_path, flags, 0o600)
    with os.fdopen(lock_fd, "a") as lock:
        info = os.fstat(lock.fileno())
        if not stat.S_ISREG(info.st_mode) or info.st_uid != os.getuid() or info.st_nlink != 1:
            fail(f"refusing non-owned or linked Kache lock: {lock_path}")
        # Keep the lock inode permanently: unlinking a lock lets waiters diverge.
        fcntl.flock(lock, fcntl.LOCK_EX)
        if root.exists() or root.is_symlink():
            validate_install(root, args.version)
            return 0

        host = subprocess.run(
            ["rustc", "-vV"], check=True, text=True, capture_output=True, timeout=15
        )
        if f"host: {args.host}" not in host.stdout.splitlines():
            fail("selected rustc host changed; refusing an inconsistent tool namespace")

        # Staging and publication share a filesystem. TemporaryDirectory removes
        # only this invocation's staging tree on ordinary failure/interruption.
        # SIGKILL may leave a hidden stage, but can never select its partial tool.
        with tempfile.TemporaryDirectory(
            prefix=f".{root.name}.install-", dir=root.parent
        ) as temporary:
            stage = Path(temporary)
            install = stage / "install"
            env = os.environ.copy()
            env.update(
                RUSTC_WRAPPER="",
                RUSTC_WORKSPACE_WRAPPER="",
                CARGO_TARGET_DIR=str(stage / "target"),
                CARGO_BUILD_BUILD_DIR=str(stage / "build"),
            )
            command = [
                "cargo", "install", "--locked", "--version", args.version,
                "--target", args.host, "--root", str(install), "kache",
            ]
            print(f"Installing kache {args.version} for {args.host} into {root}", flush=True)
            result = subprocess.run(command, env=env)
            if result.returncode:
                return result.returncode if result.returncode > 0 else 128 - result.returncode
            validate_install(install, args.version)
            if root.exists() or root.is_symlink():
                fail(f"Kache destination appeared during installation: {root}")
            os.rename(install, root)
    return 0


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--version", required=True)
    parser.add_argument("--host", required=True)
    parser.add_argument("--root", required=True)
    parser.add_argument("--bin", required=True)
    args = parser.parse_args()
    try:
        return bootstrap(args)
    except KeyboardInterrupt:
        print("Kache bootstrap interrupted; no partial tool was published", file=sys.stderr)
        return 130
    except (OSError, RuntimeError, subprocess.SubprocessError) as error:
        print(f"Kache bootstrap failed: {error}", file=sys.stderr)
        return 1


if __name__ == "__main__":
    sys.exit(main())
