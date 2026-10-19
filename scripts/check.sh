#!/usr/bin/env bash
# Local checks. With `core`, the GPUI build is skipped; `native` runs every gate.
set -euo pipefail
cd "$(dirname "$0")/.."
mode="${1:-core}"
case "$mode" in core|native) ;; *) echo 'Usage: scripts/check.sh core|native' >&2; exit 2 ;; esac
command -v cargo >/dev/null || { echo 'cargo is not installed; Rust tests were NOT run.' >&2; exit 127; }
cargo fmt --all -- --check
# Resolve the feature-free graph once and let Cargo schedule all test binaries.
# Keep syntax and wasm separate below so all three feature configurations run.
cargo test --locked -p diffz-core -p diffz-adapters -p diffz --no-default-features
cargo test --locked -p diffz-core --features syntax
cargo test --locked -p diffz-core --features wasm
cargo clippy --locked -p diffz-core -p diffz-adapters -p diffz --no-default-features --all-targets -- -D warnings
if [[ "$mode" == native ]]; then
  cargo test --locked --workspace --all-features
  cargo clippy --locked --workspace --all-features --all-targets -- -D warnings
  cargo build --locked -p diffz
  cargo build --locked --release -p diffz --features syntax
fi
