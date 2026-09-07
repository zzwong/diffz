#!/usr/bin/env bash
set -euo pipefail

cd "$(dirname "$0")/.."

version="${DIFFZ_VERSION:-$(sed -n 's/^version = "\([^"]*\)"/\1/p' Cargo.toml | head -1)}"
dist="${DIFFZ_DIST:-dist}"
stage="${DIFFZ_STAGE:-$dist/linux-root}"
top="${RPM_TOPDIR:-$dist/rpmbuild}"

mkdir -p "$top"/{BUILD,BUILDROOT,RPMS,SOURCES,SPECS,SRPMS}
rm -rf "$top/SOURCES/root"
cp -a "$stage" "$top/SOURCES/root"
rpmbuild -bb \
  --define "_topdir $PWD/$top" \
  --define "_sourcedir $PWD/$top/SOURCES" \
  --define "diffz_version $version" \
  packaging/linux/rpm/diffz.spec

find "$top/RPMS" -type f -name '*.rpm' -exec cp {} "$dist/" \;
echo "Created RPM in $dist"
