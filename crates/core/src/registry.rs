//! Extension points. Built-in languages and themes register here through the same
//! traits that extensions will implement.
use crate::{palette, syntax::Span};
use std::{
    env,
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
};

/// Larger sources stay unhighlighted so one file cannot stall the highlight worker.
const HIGHLIGHT_LIMIT: usize = 256 * 1024;

pub trait LanguageProvider: Send + Sync {
    /// The language name for `path` and a priority. The highest priority wins; ties
    /// go to the provider registered first.
    fn claim(&self, path: &str) -> Option<(&str, u8)>;
    /// One span list per line of `source`. `None` leaves the source unhighlighted.
    fn highlight(&self, path: &str, source: &str, cancel: &AtomicUsize) -> Option<Vec<Vec<Span>>>;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ThemeEntry {
    pub label: String,
    /// What settings and `--theme` store to select this theme again.
    pub reference: String,
    /// The `colors.toml` file, watched for edits while the theme is active.
    pub path: PathBuf,
}

pub trait ThemeSource: Send + Sync {
    /// Themes for the picker, in display order.
    fn list(&self) -> Vec<ThemeEntry>;
    /// `Some` only when the theme's file exists.
    fn resolve(&self, reference: &str) -> Option<ThemeEntry>;
}

#[derive(Default, Clone)]
pub struct Registry {
    languages: Vec<Arc<dyn LanguageProvider>>,
    themes: Vec<Arc<dyn ThemeSource>>,
}

impl Registry {
    pub fn builtin() -> Self {
        let mut registry = Self::default();
        registry.add_language(Arc::new(crate::syntax::BuiltinGrammars));
        registry.add_theme_source(Arc::new(palette::OmarchyCurrent));
        registry.add_theme_source(Arc::new(palette::ThemeDirectories(palette::theme_dirs())));
        registry
    }

    pub fn add_language(&mut self, provider: Arc<dyn LanguageProvider>) {
        self.languages.push(provider);
    }

    pub fn add_theme_source(&mut self, source: Arc<dyn ThemeSource>) {
        self.themes.push(source);
    }

    fn language(&self, path: &str) -> Option<(&dyn LanguageProvider, &str)> {
        let mut best: Option<(&dyn LanguageProvider, &str, u8)> = None;
        for provider in &self.languages {
            if let Some((name, priority)) = provider.claim(path)
                && best.is_none_or(|(_, _, p)| priority > p)
            {
                best = Some((provider.as_ref(), name, priority));
            }
        }
        best.map(|(provider, name, _)| (provider, name))
    }

    /// The language name for `path`, or None when no provider claims it. Status text uses this result.
    pub fn language_name(&self, path: &str) -> Option<&str> {
        self.language(path).map(|(_, name)| name)
    }

    /// One span list per line of `source`, all empty when nothing highlights it.
    pub fn highlight(&self, path: &str, source: &str, cancel: &AtomicUsize) -> Vec<Vec<Span>> {
        let count = source.bytes().filter(|b| *b == b'\n').count() + 1;
        if source.len() > HIGHLIGHT_LIMIT || cancel.load(Ordering::Relaxed) != 0 {
            return vec![vec![]; count];
        }
        self.language(path)
            .and_then(|(provider, _)| provider.highlight(path, source, cancel))
            .unwrap_or_else(|| vec![vec![]; count])
    }

    /// Every source's themes in registration order. The first source to list a reference keeps it.
    pub fn themes(&self) -> Vec<ThemeEntry> {
        let mut out: Vec<ThemeEntry> = Vec::new();
        for source in &self.themes {
            for entry in source.list() {
                if !out.iter().any(|e| e.reference == entry.reference) {
                    out.push(entry);
                }
            }
        }
        out
    }

