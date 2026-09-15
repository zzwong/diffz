#!/usr/bin/env bash
set -euo pipefail

cd "$(dirname "$0")/.."

stage="${1:-dist/linux-root}"
binary="${DIFFZ_BINARY:-target/release/diffz}"

[[ -x "$binary" ]] || {
  echo "release binary not found or not executable: $binary" >&2
  exit 1
}

rm -rf "$stage"
install -Dm755 "$binary" "$stage/usr/bin/diffz"
install -Dm644 LICENSE "$stage/usr/share/licenses/diffz/LICENSE"
install -Dm644 THIRD_PARTY_NOTICES.md \
  "$stage/usr/share/doc/diffz/THIRD_PARTY_NOTICES.md"
install -Dm644 packaging/linux/io.github.zzwong.Diffz.desktop \
  "$stage/usr/share/applications/io.github.zzwong.Diffz.desktop"
install -Dm644 packaging/linux/io.github.zzwong.Diffz.metainfo.xml \
  "$stage/usr/share/metainfo/io.github.zzwong.Diffz.metainfo.xml"
install -Dm644 packaging/linux/icons/io.github.zzwong.Diffz.svg \
  "$stage/usr/share/icons/hicolor/scalable/apps/io.github.zzwong.Diffz.svg"

echo "Staged Linux application at $stage"
