#!/usr/bin/env bash
set -euo pipefail

cd "$(dirname "$0")/.."

version="${DIFFZ_VERSION:-$(sed -n 's/^version = "\([^"]*\)"/\1/p' Cargo.toml | head -1)}"
arch="${DIFFZ_ARCH:-x86_64}"
dist="${DIFFZ_DIST:-dist}"
stage="${DIFFZ_STAGE:-$dist/linux-root}"
name="diffz-${version}-linux-${arch}"

mkdir -p "$dist"
tar -C "$stage" -czf "$dist/$name.tar.gz" .
echo "Created $dist/$name.tar.gz"
