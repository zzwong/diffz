"""Isolate crate_universe's generated files from canonical Cargo sources.

rules_rust 0.74 emits BUILD.bazel into non-member path dependencies. Give its
Cargo workspace owned directories, copied manifests, and source
links. Rust targets continue to compile the canonical first-party sources.
"""

def _relative_manifest(path):
    if path.startswith("/") or "\\" in path or any([p in ["", ".", ".."] for p in path.split("/")]):
        fail("manifest destination must be a normalized relative path: " + path)
    if path.split("/")[-1] != "Cargo.toml":
        fail("manifest destination must end in Cargo.toml: " + path)
    return path

def _root_manifest(text, backend_path):
    # This narrowly scoped rewrite is deliberately fail-closed on manifest
    # layout changes. Never maintain a second dependency/version catalog.
    old = 'gpui-pre-linux = { path = "vendor/gpui-pre-linux" }'
    if len([line for line in text.splitlines() if line == old]) != 1:
        fail("expected the canonical gpui-pre-linux path patch exactly once")
    new = "gpui-pre-linux = { path = " + json.encode(str(backend_path)) + " }"
    return text.replace(old, new)

def _cargo_workspace_impl(ctx):
    destinations = {}
    for label, relative in ctx.attr.manifests.items():
        relative = _relative_manifest(relative)
        if relative in destinations:
            fail("duplicate manifest destination: " + relative)
        destinations[relative] = label
    if "Cargo.toml" not in destinations:
        fail("a root Cargo.toml is required")
    if "vendor/gpui-pre-linux/Cargo.toml" not in destinations:
        fail("the canonical Linux backend manifest is required")

    # Validate everything before writing. A package directory itself is never
    # symlinked: cargo-bazel may create BUILD.bazel beside its copied manifest.
    for relative in sorted(destinations):
        label = destinations[relative]
        content = ctx.read(label)
        if relative == "Cargo.toml":
            # An absolute owned path remains outside cargo-bazel's temporary
            # splice workspace. A relative path is rebased to the MAIN workspace
            # by rules_rust 0.74's LockfileAnnotation, defeating isolation.
            content = _root_manifest(content, ctx.path("vendor/gpui-pre-linux"))
        ctx.file(relative, content, executable = False)
        parent = relative.rpartition("/")[0]
        if parent:
            for child in ctx.path(label).dirname.readdir():
                if child.basename in ["Cargo.toml", "BUILD", "BUILD.bazel"]:
                    continue
                ctx.watch(child)
                if child.is_dir:
                    ctx.watch_tree(child)
                ctx.symlink(child, parent + "/" + child.basename)
    ctx.file("Cargo.lock", ctx.read(ctx.attr.lockfile), executable = False)
    # Do not export subpackage manifests here: cargo-bazel may generate a BUILD
    # beneath vendor/. from_cargo only needs the workspace root manifest.
    ctx.file("BUILD.bazel", 'exports_files(["Cargo.toml", "Cargo.lock"])\n', executable = False)

cargo_workspace = repository_rule(
    implementation = _cargo_workspace_impl,
    local = True,
    attrs = {
        "manifests": attr.label_keyed_string_dict(mandatory = True, allow_files = True),
        "lockfile": attr.label(mandatory = True, allow_single_file = True),
    },
)
