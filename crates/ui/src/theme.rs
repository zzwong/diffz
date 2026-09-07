//! Shared semantic colors for the interface and diff. This is the main place for visual tuning.
use diffz_core::palette::{Palette, Rgb};
use diffz_core::syntax::Token;
use gpui_kit::{Hsla, rgb};

fn c(rgb: Rgb) -> Hsla {
    gpui_kit::rgb(((rgb.0 as u32) << 16) | ((rgb.1 as u32) << 8) | rgb.2 as u32).into()
}

#[derive(Debug, Clone, Copy)]
pub struct Skin {
    pub base: Hsla,
    pub surface: Hsla,
    pub raised: Hsla,
    pub text: Hsla,
    pub muted: Hsla,
    pub border: Hsla,
    pub accent: Hsla,
    pub added: Hsla,
    pub removed: Hsla,
    pub added_word: Hsla,
    pub removed_word: Hsla,
    pub selection: Hsla,
    pub warning: Hsla,
    pub positive: Hsla,
    pub negative: Hsla,
    /// Color used for functions and methods in highlighted code.
    pub function: Hsla,
    /// Color used for highlighted types, namespaces, labels, and attributes.
    pub symbol: Hsla,
}
impl Skin {
    pub fn new(dark: bool) -> Self {
        if dark {
            Self {
                base: rgb(0x151820).into(),
                surface: rgb(0x1b1f29).into(),
                raised: rgb(0x262d3b).into(),
                text: rgb(0xe8eaf0).into(),
                muted: rgb(0x9ba5b5).into(),
                border: rgb(0x303848).into(),
                accent: rgb(0x88b4ff).into(),
                added: rgb(0x18332a).into(),
                removed: rgb(0x3c242a).into(),
                added_word: rgb(0x28583c).into(),
                removed_word: rgb(0x703845).into(),
                selection: rgb(0x385678).into(),
                warning: rgb(0xf0be72).into(),
                positive: rgb(0x81c995).into(),
                negative: rgb(0xf28b95).into(),
                function: rgb(0xd2a8ff).into(),
                symbol: rgb(0x6fd3c8).into(),
            }
        } else {
            Self {
                base: rgb(0xfafbfd).into(),
                surface: rgb(0xf0f3f7).into(),
                raised: rgb(0xe7edf4).into(),
                text: rgb(0x1c2633).into(),
                muted: rgb(0x5a687a).into(),
                border: rgb(0xcdd7e3).into(),
                accent: rgb(0x245fbb).into(),
                added: rgb(0xe1f1e6).into(),
                removed: rgb(0xfae7e8).into(),
                added_word: rgb(0xbce0c9).into(),
                removed_word: rgb(0xf2bfc5).into(),
                selection: rgb(0xb6d5fa).into(),
                warning: rgb(0x8c5809).into(),
                positive: rgb(0x237a45).into(),
                negative: rgb(0xc73745).into(),
                function: rgb(0x6f42c1).into(),
                symbol: rgb(0x0f766e).into(),
            }
        }
    }
    pub fn from_palette(p: &Palette) -> Skin {
        Skin {
            base: c(p.background),
            surface: c(p.lighter_background),
            raised: c(p.selection),
            text: c(p.foreground),
            muted: c(p.dark_foreground),
            border: c(p.muted),
            accent: c(p.accent),
            added: c(p.background.mix(p.green, 0.18)),
            removed: c(p.background.mix(p.red, 0.18)),
            added_word: c(p.background.mix(p.green, 0.40)),
            removed_word: c(p.background.mix(p.red, 0.40)),
            selection: c(p.selection),
            warning: c(p.yellow),
            positive: c(p.green),
            negative: c(p.red),
            function: c(p.magenta),
            symbol: c(p.cyan),
        }
    }
    pub fn token(self, t: Token) -> Hsla {
        match t {
            Token::Keyword => self.accent,
            Token::Function => self.function,
            Token::Type | Token::Namespace | Token::Attribute | Token::Label => self.symbol,
            Token::String | Token::Number | Token::Constant => self.warning,
            Token::Comment => self.muted,
            Token::Property
            | Token::Parameter
            | Token::Variable
            | Token::Operator
            | Token::Punctuation
            | Token::Embedded => self.text,
        }
    }
}

pub fn ui_font() -> &'static str {
    if cfg!(target_os = "macos") {
        ".SystemUIFont"
    } else {
        "DejaVu Sans"
    }
}
pub fn configure_typography(cx: &mut gpui_kit::App) {
    let theme = gpui_kit::component::Theme::global_mut(cx);
    theme.font_family = ui_font().into();
    theme.font_size = gpui_kit::px(13.);
    theme.mono_font_family = code_font().into();
}

pub fn code_font() -> &'static str {
    if cfg!(target_os = "macos") {
        "Menlo"
    } else {
        "DejaVu Sans Mono"
    }
}
