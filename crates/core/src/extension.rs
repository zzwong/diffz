use crate::{
    registry::{LanguageProvider, ThemeEntry, ThemeSource},
    syntax::Span,
    theme::theme_file,
};
use serde::Deserialize;
use std::{
    env, fs,
    path::{Component, Path, PathBuf},
    sync::atomic::AtomicUsize,
};

pub const CONTRACT: (u64, u64) = (0, 1);
const PRIORITY: u8 = 10;

#[derive(Deserialize)]
struct Manifest {
    id: String,
    name: Option<String>,
    version: Option<String>,
    diffz: String,
    #[serde(default)]
    languages: Vec<LanguageManifest>,
    #[serde(default)]
    themes: Vec<ThemeManifest>,
}

#[derive(Deserialize)]
struct LanguageManifest {
    name: String,
    grammar: PathBuf,
    queries: PathBuf,
    extensions: Vec<String>,
}

#[derive(Deserialize)]
struct ThemeManifest {
    path: PathBuf,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Extension {
    pub id: String,
    pub name: String,
    pub version: String,
    pub dir: PathBuf,
    pub languages: Vec<Language>,
    pub themes: Vec<PathBuf>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Language {
    pub name: String,
    pub grammar: PathBuf,
    pub queries: PathBuf,
    pub extensions: Vec<String>,
}

#[derive(Debug, Default)]
pub struct Installed {
    pub extensions: Vec<Extension>,
    pub problems: Vec<String>,
}

pub fn extension_dirs() -> Vec<PathBuf> {
    match env::var_os("XDG_CONFIG_HOME") {
        Some(xdg) => vec![PathBuf::from(xdg).join("diffz/extensions")],
        None => env::var_os("HOME")
            .map(|home| PathBuf::from(home).join(".config/diffz/extensions"))
            .into_iter()
            .collect(),
    }
}

pub fn discover(dirs: &[PathBuf]) -> Installed {
    let mut installed = Installed::default();
    for dir in dirs {
        let Ok(entries) = fs::read_dir(dir) else {
            continue;
        };
        let mut folders: Vec<_> = entries.flatten().map(|e| e.path()).collect();
        folders.sort();
        for folder in folders {
            if !folder.join("extension.toml").is_file() {
                continue;
            }
            match load(&folder) {
                Ok(ext) if installed.extensions.iter().any(|e| e.id == ext.id) => installed
                    .problems
                    .push(format!("{}: duplicate id {}", folder.display(), ext.id)),
                Ok(ext) => installed.extensions.push(ext),
                Err(e) => installed
                    .problems
                    .push(format!("{}: {e}", folder.display())),
            }
        }
    }
    installed
}

pub fn load(dir: &Path) -> Result<Extension, String> {
    let text = fs::read_to_string(dir.join("extension.toml")).map_err(|e| e.to_string())?;
    let m: Manifest =
        toml::from_str(&text).map_err(|e| format!("extension.toml: {}", e.message()))?;
    if m.id.is_empty()
        || !m
            .id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"-_".contains(&b))
    {
        return Err(format!("invalid id {:?}", m.id));
    }
    let (major, minor) = m
        .diffz
        .split_once('.')
        .and_then(|(a, b)| Some((a.parse::<u64>().ok()?, b.parse::<u64>().ok()?)))
        .ok_or_else(|| format!("invalid diffz version {:?}", m.diffz))?;
    if major != CONTRACT.0 || minor > CONTRACT.1 {
        return Err(format!(
            "needs extension contract {}, this diffz provides {}.{}",
            m.diffz, CONTRACT.0, CONTRACT.1
        ));
    }
    let inside = |p: &Path| -> Result<PathBuf, String> {
        if p.components().all(|c| matches!(c, Component::Normal(_))) {
            Ok(dir.join(p))
        } else {
            Err(format!("path {} leaves the extension", p.display()))
        }
    };
    let languages = m
        .languages
        .into_iter()
        .map(|l| {
            Ok(Language {
                grammar: inside(&l.grammar)?,
                queries: inside(&l.queries)?,
                extensions: l
                    .extensions
                    .iter()
                    .map(|e| e.to_ascii_lowercase())
                    .collect(),
                name: l.name,
            })
        })
        .collect::<Result<_, String>>()?;
    let themes = m
        .themes
        .iter()
        .map(|t| inside(&t.path))
        .collect::<Result<_, String>>()?;
    Ok(Extension {
        name: m.name.unwrap_or_else(|| m.id.clone()),
        version: m.version.unwrap_or_default(),
        id: m.id,
        dir: dir.to_path_buf(),
        languages,
        themes,
    })
}

pub struct ExtensionLanguage {
    name: String,
    extensions: Vec<String>,
    #[cfg(feature = "wasm")]
    grammar: crate::syntax::WasmGrammar,
}

impl ExtensionLanguage {
    pub fn new(language: &Language) -> Self {
        Self {
            name: language.name.clone(),
            extensions: language.extensions.clone(),
            #[cfg(feature = "wasm")]
            grammar: crate::syntax::WasmGrammar::new(
                language.name.clone(),
                language.grammar.clone(),
                language.queries.clone(),
            ),
        }
    }

    pub fn check(&self) -> Result<(), String> {
        #[cfg(feature = "wasm")]
        {
            self.grammar.compile().map(|_| ())
        }
        #[cfg(not(feature = "wasm"))]
        {
            Err("this diffz was built without WebAssembly grammar support".into())
        }
    }
}

impl LanguageProvider for ExtensionLanguage {
    fn claim(&self, path: &str) -> Option<(&str, u8)> {
        let ext = path
            .rsplit('/')
            .next()?
            .rsplit_once('.')?
            .1
            .to_ascii_lowercase();
        self.extensions
            .contains(&ext)
            .then_some((self.name.as_str(), PRIORITY))
    }
    fn highlight(&self, _path: &str, source: &str, cancel: &AtomicUsize) -> Option<Vec<Vec<Span>>> {
        #[cfg(feature = "wasm")]
        {
            self.grammar.highlight(source, cancel)
        }
        #[cfg(not(feature = "wasm"))]
        {
            let _ = (source, cancel);
            None
        }
    }
}

pub struct ExtensionThemes(pub Vec<PathBuf>);

impl ThemeSource for ExtensionThemes {
    fn list(&self) -> Vec<ThemeEntry> {
        let mut out: Vec<ThemeEntry> = self
            .0
            .iter()
            .filter_map(|dir| {
                let name = dir.file_name()?.to_str()?.to_string();
                Some(ThemeEntry {
                    label: name.clone(),
                    reference: name,
                    path: theme_file(dir)?,
                })
            })
            .collect();
        out.sort_by(|a, b| a.label.cmp(&b.label));
        out
    }
    fn resolve(&self, reference: &str) -> Option<ThemeEntry> {
        self.list().into_iter().find(|e| e.reference == reference)
    }
}
