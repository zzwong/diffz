#!/usr/bin/env python3
"""Check the pinned Bazel generator patches with real Rust and cached crate sources.

No downloads, Cargo updates, fake generators, or shared build-directory writes.
Run after the ordinary core/Wasm checks have populated Cargo's archive cache.
This checks generator behavior, not full Bazel integration or cache performance.
"""
from __future__ import annotations

import argparse
from collections import Counter
import hashlib
import json
import os
from pathlib import Path, PurePosixPath
import re
import shutil
import subprocess
import sys
import tarfile
import tempfile
import time
import tomllib

VERSION = "0.135.7"
CRATES = ("cranelift-srcgen", "cranelift-assembler-x64-meta",
          "cranelift-assembler-x64", "cranelift-isle")
SOURCE_BLOBS = {
    "cranelift-assembler-x64": "7309fe57e0eae2b42f08877df0ff41c09f89f5ca",
    "cranelift-isle": "ceb30591af97166b2a897b6bc72ecb097df075ad",
}
GROUPS = (("pass", "run_pass"), ("fail", "run_fail"),
          ("link", "run_link"), ("run", "run_run"),
          ("pass", "run_print"), ("link", "run_print"), ("run", "run_print"))
TEST = re.compile(r'#\[test\]\nfn (test_[A-Za-z0-9_]+)\(\) \{\n'
                  r'    (run_[a-z]+)\("([^"\n]+)"\);\n\}\n')


class VerificationError(RuntimeError):
    pass


def require(condition: bool, message: str) -> None:
    if not condition:
        raise VerificationError(message)


