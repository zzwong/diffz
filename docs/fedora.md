# Fedora validation

Validated September 7, 2026 on Fedora 44 x86_64 in GNOME Wayland, using the
pinned Rust 1.98.1 toolchain and Cargo.lock. Application source is the PR #4
result, now on main as `5db6999`.

## Verified

- Release build: `cargo build --locked --release -p diffz`.
- All 172 workspace tests, plus the explicitly enabled native Unicode
  integration test; formatting and strict workspace Clippy.
- Native geometry probes for Unicode, markdown prose, long URLs, and
  asymmetric split content, each at nine font/width combinations.
- Native Hebrew glyph order/caret edges, mixed-direction selections, native
  widths and mirrored parentheses. The original bidi rejection is fixed.
- A Unicode patch launch remained running without native paint errors.
- RPM and portable tarball creation, desktop/AppStream metadata validation,
  and checksum verification.
- Extracted RPM payload launched Wayland surfaces both with and without a
  patch file, using application ID `io.github.zzwong.Diffz`. This did not
  install the package into the host RPM database.

Build dependencies and launch commands are in the README. The validation
machine already had its native libraries installed.

## Remaining validation

Pointer/clipboard interaction, scrolling, window controls, file dialogs,
accessibility and X11 need separate checks on Fedora. Native geometry probes
alone do not establish those behaviors. Long-URL measurements varied with
build profile and host load; they are not end-to-end latency benchmarks.

## Reproduce

Run inside a graphical session:

```sh
cargo build --locked --release -p diffz
cargo test --locked --workspace --all-features
cargo test --locked -p diffz --test native_probe -- --ignored
./target/release/diffz --probe fixtures/unicode/after.txt \
  --probe-output target/unicode-fedora.json
```

Choose a fresh report path for each run; probes never overwrite reports.
Validation logs, reports and generated packages from this assessment are
retained under the Fedora checkout's ignored `target/pr4-validation/`.
