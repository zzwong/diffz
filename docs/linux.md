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
  mesa-vulkan-drivers
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

Packages and the checksum manifest are written to `dist/`. Verify downloaded
or copied packages with `sha256sum -c SHA256SUMS` from that directory.

### Debian and Ubuntu

```sh
sudo apt-get install gcc g++ clang pkg-config libfontconfig-dev libwayland-dev \
  libwebkit2gtk-4.1-dev libxkbcommon-x11-dev libx11-xcb-dev libssl-dev libzstd-dev \
  vulkan-validationlayers libvulkan1
```

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
Use `--font "Noto Sans Mono"` to choose the diff font explicitly.

Omarchy colors are opt-in. Choose **Omarchy current theme** in the theme
picker or launch with `diffz --theme current change.patch`. The choice persists
and palette changes reload while the app is running. See [Themes](themes.md)
for palette paths, format and color mapping.
