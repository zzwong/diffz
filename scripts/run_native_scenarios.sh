#!/usr/bin/env bash
# Produces native text-measurement probe output for several fixtures. A display is required.
set -euo pipefail
cd "$(dirname "$0")/.."
command -v cargo >/dev/null || { echo 'cargo is required; no native test ran.' >&2; exit 127; }
mkdir -p target/probes
run="$(date -u +%Y%m%dT%H%M%SZ)-$$"
for case in markdown-prose markdown-url unicode split-asymmetric; do
  cargo run --locked -p diffz -- \
    --probe "fixtures/$case/after.txt" \
    --probe-output "target/probes/$run-$case.json"
done
printf '%s\n' 'Probe output is in target/probes.'
