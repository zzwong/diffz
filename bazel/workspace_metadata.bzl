"""Read the workspace release version so Cargo and Bazel report the same version."""

def _workspace_metadata_impl(ctx):
    in_package = False
    for raw_line in ctx.read(ctx.attr.manifest).splitlines():
        line = raw_line.strip()
        if line.startswith("["):
            in_package = line == "[workspace.package]"
        elif in_package and "=" in line:
            key, value = line.split("=", 1)
            if key.strip() == "version":
                value = value.strip()
                if not value.startswith('"') or '"' not in value[1:]:
                    fail("workspace.package.version must be a quoted string")
                version = value[1:].split('"', 1)[0]
                ctx.file("metadata.bzl", "PACKAGE_VERSION = %r\n" % version)
                ctx.file("BUILD.bazel", 'exports_files(["metadata.bzl"])\n')
                return
    fail("Cargo.toml is missing workspace.package.version")

workspace_metadata = repository_rule(
    implementation = _workspace_metadata_impl,
    attrs = {"manifest": attr.label(mandatory = True, allow_single_file = True)},
    local = True,
)
