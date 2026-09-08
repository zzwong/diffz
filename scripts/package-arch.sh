#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/.."
repo="$PWD"
recipe="$repo/packaging/arch/PKGBUILD"
# This is the repository-owned recipe, not an untrusted downloaded PKGBUILD.
source "$recipe"
out="$repo/target/arch-package"
mkdir -p "$out"
cp "$recipe" "$out/PKGBUILD"
git -c tar.umask=0002 archive --format=tar --prefix="$pkgname-$pkgver/" "$_commit" \
  | gzip -n > "$out/$pkgname-$pkgver.tar.gz"
cd "$out"
makepkg --verifysource
if [[ "${1:-}" == --prepare-only ]]; then
  echo "Prepared Arch sources in $out"
  exit 0
fi
makepkg "$@"
