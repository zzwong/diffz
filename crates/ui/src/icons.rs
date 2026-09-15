//! Icons bundled with diffz itself; they are served before anything gpui-kit bundles, so
//! toolbar glyphs keep one visual style, never dropping into mismatching text fonts.
use gpui_kit::component::Icon;
use gpui_kit::{AssetSource, Result, SharedString};
use std::borrow::Cow;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AppIcon {
    WrapText,
    Columns2,
    Rows2,
    MessageSquare,
    File,
    FileRust,
    FileCode,
    FileData,
    FileDocs,
    FileShell,
    FileImage,
}
impl AppIcon {
    pub const ALL: [AppIcon; 11] = [
        AppIcon::WrapText,
        AppIcon::Columns2,
        AppIcon::Rows2,
        AppIcon::MessageSquare,
        AppIcon::File,
        AppIcon::FileRust,
        AppIcon::FileCode,
        AppIcon::FileData,
        AppIcon::FileDocs,
        AppIcon::FileShell,
        AppIcon::FileImage,
    ];
    pub fn for_path(path: &str) -> Self {
        let extension = std::path::Path::new(path)
            .extension()
            .and_then(|extension| extension.to_str())
            .map(str::to_ascii_lowercase);
        match extension.as_deref() {
            Some("rs") => Self::FileRust,
            Some("js" | "jsx" | "mjs" | "cjs" | "ts" | "tsx" | "mts" | "cts") => Self::FileCode,
            Some("json" | "toml" | "yaml" | "yml") => Self::FileData,
            Some("md" | "markdown" | "mdx" | "txt") => Self::FileDocs,
            Some("sh" | "bash" | "zsh") => Self::FileShell,
            Some("png" | "jpg" | "jpeg" | "gif" | "svg" | "webp") => Self::FileImage,
            _ => Self::File,
        }
    }
    pub fn path(self) -> &'static str {
        match self {
            AppIcon::WrapText => "icons/diffz/wrap-text.svg",
            AppIcon::Columns2 => "icons/diffz/columns-2.svg",
            AppIcon::Rows2 => "icons/diffz/rows-2.svg",
            AppIcon::MessageSquare => "icons/diffz/message-square.svg",
            AppIcon::File => "icons/diffz/file.svg",
            AppIcon::FileRust => "icons/diffz/file-rust.svg",
            AppIcon::FileCode => "icons/diffz/file-code.svg",
            AppIcon::FileData => "icons/diffz/file-data.svg",
            AppIcon::FileDocs => "icons/diffz/file-docs.svg",
            AppIcon::FileShell => "icons/diffz/file-shell.svg",
            AppIcon::FileImage => "icons/diffz/file-image.svg",
        }
    }
    fn bytes(path: &str) -> Option<&'static [u8]> {
        Some(match path {
            "icons/diffz/wrap-text.svg" => include_bytes!("../icons/wrap-text.svg"),
            "icons/diffz/columns-2.svg" => include_bytes!("../icons/columns-2.svg"),
            "icons/diffz/rows-2.svg" => include_bytes!("../icons/rows-2.svg"),
            "icons/diffz/message-square.svg" => include_bytes!("../icons/message-square.svg"),
            "icons/diffz/file.svg" => include_bytes!("../icons/file.svg"),
            "icons/diffz/file-rust.svg" => include_bytes!("../icons/file-rust.svg"),
            "icons/diffz/file-code.svg" => include_bytes!("../icons/file-code.svg"),
            "icons/diffz/file-data.svg" => include_bytes!("../icons/file-data.svg"),
            "icons/diffz/file-docs.svg" => include_bytes!("../icons/file-docs.svg"),
            "icons/diffz/file-shell.svg" => include_bytes!("../icons/file-shell.svg"),
            "icons/diffz/file-image.svg" => include_bytes!("../icons/file-image.svg"),
            _ => return None,
        })
    }
}
impl From<AppIcon> for Icon {
    fn from(icon: AppIcon) -> Icon {
        Icon::default().path(icon.path())
    }
}
/// Asset source for the app: diffz's icons win, gpui-kit's fill in the rest.
pub struct Assets;
impl AssetSource for Assets {
    fn load(&self, path: &str) -> Result<Option<Cow<'static, [u8]>>> {
        if let Some(bytes) = AppIcon::bytes(path) {
            return Ok(Some(Cow::Borrowed(bytes)));
        }
        gpui_kit::assets::Assets.load(path)
    }
    fn list(&self, path: &str) -> Result<Vec<SharedString>> {
        gpui_kit::assets::Assets.list(path)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn every_app_icon_resolves_to_an_svg() {
        for icon in AppIcon::ALL {
            let bytes = Assets
                .load(icon.path())
                .unwrap()
                .unwrap_or_else(|| panic!("{}", icon.path()));
            assert!(bytes.starts_with(b"<svg"), "{}", icon.path());
        }
    }
    #[test]
    fn kit_icons_still_load() {
        assert!(Assets.load("icons/check.svg").unwrap().is_some());
    }

    #[test]
    fn file_paths_select_the_compact_file_type_glyph() {
        for (expected, paths) in [
            ("icons/diffz/file-rust.svg", [".rs"].as_slice()),
            (
                "icons/diffz/file-code.svg",
                [".js", ".jsx", ".mjs", ".cjs", ".ts", ".tsx", ".mts", ".cts"].as_slice(),
            ),
            (
                "icons/diffz/file-data.svg",
                [".json", ".toml", ".yaml", ".yml"].as_slice(),
            ),
            (
                "icons/diffz/file-docs.svg",
                [".md", ".markdown", ".mdx", ".txt"].as_slice(),
            ),
            (
                "icons/diffz/file-shell.svg",
                [".sh", ".bash", ".zsh"].as_slice(),
            ),
            (
                "icons/diffz/file-image.svg",
                [".png", ".jpg", ".jpeg", ".gif", ".svg", ".webp"].as_slice(),
            ),
        ] {
            for extension in paths {
                assert_eq!(
                    AppIcon::for_path(&format!("src/example{extension}")).path(),
                    expected
                );
            }
        }
    }

    #[test]
    fn unknown_and_extensionless_paths_use_the_generic_file_glyph() {
        for path in ["README", "src/example", "src/example.bin"] {
            assert_eq!(AppIcon::for_path(path).path(), "icons/diffz/file.svg");
        }
    }
}
