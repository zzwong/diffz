//! Shared semantic colors for the interface and diff.
use diffz_core::palette::{Mode, Rgb};
use diffz_core::syntax::Token;
use diffz_core::theme::{SkinSpec, Theme};
use gpui_kit::Hsla;

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
    tokens: [Hsla; Token::ALL.len()],
}
impl Skin {
    pub fn new(dark: bool) -> Self {
        let spec = SkinSpec::builtin(if dark { Mode::Dark } else { Mode::Light });
        Self::build(&spec, |t| spec.token(t))
    }
    pub fn from_theme(theme: &Theme) -> Skin {
        Self::build(&theme.skin, |t| theme.token(t))
    }
    fn build(s: &SkinSpec, token: impl Fn(Token) -> Rgb) -> Skin {
        Skin {
            base: c(s.base),
            surface: c(s.surface),
            raised: c(s.raised),
            text: c(s.text),
            muted: c(s.muted),
            border: c(s.border),
            accent: c(s.accent),
            added: c(s.added),
            removed: c(s.removed),
            added_word: c(s.added_word),
            removed_word: c(s.removed_word),
            selection: c(s.selection),
            warning: c(s.warning),
            positive: c(s.positive),
            negative: c(s.negative),
            function: c(s.function),
            symbol: c(s.symbol),
            tokens: Token::ALL.map(|t| c(token(t))),
        }
    }
    pub fn token(self, t: Token) -> Hsla {
        self.tokens[t as usize]
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn syntax_tokens_use_their_semantic_palette_colors() {
        let skin = Skin::new(true);

        for token in [Token::Keyword, Token::Operator] {
            assert_eq!(skin.token(token), skin.accent);
        }
        assert_eq!(skin.token(Token::Function), skin.function);
        for token in [
            Token::Type,
            Token::Namespace,
            Token::Attribute,
            Token::Label,
            Token::Property,
        ] {
            assert_eq!(skin.token(token), skin.symbol);
        }
        assert_eq!(skin.token(Token::String), skin.positive);
        for token in [Token::Number, Token::Constant] {
            assert_eq!(skin.token(token), skin.warning);
        }
        for token in [Token::Comment, Token::Punctuation] {
            assert_eq!(skin.token(token), skin.muted);
        }
        for token in [Token::Variable, Token::Parameter, Token::Embedded] {
            assert_eq!(skin.token(token), skin.text);
        }
    }

    #[test]
    fn theme_syntax_overrides_replace_only_their_token() {
        let theme = Theme::parse(
            "[syntax]\ncomment = \"#ff0000\"\n",
            std::path::Path::new("."),
        )
        .unwrap();
        let skin = Skin::from_theme(&theme);
        assert_eq!(skin.token(Token::Comment), c(Rgb(255, 0, 0)));
        assert_eq!(skin.token(Token::Punctuation), skin.muted);
    }
}
