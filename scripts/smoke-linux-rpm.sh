#!/usr/bin/env bash
# Run as root in a fresh Fedora runtime container, never the build container.
# This installs an existing RPM and checks headless behavior, not GPU rendering.
set -euo pipefail
export LC_ALL=C

cd "${1:-dist}"
shopt -s nullglob
packages=(*.rpm)
if (( ${#packages[@]} != 1 )); then
  echo 'Expected exactly one RPM in the artifact directory.' >&2
  exit 1
fi
package="${packages[0]}"

sha256sum --check --strict SHA256SUMS
# A valid manifest for only the tarball must not authorize an unchecked RPM.
checksum="$(sha256sum -- "$package")"
if ! grep -Fxq -- "$checksum" SHA256SUMS; then
  echo 'The RPM checksum is missing from SHA256SUMS.' >&2
  exit 1
fi

name="$(rpm -qp --queryformat '%{NAME}' "$package")"
if [[ "$name" != diffz ]]; then
  echo 'Expected a diffz RPM.' >&2
  exit 1
fi
identity_format='%{NAME}-%{VERSION}-%{RELEASE}.%{ARCH}'
expected="$(rpm -qp --queryformat "$identity_format" "$package")"
# Install only the package and its required dependencies. Explicit libraries
# or recommended packages could conceal incomplete RPM runtime requirements.
# Clear container transaction flags such as nodocs so %doc payload is installed.
dnf install -y --setopt=install_weak_deps=False --setopt=tsflags= "$PWD/$package"
installed="$(rpm -q --queryformat "$identity_format" diffz)"
if [[ "$installed" != "$expected" ]]; then
  echo 'Installed diffz does not match the downloaded RPM.' >&2
  exit 1
fi

work="$(mktemp -d)"
trap 'rm -rf -- "$work"' EXIT
ldd /usr/bin/diffz > "$work/libraries.txt"
cat "$work/libraries.txt"
if grep -Fq 'not found' "$work/libraries.txt"; then
  echo 'Installed diffz has unresolved shared libraries.' >&2
  exit 1
fi
/usr/bin/diffz --version
/usr/bin/diffz --help
printf 'diff --git a/check.txt b/check.txt\n--- a/check.txt\n+++ b/check.txt\n@@ -1 +1 @@\n-before\n+after\n' > "$work/check.patch"
/usr/bin/diffz --state-dir "$work/state" --inspect --patch "$work/check.patch" > "$work/snapshot.json"
test -s "$work/snapshot.json"
grep -Fq '"check.txt"' "$work/snapshot.json"
grep -Fq '"text": "before"' "$work/snapshot.json"
grep -Fq '"text": "after"' "$work/snapshot.json"

for file in \
  /usr/share/applications/io.github.zzwong.Diffz.desktop \
  /usr/share/metainfo/io.github.zzwong.Diffz.metainfo.xml \
  /usr/share/icons/hicolor/scalable/apps/io.github.zzwong.Diffz.svg \
  /usr/share/licenses/diffz/LICENSE \
  /usr/share/doc/diffz/THIRD_PARTY_NOTICES.md
do
  if [[ ! -r "$file" || ! -s "$file" ]]; then
    echo "Installed file is missing, unreadable, or empty: $file" >&2
    exit 1
  fi
done
# Check installed payload ownership, modes, and digests against the RPM database.
rpm --verify diffz
