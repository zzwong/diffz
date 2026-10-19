"""Run Rust tests with a short, isolated on-disk temporary directory."""
def _cargo_test_impl(ctx):
    launcher = ctx.actions.declare_file(ctx.label.name + ".sh")
    ctx.actions.write(launcher, """#!/usr/bin/env bash
set -euo pipefail
export TMPDIR="$(mktemp -d "$DIFFZ_TEST_TMP_ROOT/test.XXXXXX")"
trap 'rm -rf "$TMPDIR"' EXIT
root="${TEST_SRCDIR}/${TEST_WORKSPACE}"
cd "$root"
"$root/%s" "$@"
""" % ctx.executable.binary.short_path, is_executable = True)
    runfiles = ctx.runfiles(files = [ctx.executable.binary] + ctx.files.data)
    runfiles = runfiles.merge(ctx.attr.binary[DefaultInfo].default_runfiles)
    return [DefaultInfo(executable = launcher, runfiles = runfiles)]

cargo_test = rule(
    implementation = _cargo_test_impl,
    test = True,
    attrs = {
        "binary": attr.label(executable = True, cfg = "target", mandatory = True),
        "data": attr.label_list(allow_files = True),
    },
)
