#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/.."
bash scripts/build-macos-app.sh release
python3 - <<'PYTHON'
import hashlib
import json
import pathlib
import platform
import shutil
import subprocess

metadata = json.loads(subprocess.check_output([
    "cargo", "metadata", "--no-deps", "--format-version", "1", "--locked"
]))
version = next(p["version"] for p in metadata["packages"] if p["name"] == "diffz")
target = pathlib.Path(metadata["target_directory"])
app = target / "release" / "Diffz.app"
architecture = platform.machine()
if architecture not in ("arm64", "x86_64"):
    raise SystemExit(f"Unsupported macOS architecture: {architecture}")

subprocess.run(["codesign", "--force", "--deep", "--sign", "-", str(app)], check=True)
subprocess.run(["codesign", "--verify", "--deep", "--strict", str(app)], check=True)
subprocess.run(["lipo", str(app / "Contents/MacOS/diffz"), "-verify_arch", architecture], check=True)
out = target / "dist" / f"v{version}" / architecture
out.mkdir(parents=True, exist_ok=True)

for old_archive in out.glob("diffz-*-macos-*.zip"):
    old_archive.unlink()
legacy_checksum = out / "SHA256SUMS.txt"
if legacy_checksum.exists():
    legacy_checksum.unlink()

archive = out / f"diffz-{version}-macos-{architecture}.dmg"
stage = out / "dmg-root"
if stage.exists():
    shutil.rmtree(stage)
stage.mkdir()
shutil.copytree(app, stage / app.name, symlinks=True)
(stage / "Applications").symlink_to("/Applications", target_is_directory=True)
if archive.exists():
    archive.unlink()
try:
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
        str(archive),
    ], check=True)
finally:
    shutil.rmtree(stage)

with archive.open("rb") as stream:
    checksum = hashlib.file_digest(stream, "sha256").hexdigest()
(out / "SHA256SUMS").write_text(f"{checksum}  {archive.name}\n")
print(out)
PYTHON
