#!/usr/bin/env python3
"""Identify the prepared Debian/Ubuntu builder before restoring Cargo artifacts.

Run inside the same disposable container that will compile the package, after
apt and rustup have finished. This is an observed native-environment identity,
not a claim that a mutable distribution image is a hermetic build toolchain.
Only the digest goes to stdout; the allowlisted evidence is saved separately.
"""

import argparse
import hashlib
import json
import os
from pathlib import Path
import re
import shutil
import subprocess
import sys


SCHEMA_VERSION = 1
TOOLS = ("rustc", "cargo", "cc", "c++", "clang", "clang++", "ld", "ar", "cmake", "pkg-config")
PACKAGE_FORMAT = "${binary:Package}\t${Version}\t${Architecture}\t${db:Status-Status}\n"


def capture(argv):
    try:
        result = subprocess.run(
            argv, check=True, capture_output=True, text=True, timeout=30,
            env={**os.environ, "LC_ALL": "C"},
        )
    except (OSError, subprocess.SubprocessError) as error:
        # Do not echo arbitrary subprocess output or environment variables.
        raise ValueError(f"could not inspect {argv[0]}: {type(error).__name__}") from error
    value = result.stdout.strip()
    if not value:
        raise ValueError(f"{argv[0]} returned no identity")
    return value


def installed_packages(output):
    packages = {}
    for line in output.splitlines():
        fields = line.split("\t")
        if len(fields) != 4:
            raise ValueError("malformed dpkg-query package record")
        name, version, architecture, status = fields
        if status != "installed":
            continue
        if not all((name, version, architecture)):
            raise ValueError("incomplete installed package identity")
        key = (name, architecture)
        if key in packages:
            raise ValueError("duplicate installed package identity")
        packages[key] = {"name": name, "version": version, "architecture": architecture}
    if not packages:
        raise ValueError("dpkg-query reported no installed packages")
    return [packages[key] for key in sorted(packages)]


def file_digest(path):
    digest = hashlib.sha256()
    with path.open("rb") as source:
        for chunk in iter(lambda: source.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def tool_identity(name, hashes):
    # Hash real Rust executables, not their identically named rustup proxies.
    executable = capture(["rustup", "which", name]) if name in ("rustc", "cargo") else shutil.which(name)
    if not executable:
        raise ValueError(f"required tool is unavailable: {name}")
    path = Path(executable).resolve(strict=True)
    if not path.is_file() or not os.access(path, os.X_OK):
        raise ValueError(f"required tool is not executable: {name}")
    if path not in hashes:
        hashes[path] = file_digest(path)
    flags = ["-vV"] if name in ("rustc", "cargo") else ["--version"]
    return {"sha256": hashes[path], "version": capture([str(path), *flags])}


def native_identity(image_id):
    if not re.fullmatch(r"sha256:[0-9a-f]{64}", image_id):
        raise ValueError("image identity must be a resolved Docker sha256 image ID")
    architecture = capture(["dpkg", "--print-architecture"])
    if not re.fullmatch(r"[a-z0-9][a-z0-9-]*", architecture):
        raise ValueError("invalid Debian architecture")
    packages = installed_packages(capture(["dpkg-query", "-W", f"-f={PACKAGE_FORMAT}"]))
    hashes = {}
    return {
        "schema_version": SCHEMA_VERSION,
        "base_image_id": image_id,
        "architecture": architecture,
        "packages": packages,
        "tools": {name: tool_identity(name, hashes) for name in TOOLS},
        "cc_target": capture(["cc", "-dumpmachine"]),
    }


def identity_digest(identity):
    encoded = json.dumps(identity, sort_keys=True, separators=(",", ":")).encode()
    return hashlib.sha256(encoded).hexdigest()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--image-id", required=True, help="resolved ID of the base image used by this container")
    parser.add_argument("--output", type=Path, required=True, help="receipt outside the Cargo target/cache directories")
    args = parser.parse_args()
    try:
        identity = native_identity(args.image_id)
        digest = identity_digest(identity)
        receipt = {"sha256": digest, "identity": identity}
        # Refuse to overwrite an earlier receipt or follow a caller's symlink.
        # Printing only after this succeeds keeps a failed receipt from enabling
        # a cache restore with missing evidence.
        with args.output.open("x") as output:
            json.dump(receipt, output, indent=2, sort_keys=True)
            output.write("\n")
        print(digest)
    except (OSError, ValueError) as error:
        print(f"native build identity unavailable: {error}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
