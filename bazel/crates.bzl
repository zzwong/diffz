"""Experimental all-feature desktop build and Cargo-equivalent test targets."""
load("@crates//:defs.bzl", "all_crate_deps", "aliases")
load("@rules_rust//rust:defs.bzl", "rust_library", "rust_binary", "rust_test")
load("//bazel:cargo_test.bzl", "cargo_test")

_MANIFESTS = ["//:Cargo.toml", "//:crates/core/Cargo.toml", "//:crates/adapters/Cargo.toml", "//:crates/ui/Cargo.toml", "//:crates/app/Cargo.toml"]
_FLAGS = ["-Copt-level=0", "-Cdebuginfo=1"] + select({
    "//bazel:cgu256": ["-Ccodegen-units=256"],
    "//conditions:default": ["-Ccodegen-units=16"],
})

def workspace_rustc_flags():
    return _FLAGS

def diffz_crate(name, package, crate_name, features, deps):
    common = dict(
        edition = "2024",
        crate_features = features,
        rustc_flags = _FLAGS,
        rustc_env = {"CARGO_MANIFEST_DIR": package, "CARGO_PKG_VERSION": "0.3.1"},
        aliases = aliases(package_name = package),
        proc_macro_deps = all_crate_deps(package_name = package, proc_macro = True, proc_macro_dev = True),
        deps = deps + all_crate_deps(package_name = package, normal = True, normal_dev = True),
        compile_data = _MANIFESTS + ["//:fixtures"] + native.glob([package + "/wit/**", package + "/src/**/*.sql", package + "/icons/**", "packaging/linux/**"], allow_empty = True),
    )
    source = package + "/src/" + ("main.rs" if name == "app" else "lib.rs")
    srcs = native.glob([package + "/src/**/*.rs"])
    rule = rust_binary if name == "app" else rust_library
    rule(name = "diffz" if name == "app" else name, crate_name = crate_name, crate_root = source, srcs = srcs, **common)
    rust_test(name = name + "_unit_bin", crate_name = crate_name, crate_root = source,
              srcs = srcs + native.glob([package + "/tests/support/**/*.rs"], allow_empty = True), tags = ["manual"], **common)
    cargo_test(name = name + "_unit", binary = ":" + name + "_unit_bin", data = ["//:fixtures"])
    tests = [":" + name + "_unit"]
    for source in native.glob([package + "/tests/*.rs"], allow_empty = True):
        stem = source.split("/")[-1][:-3]
        target = name + "_" + stem
        env = {"CARGO_MANIFEST_DIR": package, "CARGO_PKG_VERSION": "0.3.1"}
        if name == "app":
            env["CARGO_BIN_EXE_diffz"] = "diffz"
        rust_test(
            name = target + "_bin", crate_name = stem, edition = "2024", crate_root = source,
            srcs = [source] + native.glob([package + "/tests/support/**/*.rs"], allow_empty = True),
            crate_features = features,
            deps = deps + ([] if name == "app" else [":" + name]) + all_crate_deps(package_name = package, normal = True, normal_dev = True),
            proc_macro_deps = common["proc_macro_deps"], aliases = common["aliases"],
            rustc_flags = _FLAGS, rustc_env = env,
            compile_data = _MANIFESTS + ["//:fixtures"] + native.glob([package + "/tests/support/**"], allow_empty = True),
            tags = ["manual"], data = ["//:diffz"] if name == "app" else [],
        )
        cargo_test(name = target, binary = ":" + target + "_bin", data = ["//:fixtures"])
        tests.append(":" + target)
    return tests
