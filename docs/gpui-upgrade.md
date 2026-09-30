# GPUI upgrade and visual magnification

## Current decision

Keep the Zed-derived `gpui-pre` stack while evaluating framework migrations
separately. GPUI-CE supplies pinch input, but its current `Window` implementation
does not supply diffz's whole-window visual magnification or live raster policy.
GPUI Kit pins the Zed-derived snapshot, so CE is not a dependency-only replacement.

## Zoom polish

The `codex/magnification-polish` branch of `zzwong/gpui-pre` maps accessibility
rectangles to magnified window/device coordinates and fits anchored menus and
tooltips against the visible content viewport. Layout and accessibility click
targets remain in content coordinates.

Focused magnification, anchored-menu, and exact-live-cache tests pass on both
the 0.3.3 baseline and the port to 0.3.7. These are headless tests, not native
screen-reader or high-refresh rendering validation. Oversized overlays can still
exceed the visible viewport; fitting position alone does not resize their content.

## Snapshot port

`zzwong/gpui-pre` branch `codex/gpui-pre-0.3.7` imports the published 0.3.7
snapshot (Zed `1a28cff4b409169bac058bca40dfbfeb7621d19b`) and carries the
magnification, exact-live-raster, and viewport/accessibility polish patches.
The port adapts glyph caching to the upstream owned atlas-key API.

This branch is not yet the dependency used by diffz. GPUI Kit 0.7.0 pins
gpui-pre 0.3.7 and changes Root/window/overlay hosting, so upgrading only the
core crate would not produce a compatible application dependency graph.

## Before switching diffz to 0.3.7

- Port and verify Vulkan-first startup and on-demand path textures in WGPU.
- Port and verify the vendored X11 frame-demand patch.
- Port the macOS display-link idle fix and check it on macOS.
- Check the caret-idle patch against GPUI Base 0.7.0; retain behavior not supplied
  by the newer caret lifecycle fixes.
- Upgrade GPUI Kit and its complete snapshot family together; migrate window
  creation and overlay hosting according to the 0.7.0 release notes.
- Run workspace tests, Clippy, native CI, and visible pinch/reversal checks.
- Check accessibility highlighting/clicks, IME positioning, menus, tooltips,
  resizing, window chrome, and scale-factor changes while magnified.

Any native diffz test on this machine must use a leased unused workspace and
silent placement without changing the user's active workspace or focus.

## Upstream references

- Pinch input: https://github.com/zed-industries/zed/pull/47351 and
  https://github.com/zed-industries/zed/pull/51354
- Atlas changes: https://github.com/zed-industries/zed/pull/64331
- Renderer benchmark sessions: https://github.com/zed-industries/zed/pull/64109
- Kit migration: https://github.com/longbridge/gpui-kit/releases/tag/v0.7.0
- Community Edition: https://github.com/gpui-ce/gpui-ce

Introduce whole-window magnification upstream as a framework design proposal;
do not describe existing pinch input support as new work. Include the cache
lifetime/raster quality separation and benchmark limitations. Native platform
validation and overlay sizing remain outstanding before claiming a fully
production-ready upstream implementation.
