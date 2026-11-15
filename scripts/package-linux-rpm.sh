#!/usr/bin/env bash
set -euo pipefail

cd "$(dirname "$0")/.."

version="${DIFFZ_VERSION:-$(sed -n 's/^version = "\([^"]*\)"/\1/p' Cargo.toml | head -1)}"
# Use the same absolute paths for staging, RPM macros, and output collection.
dist="$(realpath -m -- "${DIFFZ_DIST:-dist}")"
stage="$(realpath -m -- "${DIFFZ_STAGE:-$dist/linux-root}")"
top="$(realpath -m -- "${RPM_TOPDIR:-$dist/rpmbuild}")"

mkdir -p -- "$dist" "$top"/{BUILD,BUILDROOT,RPMS,SOURCES,SPECS,SRPMS}
rm -rf -- "$top/SOURCES/root"
cp -a -- "$stage" "$top/SOURCES/root"
rpmbuild -bb \
  --define "_topdir $top" \
  --define "_sourcedir $top/SOURCES" \
  --define "diffz_version $version" \
  packaging/linux/rpm/diffz.spec

find "$top/RPMS" -type f -name '*.rpm' -exec cp -t "$dist" -- {} +
echo "Created RPM in $dist"
