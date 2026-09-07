# Arch Linux and Omarchy

Diffz uses its Linux GPUI backend on Arch and Omarchy. Omarchy colors are an
application palette, not a GTK stylesheet or a terminal color scheme.

## Build an Arch package

The repository is private, so the package source is exported from your
already authenticated checkout. No GitHub token is embedded in the recipe
or package. `packaging/arch/PKGBUILD` pins the application commit and SHA-256
of its deterministic source archive. Update `_commit`, `sha256sums`, and
`pkgver`/`pkgrel` together when packaging a new version. CI rejects a mismatched
release tag or application changes absent from the pinned Arch source.

Install the Arch build tools, then build as your normal user:

```sh
sudo pacman -S --needed base-devel rust
bash scripts/package-arch.sh --syncdeps
sudo pacman -U target/arch-package/diffz-0.1.0-1-x86_64.pkg.tar.zst
```

The helper prepares the source archive and recipe under `target/arch-package/`.
It verifies the archive and runs makepkg with the arguments supplied. Cargo
also populates its cache; `--syncdeps` installs missing dependencies with pacman. Build dependencies
and runtime dependencies are declared in the PKGBUILD. Install a Vulkan
driver appropriate for your GPU. Git, GitHub CLI and GitLab CLI are optional
for their corresponding review sources.

For a conventional clean chroot build with Arch devtools:

```sh
sudo pacman -S --needed devtools
bash scripts/package-arch.sh --prepare-only
cd target/arch-package
extra-x86_64-build
```

The package installs the executable, desktop entry, scalable icon, AppStream
metadata and MIT license. Launch Diffz from the application menu or open a
patch with `diffz change.patch`. This recipe is not published to the AUR.

## Follow the current Omarchy palette

Choose **Omarchy current theme** from the app's theme picker, or launch:

```sh
diffz --theme current change.patch
```

The choice persists. Diffz reads
`~/.local/state/omarchy/current/theme/colors.toml`, applies its colors to
widgets, syntax highlighting, selections and diff backgrounds, and checks
for file changes every two seconds. Built-in light/dark palettes remain
available. Omarchy colors are opt-in; launching on Omarchy does not override
an existing theme choice.

You can also use `--theme /path/to/colors.toml` on any supported desktop.
See [theme format and color mapping](themes.md) for the required keys.

## Validation

On September 7, 2026, the application built on Omarchy 4.0.2 x86_64
with Rust 1.98.0. In its Hyprland Wayland session:

- The current Lavender Mist palette visibly applied to widgets and the diff.
- Replacing a `current/theme` symlink in an isolated test home reloaded a light
  palette in the running app and persisted the new appearance. The user's
  actual desktop theme was unchanged.
- Command-palette navigation, switching rich/raw views, keyboard selection,
  and Page Down scrolling worked.
- Clipboard copying matched the selected header and the entire 10,783-byte
  Unicode source line exactly; the previous clipboard contents were restored.

The pinned application source also built successfully with Arch's packaged
Rust 1.98.1 in a fresh, signature-verified Arch bootstrap filesystem. Bubblewrap
provided an unprivileged namespace; no host libraries or Cargo cache were
mounted into it. All 172 workspace tests passed inside this clean environment (the native
integration test requires a display and remains ignored there). The resulting
release executable passed all four native geometry fixtures on Omarchy.

The final `.pkg.tar.zst` passed `pacman -U`, `diffz --version`, `pacman -Qkk`
(16 files, zero altered), and `pacman -R` inside that isolated Arch environment.
The extracted package payload passed the Unicode probe and launched a patch
through its desktop entry with application ID `io.github.zzwong.Diffz`.
Desktop/AppStream validation passed. Namcap reported warnings, including
libraries loaded dynamically and the absent maintainer tag, but no errors.

This was an isolated Arch userspace build, not an `extra-x86_64-build` run.
Logs, native reports and package output are under the checkout's ignored
`target/arch-validation/` and `target/arch-package/` directories.
Pointer dragging, mouse-wheel scrolling, X11 and accessibility have not been
verified. The host package installation requires administrator authentication.
