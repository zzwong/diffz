#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/.."
[[ "$(uname -s)" == Darwin ]] || { echo 'An app bundle can only be built on macOS.' >&2; exit 2; }
mode="${1:-debug}"
case "$mode" in
  debug) cargo build --locked -p diffz --features syntax ;;
  release) cargo build --locked --release -p diffz --features syntax ;;
  *) echo 'Usage: bash scripts/build-macos-app.sh [debug|release]' >&2; exit 2 ;;
esac
python3 - "$mode" <<'PYTHON'
import json, pathlib, plistlib, shutil, subprocess, sys
metadata = json.loads(subprocess.check_output(["cargo", "metadata", "--no-deps", "--format-version", "1", "--locked"]))
target = pathlib.Path(metadata["target_directory"]) / sys.argv[1]
app = target / "diffz.app"
contents = app / "Contents"
(contents / "MacOS").mkdir(parents=True, exist_ok=True)
shutil.copy2(target / "diffz", contents / "MacOS" / "diffz")
(contents / "Info.plist").write_bytes(plistlib.dumps({
    "CFBundleExecutable": "diffz",
    "CFBundleIdentifier": "local.diffz",
    "CFBundleName": "diffz",
    "CFBundleDisplayName": "diffz",
    "CFBundlePackageType": "APPL",
    "CFBundleVersion": "1",
    "CFBundleShortVersionString": "0.1.0",
    "LSMinimumSystemVersion": "15.0",
    "NSHighResolutionCapable": True,
}))
print(app)
PYTHON
