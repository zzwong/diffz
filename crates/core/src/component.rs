use crate::{
    annotation::{Anchor, Annotation, Annotator, Severity},
    domain::{Side, Snapshot},
    patch::{ChangeKind, RowKind},
    provider::Cancellation,
};
use std::{path::PathBuf, sync::OnceLock, time::Duration};
use wasmtime::{
    Config, Engine, Store, StoreLimits, StoreLimitsBuilder,
    component::{Component, Linker},
};

wasmtime::component::bindgen!({ path: "wit", world: "extension" });

use diffz::extension::types as wit;

const TICK: Duration = Duration::from_millis(10);

#[derive(Debug, Clone, Copy)]
pub struct Limits {
    pub time: Duration,
    pub memory: usize,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            time: Duration::from_secs(2),
            memory: 64 << 20,
        }
    }
}

fn engine() -> Result<&'static Engine, String> {
    static ENGINE: OnceLock<Result<Engine, String>> = OnceLock::new();
    ENGINE
        .get_or_init(|| {
            let mut config = Config::new();
            config.wasm_component_model(true).epoch_interruption(true);
            let engine = Engine::new(&config).map_err(|e| e.to_string())?;
            let ticker = engine.clone();
            std::thread::Builder::new()
                .name("diffz-wasm-epoch".into())
                .spawn(move || {
                    loop {
                        std::thread::sleep(TICK);
                        ticker.increment_epoch();
                    }
                })
                .map_err(|e| e.to_string())?;
            Ok(engine)
        })
        .as_ref()
        .map_err(Clone::clone)
}

pub struct CodeAnnotator {
    id: String,
    path: PathBuf,
    limits: Limits,
    component: OnceLock<Result<Component, String>>,
}

impl CodeAnnotator {
    pub fn new(id: String, path: PathBuf, limits: Limits) -> Self {
        Self {
            id,
            path,
            limits,
            component: OnceLock::new(),
        }
    }

    pub fn compile(&self) -> Result<&Component, String> {
        self.component
            .get_or_init(|| {
                Component::from_file(engine()?, &self.path)
                    .map_err(|e| format!("{}: {e}", self.path.display()))
            })
            .as_ref()
            .map_err(Clone::clone)
    }
}

impl Annotator for CodeAnnotator {
    fn id(&self) -> &str {
        &self.id
    }

    fn annotate(
        &self,
        snapshot: &Snapshot,
        cancel: &Cancellation,
    ) -> Result<Vec<Annotation>, String> {
        let component = self.compile()?;
        if cancel.cancelled() {
            return Err("cancelled".into());
        }
        let limits = StoreLimitsBuilder::new()
            .memory_size(self.limits.memory)
            .instances(4)
            .build();
        let mut store = Store::new(engine()?, limits);
        store.limiter(|limits: &mut StoreLimits| limits);
        store.set_epoch_deadline((self.limits.time.as_millis() / TICK.as_millis()).max(1) as u64);
        let linker = Linker::new(engine()?);
        let instance = Extension::instantiate(&mut store, component, &linker)
            .map_err(|e| format!("cannot start: {e}"))?;
        let found = instance
            .diffz_extension_annotator()
            .call_annotate(&mut store, &review(snapshot))
            .map_err(|e| match e.downcast_ref::<wasmtime::Trap>() {
                Some(wasmtime::Trap::Interrupt) => {
                    format!("stopped after {} ms", self.limits.time.as_millis())
                }
                _ => format!("{e:#}"),
            })??;
        Ok(found.into_iter().map(|a| annotation(a, &self.id)).collect())
    }
}

fn review(snapshot: &Snapshot) -> wit::Review {
    wit::Review {
        title: snapshot.title.clone(),
        files: snapshot
            .patch
            .files
            .iter()
            .map(|f| wit::ChangedFile {
                path: f.display_path(),
                old_path: f
                    .old_path
                    .as_ref()
                    .and_then(|p| p.utf8().ok())
                    .map(str::to_owned),
                status: match f.kind {
                    ChangeKind::Added => wit::FileStatus::Added,
                    ChangeKind::Deleted => wit::FileStatus::Deleted,
                    ChangeKind::Modified => wit::FileStatus::Modified,
                    ChangeKind::Renamed => wit::FileStatus::Renamed,
                    ChangeKind::Copied => wit::FileStatus::Copied,
                },
                hunks: f
                    .hunks
                    .iter()
                    .map(|h| wit::Hunk {
                        old_start: h.old_start,
                        old_count: h.old_count,
                        new_start: h.new_start,
                        new_count: h.new_count,
                        rows: h
                            .rows
                            .iter()
                            .map(|r| wit::Row {
                                kind: match r.kind {
                                    RowKind::Context => wit::RowKind::Context,
                                    RowKind::Added => wit::RowKind::Added,
                                    RowKind::Removed => wit::RowKind::Removed,
                                },
                                old_line: r.old_line,
                                new_line: r.new_line,
                                text: r.text.to_string(),
                            })
                            .collect(),
                    })
                    .collect(),
            })
            .collect(),
    }
}

fn annotation(a: wit::Annotation, source: &str) -> Annotation {
    Annotation {
        anchor: match a.anchor {
            wit::Anchor::File(path) => Anchor::File { path },
            wit::Anchor::Lines(r) => Anchor::Lines {
                path: r.path,
                side: match r.side {
                    wit::Side::Old => Side::Left,
                    wit::Side::New => Side::Right,
                },
                start: r.start,
                end: r.end,
            },
        },
        severity: match a.severity {
            wit::Severity::Note => Severity::Note,
            wit::Severity::Info => Severity::Info,
            wit::Severity::Warning => Severity::Warning,
            wit::Severity::Error => Severity::Error,
        },
        title: a.title,
        body: a.body,
        source: source.into(),
    }
}
