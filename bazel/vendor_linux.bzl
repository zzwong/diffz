"""Pinned dependencies for the existing GPUI Linux source patch."""
load("@rules_rust//rust:defs.bzl", "rust_library")

def vendor_linux():
    # The existing vendored GPUI Linux patch, with dependencies resolved by Crate Universe.
    rust_library(
        name = "gpui_linux",
        visibility = ["//visibility:public"],
        crate_name = "gpui_linux",
        edition = "2024",
        crate_root = "vendor/gpui-pre-linux/src/gpui_linux.rs",
        srcs = native.glob(["vendor/gpui-pre-linux/src/**/*.rs"]),
        compile_data = ["Cargo.toml", "crates/core/Cargo.toml", "crates/adapters/Cargo.toml", "crates/ui/Cargo.toml", "crates/app/Cargo.toml"] + native.glob(["vendor/gpui-pre-linux/**"]),
        rustc_flags = ["-Cdebuginfo=1"],
        rustc_env = {"CARGO_MANIFEST_DIR": "vendor/gpui-pre-linux", "CARGO_PKG_VERSION": "0.3.3"},
        deps = ['@@rules_rust++crate+crates__accesskit-0.24.1//:accesskit', '@@rules_rust++crate+crates__accesskit_unix-0.22.1//:accesskit_unix', '@@rules_rust++crate+crates__anyhow-1.0.104//:anyhow', '@@rules_rust++crate+crates__as-raw-xcb-connection-1.0.1//:as_raw_xcb_connection', '@@rules_rust++crate+crates__ashpd-0.13.13//:ashpd', '@@rules_rust++crate+crates__bitflags-2.13.1//:bitflags', '@@rules_rust++crate+crates__bytemuck-1.25.2//:bytemuck', '@@rules_rust++crate+crates__calloop-0.14.4//:calloop', '@@rules_rust++crate+crates__calloop-wayland-source-0.4.1//:calloop_wayland_source', '@@rules_rust++crate+crates__filedescriptor-0.8.3//:filedescriptor', '@@rules_rust++crate+crates__futures-0.3.34//:futures', '@@rules_rust++crate+crates__gpui-pre-0.3.3//:gpui', '@@rules_rust++crate+crates__gpui-pre-collections-0.3.3//:collections', '@@rules_rust++crate+crates__gpui-pre-http-client-0.3.3//:http_client', '@@rules_rust++crate+crates__gpui-pre-util-0.3.3//:gpui_util', '@@rules_rust++crate+crates__gpui-pre-wgpu-0.3.3//:gpui_wgpu', '@@rules_rust++crate+crates__libc-0.2.189//:libc', '@@rules_rust++crate+crates__log-0.4.34//:log', '@@rules_rust++crate+crates__notify-rust-4.18.0//:notify_rust', '@@rules_rust++crate+crates__oo7-0.6.0//:oo7', '@@rules_rust++crate+crates__open-5.4.3//:open', '@@rules_rust++crate+crates__parking_lot-0.12.5//:parking_lot', '@@rules_rust++crate+crates__raw-window-handle-0.6.2//:raw_window_handle', '@@rules_rust++crate+crates__smallvec-1.15.2//:smallvec', '@@rules_rust++crate+crates__smol-2.0.2//:smol', '@@rules_rust++crate+crates__strum-0.28.0//:strum', '@@rules_rust++crate+crates__url-2.5.8//:url', '@@rules_rust++crate+crates__uuid-1.26.0//:uuid', '@@rules_rust++crate+crates__wayland-backend-0.3.17//:wayland_backend', '@@rules_rust++crate+crates__wayland-client-0.31.15//:wayland_client', '@@rules_rust++crate+crates__wayland-cursor-0.31.14//:wayland_cursor', '@@rules_rust++crate+crates__wayland-protocols-0.32.13//:wayland_protocols', '@@rules_rust++crate+crates__wayland-protocols-plasma-0.3.12//:wayland_protocols_plasma', '@@rules_rust++crate+crates__wayland-protocols-wlr-0.3.12//:wayland_protocols_wlr', '@@rules_rust++crate+crates__x11-clipboard-0.9.3//:x11_clipboard', '@@rules_rust++crate+crates__x11rb-0.13.2//:x11rb', '@@rules_rust++crate+crates__xkbcommon-0.8.0//:xkbcommon', '@@rules_rust++crate+crates__zed-xim-0.4.0-zed//:zed_xim'],
        aliases = {'@@rules_rust++crate+crates__zed-xim-0.4.0-zed//:zed_xim': 'xim'},
        crate_features = ['as-raw-xcb-connection', 'ashpd', 'bitflags', 'calloop-wayland-source', 'filedescriptor', 'gpui_wgpu', 'open', 'wayland', 'wayland-backend', 'wayland-client', 'wayland-cursor', 'wayland-protocols', 'wayland-protocols-plasma', 'wayland-protocols-wlr', 'x11', 'x11-clipboard', 'x11rb', 'xim', 'xkbcommon'],
    )
