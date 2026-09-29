---
name: profile
description: Measure diffz's memory and CPU on macOS or Linux and compare a change against main. Use when a change claims a memory, CPU, wakeup or frame-buffer effect, when a PR needs before/after numbers, or when asked to profile, benchmark resource use, or record a baseline.
---

# Profile diffz

[docs/performance.md](../../../docs/performance.md) is the source of truth: the
scenarios, what each metric means, how to read footprint categories, and the
caveats. Read it before interpreting numbers. Run `scripts/profile-macos.sh` on
macOS or `scripts/profile-linux.sh` on Linux (`--help` lists the options).

## Rules

- **Quiet machine only.** Never run the profile while `cargo`, `rustc` or any
  other build or heavy job is running, and never start a build while it runs.
  The script refuses a busy machine; use `--wait SECONDS` rather than `--force`.
  A forced run's CPU numbers are noise, so say so if you report one.
- **Do not touch the diffz windows** while it runs; a covered window stops drawing.
- Compare footprint − IOSurface for allocator and cache work, IOSurface for frame
  buffers on macOS. On Linux compare PSS, private dirty, DRM memory when available,
  glibc heap in use and arena, CPU % and voluntary csw/s. These memory categories
  overlap; do not add them. Idle wakeups are not a macOS metric.

## A/B protocol

1. Build both release binaries first and copy each out of the target directory
   (`/tmp/diffz-main`, `/tmp/diffz-branch`).
2. With no builds running, run them back to back, main first:
   `bash scripts/profile-macos.sh --bin /tmp/diffz-main --label main`, then
   `bash scripts/profile-macos.sh --bin /tmp/diffz-branch --label branch --compare <target>/profile/main/summary.json`.
   On Linux use `profile-linux.sh` with the same arguments in one display-server
   session and record GPU and driver. Repeat separately under Wayland and Xorg.
3. Put the `--compare` table in the PR body. A change counts only when it moves
   beyond the old run's min–max spread (unmarked by ≈) and no other scenario
   regresses by more than its spread.

A full pass takes about 40 minutes; `--scenarios` and `--repeat` narrow it while
iterating, but the PR's table comes from a full `--repeat 3` run.
