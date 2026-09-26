use crate::{
    palette::{Mode, Palette, Rgb},
    syntax::Token,
};
use std::{
    fs,
    path::{Path, PathBuf},
};

macro_rules! skin_spec {
    ($($field:ident),* $(,)?) => {
        #[derive(Debug, Clone, Copy, PartialEq, Eq)]
        pub struct SkinSpec {
            $(pub $field: Rgb,)*
        }
        impl SkinSpec {
            pub const KEYS: &[&str] = &[$(stringify!($field)),*];
            fn slot(&mut self, key: &str) -> Option<&mut Rgb> {
                match key {
                    $(stringify!($field) => Some(&mut self.$field),)*
                    _ => None,
                }
            }
        }
    };
}
skin_spec!(
    base,
    surface,
    raised,
    text,
    muted,
    border,
    accent,
    added,
    removed,
    added_word,
    removed_word,
    selection,
    warning,
    positive,
    negative,
    function,
    symbol,
);

impl SkinSpec {
    pub fn builtin(mode: Mode) -> Self {
        let c = |hex: u32| Rgb((hex >> 16) as u8, (hex >> 8) as u8, hex as u8);
        match mode {
            Mode::Dark => Self {
                base: c(0x151820),
                surface: c(0x1b1f29),
                raised: c(0x262d3b),
                text: c(0xe8eaf0),
                muted: c(0x9ba5b5),
                border: c(0x303848),
                accent: c(0x88b4ff),
                added: c(0x18332a),
                removed: c(0x3c242a),
                added_word: c(0x28583c),
                removed_word: c(0x703845),
                selection: c(0x385678),
                warning: c(0xf0be72),
                positive: c(0x81c995),
                negative: c(0xf28b95),
                function: c(0xd2a8ff),
                symbol: c(0x6fd3c8),
            },
            Mode::Light => Self {
                base: c(0xfafbfd),
                surface: c(0xf0f3f7),
                raised: c(0xe7edf4),
                text: c(0x1c2633),
                muted: c(0x5a687a),
                border: c(0xcdd7e3),
                accent: c(0x245fbb),
                added: c(0xe1f1e6),
                removed: c(0xfae7e8),
                added_word: c(0xbce0c9),
                removed_word: c(0xf2bfc5),
                selection: c(0xb6d5fa),
                warning: c(0x8c5809),
                positive: c(0x237a45),
                negative: c(0xc73745),
                function: c(0x6f42c1),
                symbol: c(0x0f766e),
            },
        }
    }

    pub fn from_palette(p: &Palette) -> Self {
        Self {
            base: p.background,
            surface: p.lighter_background,
            raised: p.selection,
            text: p.foreground,
            muted: p.dark_foreground,
            border: p.muted,
            accent: p.accent,
            added: p.background.mix(p.green, 0.18),
            removed: p.background.mix(p.red, 0.18),
            added_word: p.background.mix(p.green, 0.40),
            removed_word: p.background.mix(p.red, 0.40),
            selection: p.selection,
            warning: p.yellow,
            positive: p.green,
            negative: p.red,
            function: p.magenta,
            symbol: p.cyan,
        }
    }

