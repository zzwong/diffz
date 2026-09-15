# Linux

Build commands below run from the repository root. Launch Diffz inside a
graphical session with a Vulkan driver appropriate for your GPU. The desktop
backend supports Wayland and X11.

## Build from source

Install Rust through [rustup](https://rustup.rs); the repository's
`rust-toolchain.toml` selects the toolchain. Install the native dependencies
for your distribution, then build and open a patch:

```sh
cargo build --locked --release -p diffz
./target/release/diffz change.patch
```

### Fedora

```sh
sudo dnf install gcc gcc-c++ pkgconf-pkg-config fontconfig-devel wayland-devel \
  libxkbcommon-devel libxkbcommon-x11-devel libX11-devel libXcursor-devel \
  libXi-devel libxcb-devel openssl-devel zstd-devel vulkan-loader-devel \
  mesa-vulkan-drivers fontconfig dejavu-sans-mono-fonts
```

To create an RPM and portable tarball from the release binary:

```sh
sudo dnf install rpm-build desktop-file-utils appstream
bash scripts/stage-linux.sh
bash scripts/package-linux-rpm.sh
bash scripts/package-linux-tarball.sh
(cd dist && sha256sum *.rpm *.tar.gz > SHA256SUMS)
sudo dnf install ./dist/diffz-*.rpm
```

Packages and the checksum manifest are written to `dist/`. Verify each
downloaded or copied package with `sha256sum -c --ignore-missing SHA256SUMS`
from that directory.

### Debian and Ubuntu

Debian and Ubuntu use the same application code and `.deb` packaging helper.
Build a package **on the distribution release where it will be installed**:
sharing the format does not guarantee compatibility between library versions.
In particular, do not repackage the Fedora binary or assume a Debian 13 build
will run on Debian 12 or Ubuntu 24.04.

The Debian packaging workflow targets Debian 12, Debian 13, Ubuntu 24.04 LTS,
and Ubuntu 26.04 LTS on `amd64` (Intel/AMD 64-bit). It builds separately in each
distribution, validates desktop metadata, and installs the result in a fresh
container with only declared runtime dependencies. The headless smoke tests
cover `--version`, `--help`, and patch inspection, not GPU rendering or a live
Wayland/X11 session. Check the workflow result before relying on an artifact.

Install the native build dependencies, then use the pinned rustup toolchain:

```sh
sudo apt-get update
sudo apt-get install build-essential clang cmake pkg-config python3 \
  ca-certificates curl git fontconfig fonts-dejavu-core libfontconfig-dev libwayland-dev \
  libxkbcommon-dev libxkbcommon-x11-dev libx11-dev libx11-xcb-dev \
  libxcursor-dev libxi-dev libxcb1-dev libssl-dev libzstd-dev libvulkan-dev
cargo build --locked --release -p diffz
```

To create and install a package from that release binary:

```sh
sudo apt-get install dpkg-dev binutils desktop-file-utils appstream
bash scripts/stage-linux.sh
desktop-file-validate dist/linux-root/usr/share/applications/io.github.zzwong.Diffz.desktop
appstreamcli validate --no-net dist/linux-root/usr/share/metainfo/io.github.zzwong.Diffz.metainfo.xml
bash scripts/package-linux-deb.sh
(cd dist && sha256sum ./*.deb > SHA256SUMS)
sudo apt install ./dist/diffz_0.1.0-1_amd64.deb
```

Use the actual filename printed by the helper if the version or architecture
differs. Building the package does not need root. It includes the executable,
desktop entry, scalable icon, AppStream metadata, license, and package changelog.
APT installs dependencies automatically; installing the binary alone does not.

For a published release, download the `.deb` built for your exact distribution
and `amd64` architecture from the [GitHub Releases page](https://github.com/zzwong/diffz/releases),
together with the combined `SHA256SUMS` file. Verify the downloaded package with
`sha256sum -c --ignore-missing SHA256SUMS` before installing it. The
`--ignore-missing` option checks the matching local asset without requiring
every release asset to be downloaded. The combined manifest covers the Debian
and Ubuntu packages along with the other Linux release assets; standalone
workflow artifacts are build outputs, not public release downloads.

The helper derives linked-library requirements with `dpkg-shlibdeps`, including
minimum ABI versions and distribution-specific names such as `libssl3t64`.
The package also declares libraries loaded dynamically by the desktop backend.
A working Vulkan GPU driver is still required: use `mesa-vulkan-drivers` for
supported Mesa GPUs, or your GPU vendor's driver. Do not replace a working
vendor driver just to install Diffz. Git, `gh`, and `glab` are optional and
needed only for their corresponding review sources.

`DIFFZ_DIST` selects the output directory; `DIFFZ_STAGE` selects an existing
staged tree (default `$DIFFZ_DIST/linux-root`). `DIFFZ_VERSION` overrides the
upstream version, and `DIFFZ_DEB_REVISION` defaults to `1`; CI uses revisions
such as `1~debian13`. SemVer prereleases are mapped from `-rc` to Debian's
`~rc` ordering. Set `SOURCE_DATE_EPOCH` to make package timestamps repeatable.

The helper accepts native `amd64` and `arm64` ELF binaries and rejects a
mismatched architecture rather than relabeling it. Native ARM64 builds are not
covered by this CI matrix; ARM64 and other distribution releases need their
own build and desktop verification before being advertised as supported.

Run the packaging regression tests without compiling GPUI:

```sh
python3 scripts/tests/test_package_linux_deb.py
```

These tests compile a tiny ELF fixture and use the real Debian packaging tools;
they do not establish that the Diffz desktop binary builds or renders correctly.

## Arch Linux and Omarchy

Build a package as your normal user:

```sh
sudo pacman -S --needed base-devel rust
bash scripts/package-arch.sh --syncdeps
sudo pacman -U target/arch-package/diffz-0.1.0-1-x86_64.pkg.tar.zst
```

The helper exports a pinned source snapshot from the local Git checkout,
verifies its checksum, and invokes makepkg. Prepared sources and packages
are written to `target/arch-package/`. Cargo also populates its cache;
`--syncdeps` installs missing dependencies with pacman. The recipe uses
Arch's packaged Rust compiler and declares build and runtime dependencies.

For a clean chroot build with Arch devtools:

```sh
sudo pacman -S --needed devtools
bash scripts/package-arch.sh --prepare-only
cd target/arch-package
extra-x86_64-build
```

The package installs the executable, desktop entry, scalable icon, AppStream
metadata and MIT license. For a new package version, update `_commit`,
`sha256sums`, and `pkgver`/`pkgrel` in `packaging/arch/PKGBUILD` together.
CI rejects mismatched release tags and application changes absent from the
pinned source.

## Desktop use

Launch Diffz from the application menu or open a patch with `diffz change.patch`.
Git, GitHub CLI and GitLab CLI enable their corresponding review sources.
### Code fonts

On Linux, Diffz resolves the system's `monospace` alias with Fontconfig at
startup and passes the concrete family name to GPUI. It does not assume that
`DejaVu Sans Mono` is installed, and it does not use GPUI's proportional UI-font
fallback for code. The same resolver is used by `--probe`.

Use `--font "Noto Sans Mono"` to prefer an installed code font. Missing or
proportional choices fall back to the system monospace font; substitutions
are reported on stderr. If no fixed-width font is available, desktop startup
fails with an installation hint instead of silently displaying proportional
code. macOS font selection is unchanged. Headless `--inspect`, `--help`,
`--version`, and `--doctor` do not require Fontconfig.

RPM and DEB packages require Fontconfig and a DejaVu monospace fallback. The
Arch recipe already requires `fontconfig` and `ttf-dejavu`. Source builds and
portable archives need these runtime dependencies installed separately.

To diagnose an older build, check what is actually installed:

```sh
fc-list --format '%{family}\n' | grep -F 'DejaVu Sans Mono'
fc-match --format '%{family[0]} | spacing=%{spacing}\n' monospace
# Temporary override for an older binary:
diffz --font "$(fc-match --format '%{family[0]}' monospace)" change.patch
```

To run the resolver regressions without building GPUI (requires `rustc`,
Python, Fontconfig, DejaVu Sans and Liberation Mono):

```sh
rustc --edition=2021 --test crates/app/src/code_font.rs -o /tmp/diffz-font-tests
/tmp/diffz-font-tests
python3 scripts/tests/test_code_font.py -v
```

The integration tests compile the production resolver and use temporary,
isolated font databases, including a system with no DejaVu Sans Mono. They
never change your desktop's font configuration. GPU text rendering and
Wayland/X11 still require desktop validation.

Omarchy colors are opt-in. Choose **Omarchy current theme** in the theme
picker or launch with `diffz --theme current change.patch`. The choice persists
and palette changes reload while the app is running. See [Themes](themes.md)
for palette paths, format and color mapping.
