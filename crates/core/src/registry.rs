use crate::{
    annotation::{self, Annotation, Annotator},
    domain::Snapshot,
    extension::{self, Extension, ExtensionLanguage, ExtensionThemes, Installed},
    palette,
    syntax::Span,
};
use std::{
    env,
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
};

const HIGHLIGHT_LIMIT: usize = 256 * 1024;

pub trait LanguageProvider: Send + Sync {
    fn claim(&self, path: &str) -> Option<(&str, u8)>;
    fn highlight(&self, path: &str, source: &str, cancel: &AtomicUsize) -> Option<Vec<Vec<Span>>>;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ThemeEntry {
    pub label: String,
    pub reference: String,
    pub path: PathBuf,
}

pub trait ThemeSource: Send + Sync {
    fn list(&self) -> Vec<ThemeEntry>;
    fn resolve(&self, reference: &str) -> Option<ThemeEntry>;
}

#[derive(Default, Clone)]
pub struct Registry {
    languages: Vec<Arc<dyn LanguageProvider>>,
    themes: Vec<Arc<dyn ThemeSource>>,
    extensions: Vec<(Extension, Vec<Arc<ExtensionLanguage>>)>,
    problems: Vec<String>,
    annotators: Vec<Arc<dyn Annotator>>,
    #[cfg(feature = "wasm")]
    components: Vec<Arc<crate::component::CodeAnnotator>>,
}

impl Registry {
    pub fn builtin() -> Self {
        let mut registry = Self::default();
        registry.add_language(Arc::new(crate::syntax::BuiltinGrammars));
        registry.add_annotator(Arc::new(annotation::DiffCheck));
        registry.add_theme_source(Arc::new(palette::OmarchyCurrent));
        registry.add_theme_source(Arc::new(palette::ThemeDirectories(palette::theme_dirs())));
        registry
    }

    pub fn installed() -> Self {
        let mut registry = Self::builtin();
        registry.add_extensions(extension::discover(&extension::extension_dirs()));
        registry
    }

    pub fn add_extensions(&mut self, installed: Installed) {
        self.problems.extend(installed.problems);
        let mut themes = vec![];
        for ext in installed.extensions {
            let languages: Vec<_> = ext
                .languages
                .iter()
                .map(|l| Arc::new(ExtensionLanguage::new(l)))
                .collect();
            for language in &languages {
                self.languages.push(language.clone());
            }
            themes.extend(ext.themes.iter().cloned());
            for (id, path) in &ext.annotators {
                #[cfg(feature = "wasm")]
                {
                    let annotator = Arc::new(crate::component::CodeAnnotator::new(
                        id.clone(),
                        path.clone(),
                        Default::default(),
                    ));
                    self.components.push(annotator.clone());
                    self.annotators.push(annotator);
                }
                #[cfg(not(feature = "wasm"))]
                self.problems.push(format!(
                    "{id}: this diffz was built without WebAssembly support ({})",
                    path.display()
                ));
            }
            self.extensions.push((ext, languages));
        }
        if !themes.is_empty() {
            self.add_theme_source(Arc::new(ExtensionThemes(themes)));
        }
    }

    pub fn extensions(&self) -> impl Iterator<Item = &Extension> {
        self.extensions.iter().map(|(ext, _)| ext)
    }

    pub fn problems(&self) -> &[String] {
        &self.problems
    }

    pub fn check_extensions(&self) -> Vec<String> {
        let mut out = vec![];
        for (ext, languages) in &self.extensions {
            for (language, provider) in ext.languages.iter().zip(languages) {
                if let Err(e) = provider.check() {
                    out.push(format!("{}: {}: {e}", ext.id, language.name));
                }
            }
        }
        #[cfg(feature = "wasm")]
        for component in &self.components {
            if let Err(e) = component.compile() {
                out.push(format!("{}: {e}", component.id()));
            }
        }
        out
    }

    pub fn add_annotator(&mut self, annotator: Arc<dyn Annotator>) {
        self.annotators.push(annotator);
    }

    pub fn annotate(
        &self,
        snapshot: &Snapshot,
        cancel: &crate::provider::Cancellation,
    ) -> (Vec<Annotation>, Vec<String>) {
        let mut found = vec![];
        let mut problems = vec![];
        for annotator in &self.annotators {
            if cancel.cancelled() {
                break;
            }
            match annotator.annotate(snapshot, cancel) {
                Ok(list) => {
                    let (total, before) = (list.len(), found.len());
                    found.extend(
                        list.into_iter()
                            .filter(|a| annotation::in_snapshot(snapshot, &a.anchor)),
                    );
                    let dropped = total - (found.len() - before);
                    if dropped > 0 {
                        problems.push(format!(
                            "{}: {dropped} annotations point outside this review",
                            annotator.id()
                        ));
                    }
                }
                Err(e) => problems.push(format!("{}: {e}", annotator.id())),
            }
        }
        (found, problems)
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

    pub fn language_name(&self, path: &str) -> Option<&str> {
        self.language(path).map(|(_, name)| name)
    }

    pub fn highlight(&self, path: &str, source: &str, cancel: &AtomicUsize) -> Vec<Vec<Span>> {
        let count = source.bytes().filter(|b| *b == b'\n').count() + 1;
        if source.len() > HIGHLIGHT_LIMIT || cancel.load(Ordering::Relaxed) != 0 {
            return vec![vec![]; count];
        }
        self.language(path)
            .and_then(|(provider, _)| provider.highlight(path, source, cancel))
            .unwrap_or_else(|| vec![vec![]; count])
    }

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
        crate::theme::theme_file(&path)?
    } else {
        path
    };
    if !file.is_file() {
        return None;
    }
    let reference_path = Path::new(reference);
    let dir = if reference_path
        .file_name()
        .is_some_and(|n| n == "colors.toml" || n == "theme.toml")
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

    struct Stray;
    impl Annotator for Stray {
        fn id(&self) -> &str {
            "stray"
        }
        fn annotate(
            &self,
            s: &Snapshot,
            _: &crate::provider::Cancellation,
        ) -> Result<Vec<Annotation>, String> {
            let at = |path: &str| Annotation {
                anchor: annotation::Anchor::File { path: path.into() },
                severity: annotation::Severity::Info,
                title: "t".into(),
                body: None,
                source: "stray".into(),
            };
            Ok(vec![at(&s.patch.files[0].display_path()), at("elsewhere")])
        }
    }

    #[test]
    fn annotations_outside_the_review_are_dropped_and_counted() {
        let patch = crate::patch::parse_patch(
            b"diff --git a/a b/a\n--- a/a\n+++ b/a\n@@ -1 +1 @@\n-x\n+y\n",
            Default::default(),
        )
        .unwrap();
        let s = Snapshot::new("t".into(), patch, None, vec![]);
        let mut r = Registry::default();
        r.add_annotator(Arc::new(Stray));
        let (found, problems) = r.annotate(&s, &Default::default());
        assert_eq!(found.len(), 1);
        assert_eq!(problems, ["stray: 1 annotations point outside this review"]);
    }
}
