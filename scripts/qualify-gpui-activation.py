#!/usr/bin/env python3
"""Run real vendored GPUI activation tests and isolated Wayland/X11 checks.

Cargo cannot select features on dependencies outside the app workspace. Stage
unchanged vendor sources as a workspace member under target/. Offline resolution
may prune the app lock, but cannot introduce or replace any locked package.
"""
import argparse
import json
import os
from pathlib import Path
import re
import shutil
import subprocess
import tempfile
import tomllib

VENDOR = "gpui-pre-linux"
TESTS = {
    "linux::wayland::client::activation_lifetime_tests::absent_environment_is_not_mutated_but_every_present_value_is_removed",
    "linux::wayland::client::activation_lifetime_tests::startup_token_is_consumed_only_by_the_first_toplevel",
    "linux::wayland::window::external_activation_tests::external_submission_is_exact_and_once_without_a_diffz_size_limit",
    "linux::wayland::window::external_activation_tests::invalid_or_unsupported_context_never_calls_the_sink",
}


def verify_lock(seed, resolved):
    def identity(package):
        return package["name"], package["version"], package.get("source")

    def dependencies(package, packages):
        found = set()
        for dependency in package.get("dependencies", []):
            match = re.fullmatch(r"(\S+?)(?: ([^ (]+))?(?: \((.+)\))?", dependency)
            if match is None:
                raise RuntimeError(f"Invalid locked dependency: {dependency}")
            name, version, source = match.groups()
            candidates = [identity(entry) for entry in packages if entry["name"] == name
                          and (version is None or entry["version"] == version)
                          and (source is None or entry.get("source") == source)]
            if len(candidates) != 1:
                raise RuntimeError(f"Ambiguous locked dependency: {dependency}")
            found.add(candidates[0])
        return found

    if seed["version"] != resolved["version"]:
        raise RuntimeError("Qualification changed the lock format")
    original = {identity(package): package for package in seed["package"]}
    for package in resolved["package"]:
        key = identity(package)
        if key not in original or package.get("checksum") != original[key].get("checksum"):
            raise RuntimeError(f"Qualification changed a locked package: {key}")
        if not dependencies(package, resolved["package"]) <= dependencies(original[key], seed["package"]):
            raise RuntimeError(f"Qualification added a dependency edge: {key}")
    patch_sources = set(original) | {identity(patch) for patch in seed.get("patch", {}).get("unused", [])}
    for patch in resolved.get("patch", {}).get("unused", []):
        if identity(patch) not in patch_sources:
            raise RuntimeError(f"Qualification changed an unused patch: {identity(patch)}")


def verify_features(metadata, backend):
    packages = {package["id"]: package for package in metadata["packages"]}
    nodes = {node["id"]: node for node in metadata["resolve"]["nodes"]}
    member, = metadata["workspace_members"]
    if packages[member]["name"] != VENDOR:
        raise RuntimeError("Expected only the vendored Linux workspace member")
    pending, reachable = [member], set()
    while pending:
        current = pending.pop()
        if current not in reachable:
            reachable.add(current)
            pending.extend(dependency["pkg"] for dependency in nodes[current]["deps"])
    opposite = "x11" if backend == "wayland" else "wayland"
    for package in reachable:
        name = packages[package]["name"]
        features = set(nodes[package]["features"])
        if name == VENDOR and (backend not in features or "default" in features):
            raise RuntimeError(f"Missing isolated {backend} configuration")
        if name in (VENDOR, "gpui-pre", "gpui-pre-wgpu") and features & {opposite, "test-support"}:
            raise RuntimeError(f"Opposite backend or test support leaked into {name}")


def run(argv, cwd, env, *, capture=False):
    print("+ " + " ".join(map(str, argv)), flush=True)
    result = subprocess.run(argv, cwd=cwd, env=env, check=True,
                            stdout=subprocess.PIPE if capture else None, text=True)
    return result.stdout if capture else None


def qualify(repo):
    manifest = (repo / "Cargo.toml").read_text()
    config = tomllib.loads(manifest)
    for package, patch in config["patch"]["crates-io"].items():
        if "path" in patch and (package != VENDOR or patch["path"] != "vendor/gpui-pre-linux"):
            raise RuntimeError(f"Unstaged path patch: {package}")
    sections = manifest[manifest.index("[patch.crates-io]"):]
    if set(tomllib.loads(sections)) != {"patch", "profile"}:
        raise RuntimeError("Unexpected sections after the app patch declarations")
    toolchain = tomllib.loads((repo / "rust-toolchain.toml").read_text())["toolchain"]["channel"]
    cargo = ["rustup", "run", toolchain, "cargo"]
    env = dict(os.environ, CARGO_TARGET_DIR=str(repo / "target"), CARGO_NET_OFFLINE="true", CARGO_TERM_COLOR="never")
    version = run(["rustup", "run", toolchain, "rustc", "-vV"], repo, env, capture=True)
    host, = re.findall(r"^host: (\S+)$", version, re.MULTILINE)
    metadata_command = cargo + ["metadata", "--format-version", "1", "--filter-platform", host]
    seed_bytes = (repo / "Cargo.lock").read_bytes()
    seed = tomllib.loads(seed_bytes.decode())
    staging = repo / "target" / "activation-qualification"
    staging.mkdir(parents=True, exist_ok=True)
    with tempfile.TemporaryDirectory(prefix="workspace-", dir=staging) as temporary:
        root = Path(temporary)
        shutil.copytree(repo / "vendor" / VENDOR, root / "vendor" / VENDOR)
        (root / "Cargo.toml").write_text(
            '[workspace]\nresolver = "2"\nmembers = ["vendor/gpui-pre-linux"]\n\n' + sections)
        (root / "Cargo.lock").write_bytes(seed_bytes)
        unit_features = "gpui-pre-linux/wayland,gpui-pre-linux/x11,gpui-pre-linux/test-support"
        run(metadata_command + ["--offline",
                     "--no-default-features", "--features", unit_features], root, env, capture=True)
        verify_lock(seed, tomllib.loads((root / "Cargo.lock").read_text()))
        selection = ["--locked", "-p", VENDOR, "--no-default-features", "--features", unit_features, "--lib"]
        inventory = run(cargo + ["test", *selection, "activation", "--", "--list"], root, env, capture=True)
        actual = {line.removesuffix(": test") for line in inventory.splitlines() if line.endswith(": test")}
        if not TESTS <= actual:
            raise RuntimeError(f"Activation tests missing from inventory: {sorted(TESTS - actual)}")
        results = run(cargo + ["test", *selection, "activation", "--", "--nocapture"], root, env, capture=True)
        print(results, flush=True)
        for name in TESTS:
            if re.search(r"^test " + re.escape(name) + r" \.\.\. ok$", results, re.MULTILINE) is None:
                raise RuntimeError(f"Activation test did not execute successfully: {name}")
        for backend in ("wayland", "x11"):
            features = ["--no-default-features", "--features", f"{VENDOR}/{backend}"]
            metadata = json.loads(run(metadata_command + ["--locked", *features],
                                      root, env, capture=True))
            verify_features(metadata, backend)
            run(cargo + ["check", "--locked", "-p", VENDOR, *features, "--lib"], root, env)
        verify_lock(seed, tomllib.loads((root / "Cargo.lock").read_text()))
    if (repo / "Cargo.lock").read_bytes() != seed_bytes:
        raise RuntimeError("App lock changed during qualification")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--repo", type=Path, default=Path(__file__).resolve().parents[1])
    args = parser.parse_args()
    qualify(args.repo.resolve())


if __name__ == "__main__":
    main()
