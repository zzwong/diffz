#!/usr/bin/env python3
"""Identify the prepared Debian/Ubuntu builder before restoring Cargo artifacts.

Run in the same prepared container or hosted Ubuntu runner that will compile,
after apt and rustup have finished. This is an observed native-environment
identity, not a claim that the environment is a hermetic build toolchain.
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


def native_environment():
    architecture = capture(["dpkg", "--print-architecture"])
    if not re.fullmatch(r"[a-z0-9][a-z0-9-]*", architecture):
        raise ValueError("invalid Debian architecture")
    packages = installed_packages(capture(["dpkg-query", "-W", f"-f={PACKAGE_FORMAT}"]))
    hashes = {}
    return {
        "architecture": architecture,
        "packages": packages,
        "tools": {name: tool_identity(name, hashes) for name in TOOLS},
        "cc_target": capture(["cc", "-dumpmachine"]),
    }


def native_identity(image_id):
    if not re.fullmatch(r"sha256:[0-9a-f]{64}", image_id):
        raise ValueError("image identity must be a resolved Docker sha256 image ID")
    # Retain the existing container receipt and digest format unchanged.
    return {
        "schema_version": SCHEMA_VERSION,
        "base_image_id": image_id,
        **native_environment(),
    }


def hosted_identity(environment):
    image = {}
    for name in ("ImageOS", "ImageVersion"):
        value = environment.get(name, "")
        if not re.fullmatch(r"[A-Za-z0-9][A-Za-z0-9_.-]{0,127}", value):
            raise ValueError(f"{name} must identify the hosted runner image")
        image[name] = value
    if not image["ImageOS"].startswith("ubuntu"):
        raise ValueError("hosted native identity supports Ubuntu runners only")
    # Hosted image labels are evidence, not immutable Docker IDs. Native package
    # versions and actual compiler/linker bytes remain part of this identity.
    return {"schema_version": 2, "runner_image": image, **native_environment()}


def identity_digest(identity):
    encoded = json.dumps(identity, sort_keys=True, separators=(",", ":")).encode()
    return hashlib.sha256(encoded).hexdigest()


def read_receipt(path):
    receipt = json.loads(path.read_text())
    if not isinstance(receipt, dict) or not isinstance(receipt.get("identity"), dict):
        raise ValueError("receipt must contain an identity object")
    identity = receipt["identity"]
    if type(identity.get("schema_version")) is not int or identity["schema_version"] not in (1, 2):
        raise ValueError("unsupported native identity schema")
    if receipt.get("sha256") != identity_digest(identity):
        raise ValueError("receipt digest does not match its identity")
    if not isinstance(identity.get("packages"), list) or not isinstance(identity.get("tools"), dict):
        raise ValueError("receipt must contain packages and tools")
    required = {"schema_version", "architecture", "cc_target", "packages", "tools"}
    required.add("base_image_id" if identity["schema_version"] == 1 else "runner_image")
    if set(identity) != required:
        raise ValueError("incomplete or unexpected native identity fields")
    if not isinstance(identity["architecture"], str) or not re.fullmatch(r"[a-z0-9][a-z0-9-]*", identity["architecture"]):
        raise ValueError("invalid receipt architecture")
    if not isinstance(identity["cc_target"], str) or not identity["cc_target"].strip():
        raise ValueError("invalid receipt C compiler target")
    if identity["schema_version"] == 1:
        image = identity["base_image_id"]
        if not isinstance(image, str) or not re.fullmatch(r"sha256:[0-9a-f]{64}", image):
            raise ValueError("invalid receipt base image")
    else:
        image = identity["runner_image"]
        if not isinstance(image, dict) or set(image) != {"ImageOS", "ImageVersion"}:
            raise ValueError("invalid receipt runner image")
        if any(not isinstance(v, str) or not re.fullmatch(r"[A-Za-z0-9][A-Za-z0-9_.-]{0,127}", v)
               for v in image.values()) or not image["ImageOS"].startswith("ubuntu"):
            raise ValueError("invalid receipt runner image")
    if set(identity["tools"]) != set(TOOLS):
        raise ValueError("incomplete or unexpected receipt tools")
    for tool in identity["tools"].values():
        if not isinstance(tool, dict) or set(tool) != {"sha256", "version"}:
            raise ValueError("invalid receipt tool identity")
        if (not isinstance(tool["sha256"], str) or not re.fullmatch(r"[0-9a-f]{64}", tool["sha256"])
                or not isinstance(tool["version"], str) or not tool["version"].strip()):
            raise ValueError("invalid receipt tool identity")
    if not identity["packages"]:
        raise ValueError("receipt must contain installed packages")
    packages = {}
    for package in identity["packages"]:
        if not isinstance(package, dict) or set(package) != {"name", "architecture", "version"} or not all(isinstance(package.get(k), str) and package[k]
                                                     for k in ("name", "architecture", "version")):
            raise ValueError("invalid receipt package")
        key = (package["name"], package["architecture"])
        if key in packages:
            raise ValueError("duplicate receipt package")
        packages[key] = package["version"]
    return receipt, packages


def compare_receipts(before_path, after_path):
    before, old_packages = read_receipt(before_path)
    after, new_packages = read_receipt(after_path)
    old, new = before["identity"], after["identity"]
    packages = [dict(name=name, architecture=arch,
                     before=old_packages.get((name, arch)), after=new_packages.get((name, arch)))
                for name, arch in sorted(old_packages.keys() | new_packages.keys())
                if old_packages.get((name, arch)) != new_packages.get((name, arch))]
    tools = [dict(name=name, before=old["tools"].get(name), after=new["tools"].get(name))
             for name in sorted(old["tools"].keys() | new["tools"].keys())
             if old["tools"].get(name) != new["tools"].get(name)]
    fields = [dict(name=name, before=old.get(name), after=new.get(name))
              for name in sorted(old.keys() | new.keys()) if name not in ("packages", "tools")
              and old.get(name) != new.get(name)]
    return dict(schema_version=1, before_sha256=before["sha256"], after_sha256=after["sha256"],
                same_identity=before["sha256"] == after["sha256"],
                packages=packages, tools=tools, fields=fields,
                package_order_changed=(old_packages.keys() == new_packages.keys()
                                       and list(old_packages) != list(new_packages)))


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    source = parser.add_mutually_exclusive_group(required=True)
    source.add_argument("--image-id", help="resolved ID of the base image used by this container")
    source.add_argument("--hosted-ubuntu", action="store_true", help="use the hosted runner's ImageOS and ImageVersion")
    source.add_argument("--compare", nargs=2, type=Path, metavar=("BEFORE", "AFTER"),
                        help="compare saved identity receipts without probing tools or changing caches")
    parser.add_argument("--output", type=Path, help="receipt outside the Cargo target/cache directories")
    args = parser.parse_args()
    if args.compare:
        if args.output:
            parser.error("--compare writes JSON to stdout; do not use --output")
        try:
            print(json.dumps(compare_receipts(*args.compare), indent=2, sort_keys=True))
            return 0
        except (OSError, ValueError) as error:
            print(f"native identity comparison unavailable: {error}", file=sys.stderr)
            return 1
    if not args.output:
        parser.error("--output is required when collecting a native identity")
    try:
        identity = hosted_identity(os.environ) if args.hosted_ubuntu else native_identity(args.image_id)
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