def sha256(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


def git_blob(data: bytes) -> str:
    return hashlib.sha1(f"blob {len(data)}\0".encode() + data).hexdigest()


def locked_crates(lock: dict) -> dict[str, dict]:
    result = {}
    for name in CRATES:
        matches = [p for p in lock["package"] if p["name"] == name]
        require(len(matches) == 1, f"expected one locked package: {name}")
        p = matches[0]
        require(p["version"] == VERSION, f"requalify {name} patches for {p['version']}")
        require(p.get("source") == "registry+https://github.com/rust-lang/crates.io-index",
                f"unexpected source for {name}")
        require(bool(re.fullmatch(r"[a-f0-9]{64}", p.get("checksum", ""))),
                f"missing or invalid locked archive checksum: {name}")
        result[name] = p
    return result


def unpack_verified(archive: Path, destination: Path, name: str, checksum: str) -> Path:
    """Verify archive identity, then copy only regular files into owned scratch."""
    require(sha256(archive.read_bytes()) == checksum, f"archive checksum mismatch: {archive}")
    prefix = f"{name}-{VERSION}"
    with tarfile.open(archive, "r:gz") as tar:
        files = []
        names = set()
        total = 0
        for member in tar.getmembers():
            path = PurePosixPath(member.name)
            require(not path.is_absolute() and path.parts and path.parts[0] == prefix
                    and ".." not in path.parts and "\\" not in member.name,
                    f"unsafe archive path: {member.name}")
            require(member.isfile() or member.isdir(), f"unsupported archive member: {member.name}")
            if member.isdir():
                continue
            require(len(path.parts) > 1 and str(path) not in names,
                    f"duplicate or invalid archive member: {member.name}")
            names.add(str(path))
            total += member.size
            require(0 <= member.size <= 32 * 1024**2 and total <= 128 * 1024**2,
                    "unexpectedly large generator source archive")
            files.append((member, path))
        # All paths/types are validated before writing; no tar extraction of links.
        for member, path in files:
            out = destination.joinpath(*path.parts)
            out.parent.mkdir(parents=True, exist_ok=True)
            stream = tar.extractfile(member)
            require(stream is not None, f"missing archive member: {member.name}")
            with stream:
                out.write_bytes(stream.read())
    return destination / prefix


def test_records(data: bytes) -> list[tuple[str, str, str]]:
    text = data.decode("utf-8")
    records = TEST.findall(text)
    require(bool(records) and not TEST.sub("", text).strip(),
            "unrecognized or empty generated ISLE test format")
    require(len({r[0] for r in records}) == len(records), "duplicate generated test names")
    return records


class Experiment:
    def __init__(self, repo: Path, output: Path, rustc: str):
        self.repo, self.output, self.rustc = repo, output, rustc
        self.report = {"schema_version": 1, "completed": False, "commands": [], "checks": {}}

    def run(self, label: str, args: list[str], cwd: Path, extra: dict | None = None,
            expect_success: bool = True) -> subprocess.CompletedProcess:
        env = dict(os.environ)
        for key in ("CARGO_MAKEFLAGS", "MAKEFLAGS", "RUSTC_WRAPPER", "RUSTC_WORKSPACE_WRAPPER",
                    "RUSTFLAGS", "CARGO_ENCODED_RUSTFLAGS"):
            env.pop(key, None)
        env.update(extra or {})
        started = time.monotonic()
        result = subprocess.run(args, cwd=cwd, env=env, stdin=subprocess.DEVNULL,
                                stdout=subprocess.PIPE, stderr=subprocess.PIPE, timeout=90)
        (self.output / f"{label}.stdout").write_bytes(result.stdout)
        (self.output / f"{label}.stderr").write_bytes(result.stderr)
        self.report["commands"].append({"label": label, "argv": args, "cwd": str(cwd),
            "seconds": time.monotonic() - started, "exit_code": result.returncode})
        require((result.returncode == 0) == expect_success,
                f"{label}: unexpected exit {result.returncode}; see {label}.stderr")
        return result

    def compile(self, label: str, source: Path, binary: Path, deps: list[str] | None = None,
                extra: dict | None = None, library: bool = False) -> None:
        args = [self.rustc, "--edition=2024", "--crate-name", label.replace("-", "_"),
                "-Copt-level=0", "-Cdebuginfo=0", str(source), "-o", str(binary)]
        if library:
            args.extend(["--crate-type=rlib"])
        args.extend(deps or [])
        self.run(label, args, source.parent, extra)

    def verify(self, cargo_home: Path, scratch: Path) -> None:
        lock_bytes = (self.repo / "Cargo.lock").read_bytes()
        packages = locked_crates(tomllib.loads(lock_bytes.decode()))
        channel = tomllib.loads((self.repo / "rust-toolchain.toml").read_text())["toolchain"]["channel"]
        version = self.run("rustc-version", [self.rustc, "-vV"], self.repo).stdout.decode()
        require(version.startswith(f"rustc {channel} "), "use the repository's pinned stable rustc")
        self.report.update({"rustc": version, "cargo_lock_sha256": sha256(lock_bytes),
            "revision": self.run("revision", ["git", "rev-parse", "HEAD"], self.repo).stdout.decode().strip(),
            "archives": {name: p["checksum"] for name, p in packages.items()}})
        sources = {}
        for name, p in packages.items():
            matches = sorted((cargo_home / "registry/cache").glob(f"*/{name}-{VERSION}.crate"))
            require(bool(matches), f"cached {name}-{VERSION}.crate missing; run ordinary core checks first")
            archives = [a for a in matches if sha256(a.read_bytes()) == p["checksum"]]
            require(bool(archives), f"no checksum-valid cached archive for {name}")
            sources[name] = unpack_verified(archives[0], scratch / "sources", name, p["checksum"])
        for name, expected in SOURCE_BLOBS.items():
            require(git_blob((sources[name] / "build.rs").read_bytes()) == expected,
                    f"review changed upstream build.rs before patching {name}")
        patched = {}
        for name in SOURCE_BLOBS:
            patched[name] = scratch / f"patched-{name}"
            shutil.copytree(sources[name], patched[name])
            patch = self.repo / "bazel" / f"{name}-deterministic.patch"
            self.run(f"patch-{name}", ["patch", "--batch", "--forward", "--fuzz=0", "-p1", "-i", str(patch)],
                     patched[name])
        tools = scratch / "tools"
        tools.mkdir()
        srcgen = tools / "libcranelift_srcgen.rlib"
        self.compile("cranelift_srcgen", sources["cranelift-srcgen"] / "src/lib.rs", srcgen, library=True)
        meta = tools / "libcranelift_assembler_x64_meta.rlib"
        self.compile("cranelift_assembler_x64_meta", sources["cranelift-assembler-x64-meta"] / "src/lib.rs", meta,
                     ["--extern", f"cranelift_srcgen={srcgen}", "-L", f"dependency={tools}"], library=True)
        generators = {}
        for kind, name in (("assembler", "cranelift-assembler-x64"), ("isle", "cranelift-isle")):
            for arm, root in (("baseline", sources[name]), ("candidate", patched[name])):
                binary = tools / f"{arm}-{kind}"
                deps = ["--extern", f"cranelift_assembler_x64_meta={meta}", "-L", f"dependency={tools}"] if kind == "assembler" else []
                self.compile(f"compile-{arm}-{kind}", root / "build.rs", binary, deps)
                generators[arm, kind] = binary
        self.assembler_checks(scratch, sources, generators)
        self.isle_checks(scratch, sources["cranelift-isle"], generators)

    def assembler_checks(self, scratch: Path, sources: dict, generators: dict) -> None:
        assembly = []
        metadata = {"baseline": [], "candidate": []}
        roots = []
        for index, component in enumerate(("ordinary", 'spaces and "quotes" and \\slash', "unicode-λ")):
            for arm in metadata:
                root = scratch / "output" / arm / component
                root.mkdir(parents=True)
                self.run(f"{arm}-assembler-{index}", [str(generators[arm, "assembler"])], scratch,
                         {"OUT_DIR": str(root)})
                data = (root / "generated-files.rs").read_bytes()
                metadata[arm].append(data)
                assembly.append((root / "assembler.rs").read_bytes())
                helper = scratch / "tools" / f"helper-{arm}-{index}"
                self.compile(f"compile-helper-{arm}-{index}", sources["cranelift-assembler-x64"] / "src/main.rs",
                             helper, extra={"OUT_DIR": str(root)})
                result = self.run(f"helper-{arm}-{index}", [str(helper)], scratch)
                require(result.stdout.decode().splitlines() == [str(root / "assembler.rs")], "helper resolved wrong file")
                if arm == "candidate":
                    require(str(root).encode() not in data, "producer OUT_DIR still embedded")
                    roots.append(root)
        require(len(set(metadata["baseline"])) > 1, "baseline did not expose producer path variation")
        require(len(set(metadata["candidate"])) == 1, "candidate metadata varies across output roots")
        require(len(set(assembly)) == 1, "generated assembler source changed")
        # Transfer source metadata, not a compiled helper. Compile against consumer OUT_DIR.
        consumer = scratch / "consumer"
        consumer.mkdir()
        shutil.copyfile(roots[0] / "generated-files.rs", consumer / "generated-files.rs")
        shutil.copyfile(roots[0] / "assembler.rs", consumer / "assembler.rs")
        helper = scratch / "tools" / "consumer-helper"
        self.compile("compile-consumer-helper", sources["cranelift-assembler-x64"] / "src/main.rs", helper,
                     extra={"OUT_DIR": str(consumer)})
        result = self.run("consumer-helper", [str(helper)], scratch)
        require(result.stdout.decode().splitlines() == [str(consumer / "assembler.rs")], "consumer used producer path")
        self.report["checks"]["assembler"] = {"output_roots_per_arm": 3, "helpers_passed": 7,
            "assembler_bytes": len(assembly[0]), "assembler_sha256": sha256(assembly[0]),
            "metadata_sha256": sha256(metadata["candidate"][0]), "transferred_metadata_resolves_consumer": True}

    def isle_checks(self, scratch: Path, source: Path, generators: dict) -> None:
        examples = source / "isle_examples"
        require(examples.is_dir(), "published ISLE examples are missing")
        files = sorted(p for p in examples.rglob("*") if p.is_file())
        candidate = []
        expected = None
        baseline = []
        for order, ordered in (("forward", files), ("reverse", list(reversed(files)))):
            root = scratch / f"isle-{order}"
            for directory in ("pass", "fail", "link", "run"):
                (root / "isle_examples" / directory).mkdir(parents=True, exist_ok=True)
            for path in ordered:
                dest = root / "isle_examples" / path.relative_to(examples)
                dest.parent.mkdir(parents=True, exist_ok=True)
                dest.write_bytes(path.read_bytes())
            for arm in ("baseline", "candidate"):
                out = root / arm
                out.mkdir()
                self.run(f"{arm}-isle-{order}", [str(generators[arm, "isle"])], root, {"OUT_DIR": str(out)})
                data = (out / "isle_tests.rs").read_bytes()
                records = test_records(data)
                if expected is None:
                    expected = Counter(records)
                require(Counter(records) == expected, "ISLE test membership/runner/path changed")
                if arm == "candidate":
                    sorted_expected = [(f"test_{runner}_{p.name.replace('.isle', '')}", runner,
                                        f"isle_examples/{directory}/{p.name}")
                                       for directory, runner in GROUPS
                                       for p in sorted((root / "isle_examples" / directory).iterdir())
                                       if p.name.endswith(".isle")]
                    require(records == sorted_expected, "ISLE output not sorted within each group")
                    candidate.append(data)
                else:
                    baseline.append(data)
        require(candidate[0] == candidate[1], "ISLE bytes vary with creation order")
        root = scratch / "isle-forward"
        out = root / "candidate"
        probe = root / "isle_examples/pass/zz_diffz_build_repro_probe.isle"
        probe.write_text(";; generator filename probe\n")
        self.run("candidate-isle-add", [str(generators["candidate", "isle"])], root, {"OUT_DIR": str(out)})
        changed = Counter(test_records((out / "isle_tests.rs").read_bytes()))
        require(sum(changed.values()) == sum(expected.values()) + 2 and not (expected - changed), "real input edit not reflected")
        probe.unlink()
        (probe.parent / "ignored.txt").write_text("not an ISLE input\n")
        self.run("candidate-isle-restore", [str(generators["candidate", "isle"])], root, {"OUT_DIR": str(out)})
        require((out / "isle_tests.rs").read_bytes() == candidate[0], "ISLE restore failed")
        shutil.rmtree(root / "isle_examples/fail")
        self.run("candidate-isle-missing-directory", [str(generators["candidate", "isle"])], root,
                 {"OUT_DIR": str(out)}, expect_success=False)
        self.report["checks"]["isle"] = {"tests": sum(expected.values()), "bytes": len(candidate[0]),
            "sha256": sha256(candidate[0]), "creation_orders": 2, "added_input_and_restore_passed": True,
            "missing_directory_failed": True, "baseline_orders_differed": baseline[0] != baseline[1]}


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output", type=Path, required=True, help="new evidence directory")
    parser.add_argument("--cargo-home", type=Path, default=Path(os.environ.get("CARGO_HOME", str(Path.home() / ".cargo"))))
    parser.add_argument("--rustc", default="rustc")
    args = parser.parse_args()
    # Refuse to replace earlier evidence. A failed run gets a non-success report.
    output = args.output.resolve()
    try:
        output.mkdir(parents=True, exist_ok=False)
    except OSError as error:
        print(f"cannot create new evidence directory: {error}", file=sys.stderr)
        return 1
    repo = Path(__file__).resolve().parents[1]
    experiment = Experiment(repo, output, args.rustc)
    try:
        with tempfile.TemporaryDirectory(prefix="diffz-cranelift-generators-") as directory:
            experiment.verify(args.cargo_home.resolve(), Path(directory))
        experiment.report["completed"] = True
    except (OSError, ValueError, KeyError, VerificationError, subprocess.SubprocessError, tarfile.TarError) as error:
        experiment.report["error"] = str(error)
    (output / "result.json").write_text(json.dumps(experiment.report, indent=2) + "\n")
    print(json.dumps(experiment.report, indent=2))
    return 0 if experiment.report["completed"] else 1


if __name__ == "__main__":
    raise SystemExit(main())
