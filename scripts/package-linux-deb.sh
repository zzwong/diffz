#!/usr/bin/env bash
set -euo pipefail
export LC_ALL=C
umask 022

cd "$(dirname "$0")/.."

for tool in dpkg dpkg-architecture dpkg-deb dpkg-gencontrol dpkg-shlibdeps readelf python3; do
  command -v "$tool" >/dev/null || {
    echo "Missing $tool; install dpkg-dev, binutils, and python3." >&2
    exit 1
  }
done

upstream="$(python3 -c 'import tomllib; print(tomllib.load(open("Cargo.toml", "rb"))["workspace"]["package"]["version"])')"
version="${DIFFZ_VERSION:-$upstream}"
# Debian sorts '~rc' before the final release; SemVer uses '-rc'.
version="${version/-/\~}-${DIFFZ_DEB_REVISION:-1}"
dpkg --validate-version "$version"
arch="$(dpkg --print-architecture)"
export DEB_HOST_ARCH="$arch"

dist="$(realpath -m -- "${DIFFZ_DIST:-dist}")"
stage="$(realpath -m -- "${DIFFZ_STAGE:-$dist/linux-root}")"
binary="$stage/usr/bin/diffz"
[[ -x "$binary" ]] || {
  echo "Missing staged executable: $binary; run scripts/stage-linux.sh first." >&2
  exit 1
}
for file in \
  usr/share/applications/io.github.zzwong.Diffz.desktop \
  usr/share/metainfo/io.github.zzwong.Diffz.metainfo.xml \
  usr/share/icons/hicolor/scalable/apps/io.github.zzwong.Diffz.svg; do
  [[ -f "$stage/$file" ]] || {
    echo "Missing staged file: $stage/$file" >&2
    exit 1
  }
done

header="$(readelf -h "$binary")" || {
  echo "The staged binary must be a native Linux ELF executable." >&2
  exit 1
}
machine="$(sed -n 's/^ *Machine: *//p' <<< "$header")"
case "$arch:$machine" in
  'amd64:Advanced Micro Devices X86-64'|'arm64:AArch64') ;;
  *) echo "Unsupported or mismatched binary architecture: $arch / $machine" >&2; exit 1 ;;
esac
# Do not recursively copy a staging directory into itself.
[[ "$dist/" != "$stage/"* ]] || {
  echo 'The output directory must not be inside the staging directory.' >&2
  exit 1
}

mkdir -p "$dist"
work="$(mktemp -d "$dist/.deb-build.XXXXXX")"
trap 'rm -rf -- "$work"' EXIT
root="$work/debian/diffz"
mkdir -p "$root/DEBIAN"
cp -a -- "$stage/." "$root/"
install -Dm644 LICENSE "$root/usr/share/doc/diffz/copyright"
cp packaging/debian/control "$work/debian/control"

# Keep Debian's intermediate control files out of the checkout and shared stage.
cd "$work"
date="$(date -u -R -d "@${SOURCE_DATE_EPOCH:-$(date +%s)}")"
cat > debian/changelog <<CHANGELOG
diffz ($version) unstable; urgency=medium

  * Package the upstream desktop application for this distribution.

 -- Aaron Wong <6979793+zzwong@users.noreply.github.com>  $date
CHANGELOG
gzip -n -c debian/changelog > "$root/usr/share/doc/diffz/changelog.Debian.gz"

# Derive ABI floors and renamed packages (for example libssl3 vs libssl3t64)
# from the native build system. The control file also covers dlopen libraries,
# which cannot be discovered from the binary's ELF dependency table.
dpkg-shlibdeps -l"/usr/lib/$(dpkg-architecture -qDEB_HOST_MULTIARCH)" -Tdebian/substvars -e"$root/usr/bin/diffz"
dpkg-gencontrol -pdiffz -v"$version" -P"$root" -DArchitecture="$arch"
(
  cd "$root"
  find usr -type f -print0 | sort -z | xargs -0 md5sum > DEBIAN/md5sums
)

package="$dist/diffz_${version}_${arch}.deb"
dpkg-deb --root-owner-group -Zxz --build "$root" "$package"
echo "Created Debian package: $package"
