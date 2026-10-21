# The vendored Linux backend is a Bazel package

The Cargo path patch at `vendor/gpui-pre-linux` is also an explicit Bazel
package. Keep its source ownership stable before and after crate_universe
regeneration. This fixes the clean-graph failure tracked in #148 (and the
preceding investigation in #146); it does not change the application's features
or compiler profile.

## Why the boundary matters

Pinned rules_rust 0.74.0 renders `BUILD.bazel` into this path dependency while
regenerating its dependency graph. Once that file exists, a root-package
`glob(["vendor/gpui-pre-linux/src/**/*.rs"])` cannot cross the new package
boundary. The old root label `//:vendor/gpui-pre-linux/Cargo.toml` is also invalid.
Previously materialized outputs can conceal this fresh-checkout failure.

The checked-in `vendor/gpui-pre-linux/BUILD.bazel` makes the boundary explicit
from the start. Its extra source groups come from
`bazel/gpui-linux-sources.BUILD.bazel`, supplied through the existing
`crate.annotation(..., additive_build_file = ...)`. Regeneration must retain
those groups. Edit the additive file, not just its generated copy.

The actual Rust library remains **`//:gpui_linux`**, as do the existing override
and downstream dependency identity. Its source and compile-data labels now point
into `//vendor/gpui-pre-linux`. The generated package's `gpui_linux` alias points
back at the existing Rust target; it does not compile a second library. Build
metadata files are excluded from `crate_data` so rewriting BUILD files does not
become an unrelated Rust compiler input.

## Regeneration and checks

With the pinned Cargo toolchain and locked dependencies available:

```sh
python3 scripts/bazel-vendor-linux.py
python3 scripts/tests/test_bazel_vendor_boundary.py -v
bash scripts/bazel-trial.sh build //:diffz --nobuild
```

The first command updates the dependency list, aliases and features in
`bazel/vendor_linux.bzl` from real Cargo metadata. The regression suite uses
small synthetic metadata to check source labels, optional/dev-dependency
filtering, preservation of aliases/features, deterministic rendering, and
failure without overwriting output when a new proc macro requires handling.
It is not a substitute for actual Bazel loading or compiler execution.

Inspect `git diff -- vendor/gpui-pre-linux/BUILD.bazel bazel/vendor_linux.bzl`
after real regeneration. An unexplained change is not a new accepted baseline.
Then build the desktop and run the existing core/extension tests and headless
fixture checks with equivalent flags and native inputs. Preserve a clean
baseline failure, the candidate result and any generated-source differences.

This work does not close #140's independent GPUI production/test feature
unification issue. Full graph execution does not establish remote cache reuse:
that still needs the fresh-consumer qualification in #119. Acquisition through
local repository overrides must be described separately from cached **action
outputs**, and processwrapper execution is not Linux namespace isolation.
