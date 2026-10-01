# GPUI upgrade status

## Branch and revision policy

Keep `zzwong/gpui-pre`'s `diffz/main` as the single integration branch. The
accepted integration remains on the 0.3.3 snapshot; the 0.3.7 port is a review
candidate and is not merged or released. Diffz pins the candidate by exact Git
revision, `c4b1252378b02877d1f035d5fd1d06d0e8c4b1ab` (tree
`dfca58e437addb1a58212fe2fb976e159e42067d`). The GPUI review branch head is
`6477260ac00e66b030eff57d5826a3f79bc40592`; its tree matches the pinned code.
GitHub automatically deletes merged PR branches; archive tags preserve the
older 0.3.3 snapshots.

The former 0.3.3 work branches are preserved as archive tags:

- `archive/0.3.3-upstream-baseline` — `7746cf44a006f2320cf062818f352bcc6f9444e1`
- `archive/0.3.3-visual-zoom` — `38a1ce299869b9e84967444a46755c043e8338aa`
- `archive/0.3.3-live-pinch-exact-raster` — `ee8aad5ffbfa82e6ffedc18eb032b199d6c15c82`
- `archive/0.3.3-magnification-polish` — `286f1e1313f21ebe213ca0b1f6fed390be7160c1`

The 0.3.7 port imports the published snapshot, then carries visual
magnification, exact live-raster behavior, and viewport/accessibility polish.
The diffz candidate also ports the app to GPUI Kit 0.7.0 and the complete 0.3.7
snapshot family. Keep the candidate pin exact until the upgrade is accepted.
The supporting pins preserve Vulkan-first startup, lazy path textures, idle
caret handling, macOS display-link demand/idle handling, and the X11
frame-demand patch.

## Validation record

On Linux, app commit `6affe7e9119b0034363a25b86cb24c2a2fabe3e7` (tree
`5748dcf50404d8736e90cea693c348f29622cfb3`) passed:

- `cargo fmt --all -- --check`
- `cargo test --locked --workspace --all-features` (including 95 UI tests; two
  benchmark tests and the separate native probe were ignored)
- `cargo clippy --locked --workspace --all-features --all-targets -- -D warnings`
- `cargo build --locked --release -p diffz`

Four GitHub checks passed at this app commit. Automated test and lint checks
run on Ubuntu; the macOS packaging job is not native runtime validation, and
there is no Windows runtime result.

The release binary is
`/home/aaron/Projects/diffz/target/release/diffz`, SHA-256
`89d9c331929616617f2c0e98acf4518fa32341c5c45fd639ad81055616a122e1`
(63,528,368 bytes). It was built from the app commit above with the exact GPUI
revision and tree recorded above.

## Linux Wayland smoke

Two short native Wayland launches ran under Hyprland 0.56.2 in a leased unused
workspace. Per-launch rules placed the client silently and suppressed focus
activation; the user's active workspace and focused window were unchanged
before and after, and the lease was released. Both clients were native Wayland
(not Xwayland) and reported `first render`, `snapshot opened`, and
`first content paint` without app errors:

- Normal launch: fixture F01, with `DIFFZ_DEBUG_MAGNIFY` unset; initial client
  geometry was 1050×750 at (150, 90).
- Magnified launch: fixture F01 with `DIFFZ_DEBUG_MAGNIFY=2@100,100` and
  `DIFFZ_TIMING=1`; initial geometry was 1050×750 at (150, 90). A second
  invocation handed off fixture F02 and returned `status: handed_off`. The
  client remained alive. Resizing that client produced compositor geometry
  900×650 at (225, 140).

These off-workspace runs confirm startup, fixture open/handoff, and compositor
geometry only. They do not verify presented pixels, visible magnification or
pinch gestures, repaint/exposure behavior, or frame pacing. Logs and before,
handoff, resize, and cleanup state captures are in
`/home/aaron/.cache/diffz-gpui-upgrade/native-wayland-20261001T015702Z/`.

## Remaining native checks

- Verify visible magnification, pinch/reversal, repaint after exposure, and
  scale-factor changes on a user-visible surface.
- Check accessibility highlighting and clicks, IME positioning, menus,
  tooltips, resizing, and window chrome while magnified.
- Validate macOS display-link behavior on native macOS.
- Check caret visibility immediately after programmatic focus of an active
  input across platforms. This remains a follow-up; no fork-specific regression
  is established.
- Validate X11 idle/wake behavior in a real Xorg session. No Windows runtime
  validation has been performed.

Any further native diffz test on this machine must use a leased unused
workspace and silent placement without changing the user's active workspace or
focus.

## Upstream references

- Zed pinch input: https://github.com/zed-industries/zed/pull/47351 and
  https://github.com/zed-industries/zed/pull/51354
- Atlas changes: https://github.com/zed-industries/zed/pull/64331
- GPUI Kit 0.7.0: https://github.com/longbridge/gpui-kit/releases/tag/v0.7.0
- GPUI Community Edition: https://github.com/gpui-ce/gpui-ce

Treat whole-window magnification as a separate upstream design proposal:
existing pinch input support is not itself new work, and native platform and
visible interaction checks remain outstanding.
