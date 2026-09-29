#!/usr/bin/env bash
# Prints the SHA-256 of the Arch source archive for a commit, computed the way the
# release workflow computes it: git archive and gzip -n on x86_64 Arch Linux. A macOS or
# arm64 archive can hash differently, so this runs in an amd64 archlinux container.
# Usage: scripts/arch-source-sha256.sh COMMIT VERSION
set -euo pipefail
cd "$(dirname "$0")/.."
[[ $# -eq 2 ]] || { echo 'Usage: scripts/arch-source-sha256.sh COMMIT VERSION' >&2; exit 2; }
commit="$(git rev-parse --verify "$1^{commit}")"
version="$2"
docker run --rm --platform linux/amd64 -v "$PWD":/src:ro archlinux:base-devel \
  bash -euo pipefail -c '
    # pacman sandboxing fails under emulation; this container is throwaway.
    pacman -Syu --noconfirm --needed --disable-sandbox git >/dev/null
    git config --global --add safe.directory /src
    cd /src
    git -c tar.umask=0002 archive --format=tar --prefix="diffz-$2/" "$1" | gzip -n > /tmp/source.tar.gz
    # An empty or truncated archive would still hash; refuse anything implausibly small.
    [[ $(stat -c %s /tmp/source.tar.gz) -gt 100000 ]] || { echo "archive is too small" >&2; exit 1; }
    sha256sum /tmp/source.tar.gz | cut -d" " -f1
  ' _ "$commit" "$version"
