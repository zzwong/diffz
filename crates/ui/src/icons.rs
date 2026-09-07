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
}
impl AppIcon {
    pub const ALL: [AppIcon; 4] = [
        AppIcon::WrapText,
        AppIcon::Columns2,
        AppIcon::Rows2,
        AppIcon::MessageSquare,
    ];
    pub fn path(self) -> &'static str {
        match self {
            AppIcon::WrapText => "icons/diffz/wrap-text.svg",
            AppIcon::Columns2 => "icons/diffz/columns-2.svg",
            AppIcon::Rows2 => "icons/diffz/rows-2.svg",
            AppIcon::MessageSquare => "icons/diffz/message-square.svg",
        }
    }
    fn bytes(path: &str) -> Option<&'static [u8]> {
        Some(match path {
            "icons/diffz/wrap-text.svg" => include_bytes!("../icons/wrap-text.svg"),
            "icons/diffz/columns-2.svg" => include_bytes!("../icons/columns-2.svg"),
            "icons/diffz/rows-2.svg" => include_bytes!("../icons/rows-2.svg"),
            "icons/diffz/message-square.svg" => include_bytes!("../icons/message-square.svg"),
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
}
