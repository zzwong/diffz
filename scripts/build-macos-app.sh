#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/.."
[[ "$(uname -s)" == Darwin ]] || { echo 'An app bundle can only be built on macOS.' >&2; exit 2; }
mode="${1:-debug}"
case "$mode" in
  debug) build_args=(build --locked -p diffz --features syntax) ;;
  release) build_args=(build --locked --release -p diffz --features syntax) ;;
  *) echo 'Usage: bash scripts/build-macos-app.sh [debug|release]' >&2; exit 2 ;;
esac
python3 - "$mode" <<'PYTHON'
import json, pathlib, shutil, subprocess, sys
metadata = json.loads(subprocess.check_output(["cargo", "metadata", "--no-deps", "--format-version", "1", "--locked"]))
target = pathlib.Path(metadata["target_directory"]) / sys.argv[1]

def remove_path(path):
    if path.is_symlink():
        path.unlink()
    elif path.is_dir():
        shutil.rmtree(path)
    elif path.exists():
        path.unlink()

for stale in (target / "Diffz.app", target / "diffz.app"):
    remove_path(stale)
PYTHON
cargo "${build_args[@]}"
python3 - "$mode" <<'PYTHON'
import json, pathlib, plistlib, shutil, subprocess, sys
metadata = json.loads(subprocess.check_output(["cargo", "metadata", "--no-deps", "--format-version", "1", "--locked"]))
version = next(p["version"] for p in metadata["packages"] if p["name"] == "diffz")
target = pathlib.Path(metadata["target_directory"]) / sys.argv[1]
app = target / "Diffz.app"
contents = app / "Contents"
(contents / "MacOS").mkdir(parents=True, exist_ok=True)
shutil.copy2(target / "diffz", contents / "MacOS" / "diffz")
resources = contents / "Resources"
resources.mkdir(parents=True, exist_ok=True)
shutil.copy2("LICENSE", resources / "LICENSE")
shutil.copy2("THIRD_PARTY_NOTICES.md", resources / "THIRD_PARTY_NOTICES.md")
(contents / "Info.plist").write_bytes(plistlib.dumps({
    "CFBundleExecutable": "diffz",
    "CFBundleIdentifier": "io.github.zzwong.Diffz",
    "CFBundleName": "Diffz",
    "CFBundleDisplayName": "Diffz",
    "CFBundlePackageType": "APPL",
    "CFBundleVersion": "1",
    "CFBundleShortVersionString": version,
    "LSMinimumSystemVersion": "15.0",
    "NSHighResolutionCapable": True,
}))
print(app)
PYTHON
