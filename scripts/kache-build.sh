#!/usr/bin/env bash
# Opt-in Cargo build wrapper with a pinned, local-only Kache installation.
set -euo pipefail

repo_root="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd -P)"
cd "$repo_root"

readonly kache_version="0.21.0"
readonly kache_root="$repo_root/target/kache/$kache_version"
readonly kache_bin="$kache_root/bin/kache"
readonly kache_config="$repo_root/scripts/kache.toml"
readonly cargo_config="$repo_root/scripts/kache-cargo.toml"
readonly bootstrap_cargo_config="$repo_root/scripts/kache-bootstrap-cargo.toml"
readonly kache_runtime_dir="$repo_root/target/kache/runtime"
readonly kache_socket="$kache_runtime_dir/daemon.sock"

usage() {
  cat <<'USAGE'
Usage: bash scripts/kache-build.sh [--dry-run] [cargo build flags...]

Build diffz through the pinned local Kache wrapper. The first invocation
installs Kache into target/kache/0.21.0. --dry-run checks the installation and
prints the command and cache policy without compiling diffz.
USAGE
}

if [[ "${1:-}" == "--help" || "${1:-}" == "-h" ]]; then
  usage
  exit 0
fi

dry_run=false
if [[ "${1:-}" == "--dry-run" ]]; then
  dry_run=true
  shift
fi

command -v cargo >/dev/null || {
  echo 'cargo is required to install Kache and build diffz.' >&2
  exit 127
}

if [[ ! -r "$kache_config" || ! -r "$cargo_config" || ! -r "$bootstrap_cargo_config" ]]; then
  echo "Pinned Kache/Cargo config is missing or unreadable." >&2
  exit 1
fi

if [[ ! -e "$kache_bin" ]]; then
  mkdir -p "$kache_root"
  echo "Installing pinned kache $kache_version into $kache_root ..." >&2
  if ! RUSTC_WRAPPER='' RUSTC_WORKSPACE_WRAPPER='' \
    CARGO_BUILD_RUSTC_WRAPPER='' CARGO_BUILD_RUSTC_WORKSPACE_WRAPPER='' cargo install \
    --config "$bootstrap_cargo_config" \
    --locked --force --version "$kache_version" --root "$kache_root" kache; then
    echo "Unable to install pinned kache $kache_version; the exact version is required." >&2
    exit 1
  fi
fi

if [[ ! -x "$kache_bin" ]]; then
  echo "Expected an executable pinned kache at $kache_bin." >&2
  exit 1
fi

installed_version="$("$kache_bin" --version 2>/dev/null || true)"
if [[ "$installed_version" != "kache $kache_version" ]]; then
  echo "Expected kache $kache_version at $kache_bin, got:" \
    "${installed_version:-no version output}." >&2
  exit 1
fi

if [[ "$dry_run" == true ]]; then
  echo "kache: $installed_version"
  echo "config: $kache_config"
  echo "cargo config: $cargo_config"
  echo 'cache: local-only; 8GiB GC budget for registered cache-blob bytes'
  echo "runtime: $kache_runtime_dir"
  echo "socket: $kache_socket"
  printf "command: cargo --config 'build.rustc-workspace-wrapper=\"\"' build"
  if (($# > 0)); then
    printf ' %q' "$@"
  fi
  echo
  exit 0
fi

unset RUSTC_WRAPPER RUSTC_WORKSPACE_WRAPPER
unset CARGO_BUILD_RUSTC_WRAPPER CARGO_BUILD_RUSTC_WORKSPACE_WRAPPER

exec cargo --config "$cargo_config" --config 'build.rustc-workspace-wrapper=""' build "$@"
