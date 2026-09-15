#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/.."
python3 - <<'PYTHON'
import hashlib
import json
import os
import pathlib
import platform
import shutil
import subprocess

metadata = json.loads(subprocess.check_output([
    "cargo", "metadata", "--no-deps", "--format-version", "1", "--locked"
]))
version = next(p["version"] for p in metadata["packages"] if p["name"] == "diffz")
target = pathlib.Path(metadata["target_directory"])
architecture = platform.machine()
if architecture not in ("arm64", "x86_64"):
    raise SystemExit(f"Unsupported macOS architecture: {architecture}")

out = target / "dist" / f"v{version}" / architecture
out.mkdir(parents=True, exist_ok=True)

app = target / "release" / "Diffz.app"
archive = out / f"diffz-{version}-macos-{architecture}.dmg"
checksums = out / "SHA256SUMS"
stage = out / "dmg-root"
temporary_archive = out / f".{archive.name}.tmp.dmg"
temporary_checksums = out / ".SHA256SUMS.tmp"

def remove_path(path):
    if path.is_symlink():
        path.unlink()
    elif path.is_dir():
        shutil.rmtree(path)
    elif path.exists():
        path.unlink()

legacy_checksum = out / "SHA256SUMS.txt"

try:
    for path in (archive, checksums, temporary_archive, temporary_checksums, stage):
        remove_path(path)
    for old_archive in out.glob("diffz-*-macos-*.zip"):
        old_archive.unlink()
    if legacy_checksum.exists():
        legacy_checksum.unlink()

    subprocess.run(["bash", "scripts/build-macos-app.sh", "release"], check=True)
    subprocess.run(["codesign", "--force", "--deep", "--sign", "-", str(app)], check=True)
    subprocess.run(["codesign", "--verify", "--deep", "--strict", str(app)], check=True)
    subprocess.run(["lipo", str(app / "Contents/MacOS/diffz"), "-verify_arch", architecture], check=True)
    stage.mkdir()
    shutil.copytree(app, stage / app.name, symlinks=True)
    (stage / "Applications").symlink_to("/Applications", target_is_directory=True)
    subprocess.run([
        "hdiutil",
        "create",
        "-volname",
        "Diffz",
        "-srcfolder",
        str(stage),
        "-ov",
        "-format",
        "UDZO",
        str(temporary_archive),
    ], check=True)
    os.replace(temporary_archive, archive)
    with archive.open("rb") as stream:
        checksum = hashlib.file_digest(stream, "sha256").hexdigest()
    temporary_checksums.write_text(f"{checksum}  {archive.name}\n")
    os.replace(temporary_checksums, checksums)
    remove_path(stage)
except BaseException:
    for path in (stage, temporary_archive, temporary_checksums, archive, checksums):
        remove_path(path)
    raise
print(out)
PYTHON
