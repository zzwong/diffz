#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/.."
bash scripts/build-macos-app.sh release
python3 - <<'PYTHON'
import hashlib
import json
import pathlib
import platform
import subprocess

metadata = json.loads(subprocess.check_output([
    "cargo", "metadata", "--no-deps", "--format-version", "1", "--locked"
]))
version = next(p["version"] for p in metadata["packages"] if p["name"] == "diffz")
target = pathlib.Path(metadata["target_directory"])
app = target / "release" / "diffz.app"
architecture = platform.machine()
if architecture not in ("arm64", "x86_64"):
    raise SystemExit(f"Unsupported macOS architecture: {architecture}")
subprocess.run(["lipo", str(app / "Contents/MacOS/diffz"), "-verify_arch", architecture], check=True)
out = target / "dist" / f"v{version}" / architecture
out.mkdir(parents=True, exist_ok=True)
archive = out / f"diffz-{version}-macos-{architecture}.zip"
if archive.exists():
    archive.unlink()
subprocess.run(["ditto", "-c", "-k", "--sequesterRsrc", "--keepParent", str(app), str(archive)], check=True)
checksum = hashlib.file_digest(archive.open("rb"), "sha256").hexdigest()
(out / "SHA256SUMS.txt").write_text(f"{checksum}  {archive.name}\n")
print(out)
PYTHON