    pub fn token(&self, t: Token) -> Rgb {
        match t {
            Token::Keyword | Token::Operator => self.accent,
            Token::Function => self.function,
            Token::Type | Token::Namespace | Token::Attribute | Token::Label | Token::Property => {
                self.symbol
            }
            Token::String => self.positive,
            Token::Number | Token::Constant => self.warning,
            Token::Comment | Token::Punctuation => self.muted,
            Token::Variable | Token::Parameter | Token::Embedded => self.text,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Theme {
    pub mode: Mode,
    pub palette: Option<Palette>,
    pub skin: SkinSpec,
    syntax: [Option<Rgb>; Token::ALL.len()],
    pub files: Vec<PathBuf>,
    pub warnings: Vec<String>,
}

impl Theme {
    pub fn from_palette(palette: Palette) -> Self {
        Self {
            mode: palette.mode,
            skin: SkinSpec::from_palette(&palette),
            palette: Some(palette),
            syntax: [None; Token::ALL.len()],
            files: vec![],
            warnings: vec![],
        }
    }

    pub fn token(&self, t: Token) -> Rgb {
        self.syntax[t as usize].unwrap_or_else(|| self.skin.token(t))
    }

    pub fn load(path: &Path) -> Result<Theme, String> {
        let file = if path.is_dir() {
            theme_file(path).ok_or_else(|| format!("no theme in {}", path.display()))?
        } else {
            path.to_path_buf()
        };
        let text = fs::read_to_string(&file)
            .map_err(|e| format!("cannot read {}: {e}", file.display()))?;
        let mut theme = if file.file_name().is_some_and(|n| n == "theme.toml") {
            Self::parse(&text, file.parent().unwrap_or(Path::new(".")))?
        } else {
            Self::from_palette(Palette::parse(&text)?)
        };
        theme.files.insert(0, file);
        Ok(theme)
    }

    pub fn parse(text: &str, dir: &Path) -> Result<Theme, String> {
        let mut section = String::new();
        let mut extends = None;
        let mut mode = None;
        let mut skin: Vec<(&str, &str)> = vec![];
        let mut syntax: Vec<(&str, &str)> = vec![];
        let mut warnings = vec![];
        for line in text.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            if let Some(name) = line.strip_prefix('[').and_then(|l| l.split(']').next()) {
                section = name.trim().to_string();
                if !["skin", "syntax"].contains(&section.as_str()) {
                    warnings.push(format!("unknown section [{section}]"));
                }
                continue;
            }
            let Some((key, rest)) = line.split_once('=') else {
                continue;
            };
            let (key, value) = (key.trim(), quoted(rest));
            match (section.as_str(), key) {
                ("", "extends") => extends = Some(value),
                ("", "mode") => mode = Some(value),
                ("", _) => warnings.push(format!("unknown key {key}")),
                ("skin", _) => skin.push((key, value)),
                ("syntax", _) => syntax.push((key, value)),
                _ => {}
            }
        }

        let mut files = vec![];
        let palette = match extends {
            Some(name) => {
                let path = dir.join(name);
                let palette = Palette::load(&path).map_err(|e| format!("extends: {e}"))?;
                files.push(if path.is_dir() {
                    path.join("colors.toml")
                } else {
                    path
                });
                Some(palette)
            }
            None => None,
        };
        let mode = match mode {
            Some("dark") => Mode::Dark,
            Some("light") => Mode::Light,
            Some(other) => return Err(format!("bad value for mode: {other:?}")),
            None => palette.as_ref().map_or(Mode::Dark, |p| p.mode),
        };
        let mut theme = Theme {
            mode,
            skin: palette
                .as_ref()
                .map_or_else(|| SkinSpec::builtin(mode), SkinSpec::from_palette),
            palette,
            syntax: [None; Token::ALL.len()],
            files,
            warnings,
        };

        let mut bad = vec![];
        for (key, value) in skin {
            let Some(slot) = theme.skin.slot(key) else {
                theme.warnings.push(format!("unknown key skin.{key}"));
                continue;
            };
            match Rgb::parse(value) {
                Some(rgb) => *slot = rgb,
                None => bad.push(format!("bad value for skin.{key}: {value:?}")),
            }
        }
        for (key, value) in syntax {
            let Some(token) = Token::ALL.into_iter().find(|t| t.name() == key) else {
                theme.warnings.push(format!("unknown key syntax.{key}"));
                continue;
            };
            match Rgb::parse(value) {
                Some(rgb) => theme.syntax[token as usize] = Some(rgb),
                None => bad.push(format!("bad value for syntax.{key}: {value:?}")),
            }
        }
        if bad.is_empty() {
            Ok(theme)
        } else {
            Err(bad.join("; "))
        }
    }
}

pub fn theme_file(dir: &Path) -> Option<PathBuf> {
    ["theme.toml", "colors.toml"]
        .into_iter()
        .map(|name| dir.join(name))
        .find(|file| file.is_file())
}

fn quoted(rest: &str) -> &str {
    let rest = rest.trim();
    match rest.split_once('"') {
        Some((_, after)) => after.split_once('"').map_or(after, |(inside, _)| inside),
        None => rest,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process;

    fn palette_text(background: &str) -> String {
        let mut text = String::from("mode = \"light\"\n");
        for key in [
            "accent",
            "selection",
            "muted",
            "dark_background",
            "darker_background",
            "lighter_background",
            "foreground",
            "dark_foreground",
            "light_foreground",
            "bright_foreground",
            "red",
            "yellow",
            "orange",
            "green",
            "cyan",
            "blue",
            "magenta",
            "brown",
            "bright_red",
            "bright_yellow",
            "bright_green",
            "bright_cyan",
            "bright_blue",
            "bright_magenta",
        ] {
            text.push_str(&format!("{key} = \"#102030\"\n"));
        }
        text.push_str(&format!("background = \"{background}\"\n"));
        text
    }

    #[test]
    fn overrides_apply_on_the_builtin_skin_without_extends() {
        let t = Theme::parse(
            "mode = \"light\"\n[skin]\naccent = \"#ff0000\" # red\n[syntax]\nstring = \"#00ff00\"\n",
            Path::new("."),
        )
        .unwrap();
        assert_eq!(t.mode, Mode::Light);
        assert!(t.palette.is_none());
        assert_eq!(t.skin.accent, Rgb(255, 0, 0));
        assert_eq!(t.skin.base, SkinSpec::builtin(Mode::Light).base);
        assert_eq!(t.token(Token::String), Rgb(0, 255, 0));
        assert_eq!(t.token(Token::Keyword), Rgb(255, 0, 0));
        assert!(t.warnings.is_empty());
    }

    #[test]
    fn unknown_keys_warn_and_bad_values_fail() {
        let t = Theme::parse(
            "flavour = \"x\"\n[skin]\nglow = \"#000000\"\n[syntax]\nregex = \"#000000\"\n[metrics]\nradius = 6\n",
            Path::new("."),
        )
        .unwrap();
        assert_eq!(
            t.warnings,
            [
                "unknown key flavour",
                "unknown section [metrics]",
                "unknown key skin.glow",
                "unknown key syntax.regex",
            ]
        );
        let err = Theme::parse(
            "[skin]\ntext = \"#12\"\n[syntax]\ncomment = \"grey\"\n",
            Path::new("."),
        )
        .unwrap_err();
        assert_eq!(
            err,
            "bad value for skin.text: \"#12\"; bad value for syntax.comment: \"grey\""
        );
        assert!(Theme::parse("mode = \"dim\"\n", Path::new(".")).is_err());
    }

    #[test]
    fn a_folder_prefers_theme_toml_and_watches_the_palette_it_extends() {
        let dir = std::env::temp_dir().join(format!("diffz-theme-{}", process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("colors.toml"), palette_text("#abcdef")).unwrap();
        let plain = Theme::load(&dir).unwrap();
        assert_eq!(plain.skin.base, Rgb(0xab, 0xcd, 0xef));
        assert_eq!(plain.files, [dir.join("colors.toml")]);

        fs::write(
            dir.join("theme.toml"),
            "extends = \"colors.toml\"\n[skin]\nadded = \"#010203\"\n",
        )
        .unwrap();
        let t = Theme::load(&dir).unwrap();
        assert_eq!(t.mode, Mode::Light);
        assert_eq!(t.skin.base, Rgb(0xab, 0xcd, 0xef));
        assert_eq!(t.skin.added, Rgb(1, 2, 3));
        assert!(t.palette.is_some());
        assert_eq!(t.files, [dir.join("theme.toml"), dir.join("colors.toml")]);

        fs::write(dir.join("theme.toml"), "extends = \"missing.toml\"\n").unwrap();
        assert!(Theme::load(&dir).unwrap_err().starts_with("extends: "));
        let _ = fs::remove_dir_all(&dir);
    }
}
