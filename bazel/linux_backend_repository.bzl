"""An alias-only repository for the first-party patched Linux backend.

The backend sources are owned by the main workspace. A repository containing
only the alias must not mirror that source directory or create source symlinks.
"""

def _linux_backend_repository_impl(ctx):
    ctx.file("BUILD.bazel", ctx.read(ctx.attr.build_file), executable = False)

linux_backend_repository = repository_rule(
    implementation = _linux_backend_repository_impl,
    attrs = {"build_file": attr.label(mandatory = True, allow_single_file = True)},
)