    /// A reference holding `'/'` or starting with `'~'` is a path to a theme folder or
    /// its `colors.toml`, with `~` → `$HOME`. Anything else goes to the sources in order.
    pub fn resolve_theme(&self, reference: &str) -> Option<ThemeEntry> {
        if reference.starts_with('~') || reference.contains('/') {
            return theme_at_path(reference);
        }
        self.themes.iter().find_map(|s| s.resolve(reference))
    }
}

fn theme_at_path(reference: &str) -> Option<ThemeEntry> {
    let path = match reference.strip_prefix('~') {
        Some(rest) => {
            let home = PathBuf::from(env::var_os("HOME")?);
            if rest.is_empty() {
                home
            } else {
                home.join(rest.trim_start_matches('/'))
            }
        }
        None => PathBuf::from(reference),
    };
    let file = if path.is_dir() {
        path.join("colors.toml")
    } else {
        path
    };
    if !file.is_file() {
        return None;
    }
    let reference_path = Path::new(reference);
    let dir = if reference_path
        .file_name()
        .is_some_and(|n| n == "colors.toml")
    {
        reference_path.parent().unwrap_or(reference_path)
    } else {
        reference_path
    };
    let label = dir
        .file_name()
        .and_then(|n| n.to_str())
        .map(String::from)
        .unwrap_or_else(|| reference.to_string());
    Some(ThemeEntry {
        label,
        reference: reference.to_string(),
        path: file,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Fixed(&'static str, u8);
    impl LanguageProvider for Fixed {
        fn claim(&self, path: &str) -> Option<(&str, u8)> {
            path.ends_with(".x").then_some((self.0, self.1))
        }
        fn highlight(&self, _: &str, source: &str, _: &AtomicUsize) -> Option<Vec<Vec<Span>>> {
            let token = crate::syntax::Token::Keyword;
            Some(
                source
                    .split('\n')
                    .map(|l| {
                        vec![Span {
                            bytes: 0..l.len(),
                            token,
                        }]
                    })
                    .collect(),
            )
        }
    }

    #[test]
    fn highest_priority_wins_and_ties_keep_the_first() {
        let mut r = Registry::default();
        r.add_language(Arc::new(Fixed("low", 0)));
        r.add_language(Arc::new(Fixed("high", 5)));
        r.add_language(Arc::new(Fixed("tie", 5)));
        assert_eq!(r.language_name("a.x"), Some("high"));
        assert_eq!(r.language_name("a.y"), None);
    }

    #[test]
    fn highlight_is_plain_when_unclaimed_oversized_or_cancelled() {
        let mut r = Registry::default();
        r.add_language(Arc::new(Fixed("x", 0)));
        let live = AtomicUsize::new(0);
        let plain = |lines: Vec<Vec<Span>>, count: usize| {
            lines.len() == count && lines.iter().all(Vec::is_empty)
        };
        assert_eq!(r.highlight("a.x", "ab\ncd", &live)[1].len(), 1);
        assert!(plain(r.highlight("a.y", "ab\ncd", &live), 2));
        assert!(plain(r.highlight("a.x", "ab", &AtomicUsize::new(1)), 1));
        let big = "a".repeat(HIGHLIGHT_LIMIT + 1);
        assert!(plain(r.highlight("a.x", &big, &live), 1));
    }

    struct Listed(Vec<(&'static str, &'static str)>);
    impl ThemeSource for Listed {
        fn list(&self) -> Vec<ThemeEntry> {
            self.0
                .iter()
                .map(|(label, reference)| ThemeEntry {
                    label: label.to_string(),
                    reference: reference.to_string(),
                    path: PathBuf::from(format!("/{label}")),
                })
                .collect()
        }
        fn resolve(&self, reference: &str) -> Option<ThemeEntry> {
            self.list().into_iter().find(|e| e.reference == reference)
        }
    }

    #[test]
    fn earlier_theme_sources_keep_a_shared_reference() {
        let mut r = Registry::default();
        r.add_theme_source(Arc::new(Listed(vec![("one", "a")])));
        r.add_theme_source(Arc::new(Listed(vec![("two", "a"), ("three", "b")])));
        let labels: Vec<_> = r.themes().into_iter().map(|e| e.label).collect();
        assert_eq!(labels, ["one", "three"]);
        assert_eq!(r.resolve_theme("a").unwrap().label, "one");
        assert_eq!(r.resolve_theme("b").unwrap().label, "three");
        assert_eq!(r.resolve_theme("c"), None);
    }
}
