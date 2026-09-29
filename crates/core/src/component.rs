use crate::{
    annotation::{Anchor, Annotation, Annotator, Severity},
    domain::{Side, Snapshot},
    patch::{ChangeKind, RowKind},
    provider::Cancellation,
};
use std::{
    path::PathBuf,
    sync::{Condvar, Mutex, MutexGuard, OnceLock},
    time::Duration,
};
use wasmtime::{
    Config, Engine, Store, StoreLimits, StoreLimitsBuilder,
    component::{Component, Linker},
};

wasmtime::component::bindgen!({ path: "wit", world: "extension" });

use diffz::extension::types as wit;

const TICK: Duration = Duration::from_millis(10);

/// Component calls in flight. The epoch thread ticks only while there is one,
/// so an idle process that has run an extension does not wake 100 times a second.
static CALLS: Calls = Calls::new();

struct Calls {
    running: Mutex<usize>,
    changed: Condvar,
}

impl Calls {
    const fn new() -> Self {
        Self {
            running: Mutex::new(0),
            changed: Condvar::new(),
        }
    }

    fn count(&self) -> MutexGuard<'_, usize> {
        // The count stays consistent across a panic, so a poisoned lock is fine.
        self.running.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn start(&self) -> Call<'_> {
        *self.count() += 1;
        self.changed.notify_all();
        Call(self)
    }

    /// Blocks while no call is running.
    fn wait(&self) {
        let running = self.count();
        drop(
            self.changed
                .wait_while(running, |n| *n == 0)
                .unwrap_or_else(|e| e.into_inner()),
        );
    }
}

/// Counts one call from creation until drop, including on early return or panic.
struct Call<'a>(&'a Calls);

impl Drop for Call<'_> {
    fn drop(&mut self) {
        *self.0.count() -= 1;
    }
}

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
                    // A deadline counts ticks from when it is set. The first tick
                    // after waking comes a full TICK later; one left over from an
                    // earlier call can come sooner, as any first tick could when
                    // this thread never waited.
                    loop {
                        CALLS.wait();
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
        let _call = CALLS.start();
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicBool, Ordering};

    #[test]
    fn the_ticker_waits_until_a_call_starts_and_calls_end_on_panic() {
        let calls = Calls::new();
        let woke = AtomicBool::new(false);
        std::thread::scope(|s| {
            let ticker = s.spawn(|| {
                calls.wait();
                woke.store(true, Ordering::SeqCst);
            });
            std::thread::sleep(3 * TICK);
            assert!(!woke.load(Ordering::SeqCst));
            let call = calls.start();
            let also = calls.start();
            assert_eq!(*calls.count(), 2);
            // Hold the calls until the waiter sees them.
            ticker.join().unwrap();
            drop((call, also));
        });
        assert!(woke.load(Ordering::SeqCst));
        assert_eq!(*calls.count(), 0);
        let panicked = std::panic::catch_unwind(|| {
            let _call = calls.start();
            panic!("component host panicked");
        });
        assert!(panicked.is_err());
        assert_eq!(*calls.count(), 0);
    }

    #[test]
    fn no_call_is_counted_after_an_annotation_ends() {
        let todo = CodeAnnotator::new(
            "todo/todo".into(),
            PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("tests/fixtures/extensions/todo/annotators/todo.wasm"),
            Limits {
                time: Duration::from_millis(50),
                ..Default::default()
            },
        );
        let patch = crate::patch::parse_patch(
            b"diff --git a/a.rs b/a.rs\n--- a/a.rs\n+++ b/a.rs\n@@ -1 +1 @@\n-x\n+// TODO: z\n",
            Default::default(),
        )
        .unwrap();
        let cancel = Cancellation::default();
        let run = |title: &str| {
            let snapshot = Snapshot::new(title.into(), patch.clone(), None, vec![]);
            let found = todo.annotate(&snapshot, &cancel);
            assert_eq!(*CALLS.count(), 0, "{title}");
            found
        };
        assert_eq!(run("t").unwrap().len(), 1);
        assert_eq!(run("diffz-test-spin").unwrap_err(), "stopped after 50 ms");
    }
}
