//! Omarchy theme palettes. Parse `colors.toml` files, load palettes from a file
//! or a directory, and resolve or discover themes beneath the standard
//! configuration and state dirs.
use std::{
    collections::{HashMap, HashSet},
    env, fs,
    path::{Path, PathBuf},
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rgb(pub u8, pub u8, pub u8);

impl Rgb {
    /// Read a `#rrggbb` string without case sensitivity. Any other input yields `None`.
    pub fn parse(hex: &str) -> Option<Rgb> {
        let bytes = hex.as_bytes();
        if bytes.len() != 7 || bytes[0] != b'#' {
            return None;
        }
        let mut out = [0u8; 6];
        for (i, byte) in bytes[1..].iter().enumerate() {
            out[i] = match byte {
                b'0'..=b'9' => byte - b'0',
                b'a'..=b'f' => byte - b'a' + 10,
                b'A'..=b'F' => byte - b'A' + 10,
                _ => return None,
            };
        }
        Some(Rgb(
            out[0] * 16 + out[1],
            out[2] * 16 + out[3],
            out[4] * 16 + out[5],
        ))
    }

    /// Linearly interpolate toward `other`: `t = 0` gives `self`, `t = 1` gives
    /// `other`. Each channel rounds half up, away from zero.
    pub fn mix(self, other: Rgb, t: f32) -> Rgb {
        let lerp = |a: u8, b: u8| (a as f32 + (b as f32 - a as f32) * t).round() as u8;
        Rgb(
            lerp(self.0, other.0),
            lerp(self.1, other.1),
            lerp(self.2, other.2),
        )
    }

    /// Render lowercase `#rrggbb`.
    pub fn hex(self) -> String {
        format!("#{:02x}{:02x}{:02x}", self.0, self.1, self.2)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    Dark,
    Light,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Palette {
    pub mode: Mode,
    pub accent: Rgb,
    pub selection: Rgb,
    pub muted: Rgb,
    pub background: Rgb,
    pub dark_background: Rgb,
    pub darker_background: Rgb,
    pub lighter_background: Rgb,
    pub foreground: Rgb,
    pub dark_foreground: Rgb,
    pub light_foreground: Rgb,
    pub bright_foreground: Rgb,
    pub red: Rgb,
    pub yellow: Rgb,
    pub orange: Rgb,
    pub green: Rgb,
    pub cyan: Rgb,
    pub blue: Rgb,
    pub magenta: Rgb,
    pub brown: Rgb,
    pub bright_red: Rgb,
    pub bright_yellow: Rgb,
    pub bright_green: Rgb,
    pub bright_cyan: Rgb,
    pub bright_blue: Rgb,
    pub bright_magenta: Rgb,
}

/// Canonical color keys, listed in the spec's own order.
const KEYS: [&str; 25] = [
    "accent",
    "selection",
    "muted",
    "background",
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
];

/// Legacy aliases usable where the canonical key would go.
const ALIASES: [(&str, &str); 8] = [
    ("bg", "background"),
    ("fg", "foreground"),
    ("dark_bg", "dark_background"),
    ("darker_bg", "darker_background"),
    ("lighter_bg", "lighter_background"),
    ("dark_fg", "dark_foreground"),
    ("light_fg", "light_foreground"),
    ("bright_fg", "bright_foreground"),
];

/// Return the canonical form of a key, or `None` when the key is unknown.
fn canonical_key(key: &str) -> Option<&'static str> {
    if let Some(&(_, canonical)) = ALIASES.iter().find(|(alias, _)| *alias == key) {
        return Some(canonical);
    }
    KEYS.iter().copied().find(|candidate| *candidate == key)
}

impl Palette {
    /// The error message names every required key that is missing, plus every bad value,
    /// for example `missing: red, green; bad value for accent: "#12"`.
    pub fn parse(text: &str) -> Result<Palette, String> {
        let mut raw: HashMap<&str, &str> = HashMap::new();
        let mut mode_raw: Option<&str> = None;
        for line in text.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let Some(eq) = line.find('=') else {
                continue;
            };
            let key = line[..eq].trim();
            let rest = line[eq + 1..].trim();
            let value = match rest.find('"') {
                None => rest,
                Some(open) => {
                    let after = &rest[open + 1..];
                    match after.find('"') {
                        Some(close) => &after[..close],
                        None => after,
                    }
                }
            };
            if key == "mode" {
                mode_raw = Some(value);
                continue;
            }
            let Some(canonical) = canonical_key(key) else {
                continue; // unknown key
            };
            if canonical != key && raw.contains_key(canonical) {
                continue; // an alias never displaces the canonical key, whichever arrives first
            }
            raw.insert(canonical, value);
        }

        let mut missing: Vec<&str> = Vec::new();
        let mut bad: Vec<String> = Vec::new();
        let mut colors: HashMap<&str, Rgb> = HashMap::new();
        for key in KEYS {
            match raw.get(key) {
                None => missing.push(key),
                Some(value) => match Rgb::parse(value) {
                    Some(rgb) => {
                        colors.insert(key, rgb);
                    }
                    None => bad.push(format!("bad value for {key}: {value:?}")),
                },
            }
        }
        if !missing.is_empty() || !bad.is_empty() {
            let mut parts: Vec<String> = Vec::new();
            if !missing.is_empty() {
                parts.push(format!("missing: {}", missing.join(", ")));
            }
            parts.extend(bad);
            return Err(parts.join("; "));
        }

        let mode = if mode_raw == Some("light") {
            Mode::Light
        } else {
            Mode::Dark
        };
        Ok(Palette {
            mode,
            accent: colors["accent"],
            selection: colors["selection"],
            muted: colors["muted"],
            background: colors["background"],
            dark_background: colors["dark_background"],
            darker_background: colors["darker_background"],
            lighter_background: colors["lighter_background"],
            foreground: colors["foreground"],
            dark_foreground: colors["dark_foreground"],
            light_foreground: colors["light_foreground"],
            bright_foreground: colors["bright_foreground"],
            red: colors["red"],
            yellow: colors["yellow"],
            orange: colors["orange"],
            green: colors["green"],
            cyan: colors["cyan"],
            blue: colors["blue"],
            magenta: colors["magenta"],
            brown: colors["brown"],
            bright_red: colors["bright_red"],
            bright_yellow: colors["bright_yellow"],
            bright_green: colors["bright_green"],
            bright_cyan: colors["bright_cyan"],
            bright_blue: colors["bright_blue"],
            bright_magenta: colors["bright_magenta"],
        })
    }

    /// `path` names a `colors.toml` palette file, or a directory that holds one. Read
    /// errors turn into `Err(String)` that names the path.
    pub fn load(path: &Path) -> Result<Palette, String> {
        let target = if path.is_dir() {
            path.join("colors.toml")
        } else {
            path.to_path_buf()
        };
        let text = fs::read_to_string(&target)
            .map_err(|e| format!("cannot read {}: {e}", target.display()))?;
        Self::parse(&text)
    }
}

/// Dirs searched in the order below: `$XDG_CONFIG_HOME/diffz/themes` (or
/// `~/.config/diffz/themes`), `~/.config/omarchy/themes`, and
/// `/usr/share/omarchy/themes` is last. Home comes from `$HOME`. Missing dirs
/// still get listed.
pub fn theme_dirs() -> Vec<PathBuf> {
    let home = env::var_os("HOME").map(PathBuf::from);
    let mut dirs = Vec::new();
    if let Some(xdg) = env::var_os("XDG_CONFIG_HOME") {
        dirs.push(PathBuf::from(xdg).join("diffz").join("themes"));
    } else if let Some(home) = &home {
        dirs.push(home.join(".config").join("diffz").join("themes"));
    }
    if let Some(home) = &home {
        dirs.push(home.join(".config").join("omarchy").join("themes"));
    }
    dirs.push(PathBuf::from("/usr/share/omarchy/themes"));
    dirs
}

/// Turn a reference into a colors.toml file path, `Some` only when the file exists.
/// `"current"` and `"omarchy"` both → `~/.local/state/omarchy/current/theme/colors.toml`;
/// a reference holding `'/'` or starting with `'~'` → that path (a folder or a file,
/// with `~` → `$HOME`); if neither holds, the earliest `<dir>/<name>/colors.toml` within
/// `dirs` that exists.
pub fn resolve_in(reference: &str, dirs: &[PathBuf]) -> Option<PathBuf> {
    let home = env::var_os("HOME").map(PathBuf::from);
    if reference == "current" || reference == "omarchy" {
        let path = home?
            .join(".local")
            .join("state")
            .join("omarchy")
            .join("current")
            .join("theme")
            .join("colors.toml");
        return path.is_file().then_some(path);
    }
    if let Some(rest) = reference.strip_prefix('~') {
        let base = home?;
        let path = if rest.is_empty() {
            base
        } else {
            base.join(rest.trim_start_matches('/'))
        };
        return as_colors_file(path);
    }
    if reference.contains('/') {
        return as_colors_file(PathBuf::from(reference));
    }
    for dir in dirs {
        let candidate = dir.join(reference).join("colors.toml");
        if candidate.is_file() {
            return Some(candidate);
        }
    }
    None
}

/// If `path` names a directory, target `path/colors.toml`; else treat it as
/// the file. `Some` only when that file is present.
fn as_colors_file(path: PathBuf) -> Option<PathBuf> {
    let file = if path.is_dir() {
        path.join("colors.toml")
    } else {
        path
    };
    file.is_file().then_some(file)
}

/// `resolve_in(reference, &theme_dirs())`
pub fn resolve(reference: &str) -> Option<PathBuf> {
    resolve_in(reference, &theme_dirs())
}

/// Every `<dir>/<name>/colors.toml` beneath `dirs`; the first dir wins when
/// names repeat; sorted by name.
pub fn discover_in(dirs: &[PathBuf]) -> Vec<(String, PathBuf)> {
    let mut out: Vec<(String, PathBuf)> = Vec::new();
    let mut seen: HashSet<String> = HashSet::new();
    for dir in dirs {
        let Ok(entries) = fs::read_dir(dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            let colors = path.join("colors.toml");
            let Some(name) = entry.file_name().to_str().map(String::from) else {
                continue;
            };
            if !colors.is_file() || !seen.insert(name.clone()) {
                continue; // lacks colors.toml, or a directory seen earlier already claimed that name
            }
            out.push((name, colors));
        }
    }
    out.sort_by(|a, b| a.0.cmp(&b.0));
    out
}

/// `discover_in(&theme_dirs())`
pub fn discover() -> Vec<(String, PathBuf)> {
    discover_in(&theme_dirs())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process;

    fn fixture_path(name: &str) -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/themes")
            .join(name)
            .join("colors.toml")
    }

    fn fixture(name: &str) -> String {
        fs::read_to_string(fixture_path(name)).unwrap()
    }

    /// Drop any line whose key appears in `keys`.
    fn without_keys(text: &str, keys: &[&str]) -> String {
        text.lines()
            .filter(|line| {
                let key = line.trim().split('=').next().unwrap_or("").trim();
                !keys.contains(&key)
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    fn assert_fixture(name: &str, mode: Mode, accent: Rgb, background: Rgb, green: Rgb) {
        let pal = Palette::load(&fixture_path(name)).unwrap();
        assert_eq!(pal.mode, mode, "{name} mode");
        assert_eq!(pal.accent, accent, "{name} accent");
        assert_eq!(pal.background, background, "{name} background");
        assert_eq!(pal.green, green, "{name} green");
    }

    #[test]
    fn rgb_parse_and_hex() {
        assert_eq!(Rgb::parse("#AbCdEf"), Some(Rgb(0xab, 0xcd, 0xef)));
        assert_eq!(Rgb::parse("#ABCDEF"), Some(Rgb(0xab, 0xcd, 0xef)));
        assert_eq!(Rgb::parse("123456"), None);
        assert_eq!(Rgb::parse("#12345"), None);
        assert_eq!(Rgb::parse("#gggggg"), None);
        assert_eq!(Rgb::parse("#1234567"), None);
        assert_eq!(Rgb::parse(""), None);
        assert_eq!(Rgb(0xab, 0xcd, 0xef).hex(), "#abcdef");
    }

    #[test]
    fn rgb_mix() {
        let black = Rgb(0, 0, 0);
        let white = Rgb(255, 255, 255);
        assert_eq!(black.mix(white, 0.0), black);
        assert_eq!(black.mix(white, 1.0), white);
        // 127.5 rounds up to 128, away from zero
        assert_eq!(black.mix(white, 0.5), Rgb(128, 128, 128));
        assert_eq!(Rgb(10, 20, 30).mix(Rgb(20, 40, 60), 0.5), Rgb(15, 30, 45));
    }

    #[test]
    fn fixtures_parse() {
        assert_fixture(
            "tokyo-night",
            Mode::Dark,
            Rgb(0x7a, 0xa2, 0xf7),
            Rgb(0x1a, 0x1b, 0x26),
            Rgb(0x9e, 0xce, 0x6a),
        );
        // Load via the file path, not the directory.
        let latte = Palette::load(&fixture_path("catppuccin-latte")).unwrap();
        assert_eq!(latte.mode, Mode::Light);
        assert_eq!(latte.accent, Rgb(0x1e, 0x66, 0xf5));
        assert_eq!(latte.background, Rgb(0xef, 0xf1, 0xf5));
        assert_eq!(latte.green, Rgb(0x40, 0xa0, 0x2b));
        // Parse directly from the text.
        let gruvbox = Palette::parse(&fixture("gruvbox")).unwrap();
        assert_eq!(gruvbox.mode, Mode::Dark);
        assert_eq!(gruvbox.accent, Rgb(0x7d, 0xae, 0xa3));
        assert_eq!(gruvbox.background, Rgb(0x28, 0x28, 0x28));
        assert_eq!(gruvbox.green, Rgb(0xa9, 0xb6, 0x65));
    }

    #[test]
    fn legacy_aliases_fill_and_canonical_wins() {
        let tokyo = fixture("tokyo-night");
        // Remove the canonical lines for `background` and `foreground`, then aliases fill them.
        let no_canonical = without_keys(&tokyo, &["background", "foreground"]);
        let aliased = format!("{no_canonical}\nbg = \"#0a0a0a\"\nfg = \"#eeeeee\"\n");
        let pal = Palette::parse(&aliased).unwrap();
        assert_eq!(pal.background, Rgb(0x0a, 0x0a, 0x0a));
        assert_eq!(pal.foreground, Rgb(0xee, 0xee, 0xee));

        // Alias comes first, canonical later: the canonical key wins.
        let alias_first = format!("bg = \"#0a0a0a\"\n{tokyo}\n");
        assert_eq!(
            Palette::parse(&alias_first).unwrap().background,
            Rgb(0x1a, 0x1b, 0x26)
        );

        // Canonical comes first, alias later: the canonical key still wins.
        let alias_last = format!("{tokyo}\nbg = \"#0a0a0a\"\n");
        assert_eq!(
            Palette::parse(&alias_last).unwrap().background,
            Rgb(0x1a, 0x1b, 0x26)
        );
    }

    #[test]
    fn missing_keys_are_reported() {
        let tokyo = fixture("tokyo-night");
        let err = Palette::parse(&without_keys(&tokyo, &["red"])).unwrap_err();
        assert!(err.contains("red"), "error was: {err}");

        let err = Palette::parse(&without_keys(&tokyo, &["red", "green"])).unwrap_err();
        assert!(err.contains("red"), "error was: {err}");
        assert!(err.contains("green"), "error was: {err}");
    }

    #[test]
    fn missing_and_bad_are_all_listed() {
        let tokyo = fixture("tokyo-night");
        let text = without_keys(&tokyo, &["red", "yellow", "orange", "green"])
            .replace("accent = \"#7aa2f7\"", "accent = \"#12\"");
        assert_eq!(
            Palette::parse(&text).unwrap_err(),
            r##"missing: red, yellow, orange, green; bad value for accent: "#12""##
        );
    }

    #[test]
    fn comments_blanks_unknown_and_whitespace_ignored() {
        let tokyo = fixture("tokyo-night");
        let junk = format!(
            "{tokyo}\n\n  # a comment line\nunknown_key = \"#010203\"\ntheme = \"nord\"\n  green  =  \"#9ece6a\"   # trailing comment\n"
        );
        let pal = Palette::parse(&junk).unwrap();
        assert_eq!(pal, Palette::parse(&tokyo).unwrap());

        // A changed value with a trailing comment still applies, and the
        // trailing comment does not break parsing.
        let no_selection = without_keys(&tokyo, &["selection"]);
        let commented = format!("{no_selection}\nselection = \"#ffffff\"  # trailing comment\n");
        assert_eq!(
            Palette::parse(&commented).unwrap().selection,
            Rgb(0xff, 0xff, 0xff)
        );
    }

    #[test]
    fn resolve_and_discover_under_temp_tree() {
        let root = env::temp_dir().join(format!("diffz-palette-{}", process::id()));
        let _ = fs::remove_dir_all(&root);
        let first = root.join("first");
        let second = root.join("second");

        fn theme_dir(dir: &Path, name: &str) {
            let dir = dir.join(name);
            fs::create_dir_all(&dir).unwrap();
            fs::write(dir.join("colors.toml"), "mode = \"dark\"\n").unwrap();
        }
        theme_dir(&first, "dup");
        theme_dir(&first, "only-first");
        theme_dir(&second, "dup");
        theme_dir(&second, "only-second");
        // A dir holding no colors.toml is never discovered.
        fs::create_dir_all(first.join("no-theme")).unwrap();
        // A nested dir pointed at by path.
        let nested = root.join("refs").join("nested");
        theme_dir(&root.join("refs"), "nested");

        let dirs = [first.clone(), second.clone()];

        // Duplicate name: the first dir keeps it; names sorted.
        assert_eq!(
            discover_in(&dirs),
            vec![
                ("dup".to_string(), first.join("dup/colors.toml")),
                (
                    "only-first".to_string(),
                    first.join("only-first/colors.toml")
                ),
                (
                    "only-second".to_string(),
                    second.join("only-second/colors.toml")
                ),
            ]
        );

        // Name lookup: the first match that exists.
        assert_eq!(
            resolve_in("dup", &dirs),
            Some(first.join("dup/colors.toml"))
        );
        assert_eq!(
            resolve_in("only-second", &dirs),
            Some(second.join("only-second/colors.toml"))
        );
        // A missing name gives None.
        assert_eq!(resolve_in("missing-name", &dirs), None);

        // Path references holding '/': a dir or the file colors.toml itself.
        let nested_dir = root.join("refs").join("nested");
        assert_eq!(
            resolve_in(nested_dir.to_str().unwrap(), &dirs),
            Some(nested.join("colors.toml"))
        );
        assert_eq!(
            resolve_in(nested.join("colors.toml").to_str().unwrap(), &dirs),
            Some(nested.join("colors.toml"))
        );
        // A path reference that does not exist gives None.
        assert_eq!(
            resolve_in(root.join("refs").join("gone").to_str().unwrap(), &dirs),
            None
        );

        let _ = fs::remove_dir_all(&root);
    }
}
