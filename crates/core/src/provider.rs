//! The GUI receives typed services instead of subprocesses or SQL handles.
use crate::{anchor::ViewportAnchor, domain::*, patch::FileChange, review::*};
use serde::{Deserialize, Serialize};
use serde_json::value::RawValue;
use std::{
    collections::BTreeMap,
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};
#[derive(Debug, Clone, Default)]
pub struct Cancellation(Arc<AtomicBool>);
impl Cancellation {
    pub fn cancel(&self) {
        self.0.store(true, Ordering::Relaxed)
    }
    pub fn cancelled(&self) -> bool {
        self.0.load(Ordering::Relaxed)
    }
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OpenRequest {
    Fixture(String),
    Patch(PathBuf),
    /// A review on a hosted provider, in any address form that provider's `open` accepts.
    Remote {
        provider: ProviderId,
        address: String,
    },
    LocalGit {
        root: PathBuf,
        base: String,
        head: String,
    },
    LocalIndex(PathBuf),
    LocalWorktree(PathBuf),
    Resume(SnapshotId),
}
/// Everything diffz knows about one review host that needs no I/O. Implementations live
/// beside the host's client in diffz-adapters; the UI and the review model reach them
/// through [`WorkbenchServices::provider`].
pub trait ReviewRules: Send + Sync {
    fn id(&self) -> ProviderId;
    /// The host's name, as in "Send this review unchanged to GitHub".
    fn name(&self) -> &str;
    /// Open panel text: the mode button, then the address field's label, placeholder and help.
    fn open_label(&self) -> &str;
    fn address_label(&self) -> &str;
    fn address_hint(&self) -> &str;
    fn address_help(&self) -> &str;
    /// The command-line flag that opts in to publishing on this host.
    fn write_flag(&self) -> &str;
    /// A limit of this host, shown beside the review preview.
    fn preview_note(&self) -> Option<&str> {
        None
    }
    /// An address `open` accepts for this target, so a review resumed from Recents can refresh.
    fn reopen_address(&self, target: &RemoteTarget) -> String;
    /// A browser link to one line of a file at a revision.
    fn line_url(&self, target: &RemoteTarget, path: &str, revision: &str, line: u32) -> String;
    /// Verdicts this host can record. The preview disables the others.
    fn supports(&self, _verdict: Verdict) -> bool {
        true
    }
    /// Refuse a review this host cannot represent, before anything is frozen.
    fn check(
        &self,
        _verdict: Verdict,
        _summary: &str,
        _drafts: &[Draft],
    ) -> Result<(), ReviewError> {
        Ok(())
    }
    /// Freeze where one comment attaches. It is stored with the prepared review and covered
    /// by its fingerprint.
    fn position(
        &self,
        _target: &RemoteTarget,
        _file: &FileChange,
        _draft: &Draft,
    ) -> Result<Option<Box<RawValue>>, ReviewError> {
        Ok(None)
    }
    /// The review exactly as it is sent. These bytes are part of the fingerprint, so serialize
    /// structs rather than maps, and never reorder the fields of an existing payload.
    fn payload(&self, review: &PreparedReview) -> Box<RawValue>;
    /// Whether this host's remote reviews carry diffz's fingerprint marker, which
    /// reconciliation then requires.
    fn marks_reviews(&self) -> bool {
        false
    }
}
impl dyn ReviewRules {
    pub fn reopen(&self, target: &RemoteTarget) -> OpenRequest {
        OpenRequest::Remote {
            provider: self.id(),
            address: self.reopen_address(target),
        }
    }
}
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SavedView {
    #[serde(default)]
    pub revision: u64,
    #[serde(default)]
    pub review_summary: String,
    #[serde(default)]
    pub review_verdict: Option<Verdict>,
    pub preferences: Preferences,
    pub anchors: BTreeMap<String, ViewportAnchor>,
    pub selected_file: Option<FileId>,
}
#[derive(Debug, Clone)]
pub struct Opened {
    pub snapshot: Snapshot,
    pub drafts: Vec<Draft>,
    pub view: SavedView,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RecentSession {
    pub id: SnapshotId,
    pub title: String,
}
#[derive(Debug, Clone, thiserror::Error)]
#[error("{message}")]
pub struct ServiceError {
    pub message: String,
}
impl From<String> for ServiceError {
    fn from(message: String) -> Self {
        Self { message }
    }
}
impl From<&str> for ServiceError {
    fn from(s: &str) -> Self {
        s.to_string().into()
    }
}
/// Call synchronous implementations from a bounded worker, never during GPUI update or render.
pub trait WorkbenchServices: Send + Sync {
    fn open(&self, request: OpenRequest, cancel: Cancellation) -> Result<Opened, ServiceError>;
    fn source_lines(
        &self,
        _target: &RemoteTarget,
        _path: &str,
        _revision: &str,
        _start: u32,
        _count: u32,
    ) -> Result<Vec<String>, ServiceError> {
        Err("Provider source context cannot be retrieved".into())
    }
    fn save_draft(&self, draft: Draft) -> Result<u64, ServiceError>;
    fn discard_draft(&self, id: DraftId, version: u64) -> Result<(), ServiceError>;
    fn save_view(&self, snapshot: &SnapshotId, view: SavedView) -> Result<(), ServiceError>;
    fn settings(&self) -> Result<Settings, ServiceError> {
        Ok(Settings::default())
    }
    fn save_settings(&self, _settings: Settings) -> Result<(), ServiceError> {
        Ok(())
    }
    fn recent(&self) -> Result<Vec<RecentSession>, ServiceError>;
    fn hide_recent(&self, _id: Option<SnapshotId>) -> Result<(), ServiceError> {
        Err("History management unavailable".into())
    }
    fn prepare(
        &self,
        snapshot: &SnapshotId,
        drafts: Vec<Draft>,
        verdict: Verdict,
        summary: String,
    ) -> Result<PreparedReview, ServiceError>;
    fn publish(&self, prepared: PreparedReview) -> Result<OutboxEntry, ServiceError>;
    fn outbox(&self) -> Result<Vec<OutboxEntry>, ServiceError>;
    fn reconcile(&self, operation: OperationId) -> Result<OutboxEntry, ServiceError>;
    fn export(
        &self,
        context: crate::export::ContextExport,
        destination: PathBuf,
    ) -> Result<(), ServiceError>;
    fn writes_enabled(&self) -> bool;
    fn writes_enabled_for(&self, _provider: &ProviderId) -> bool {
        false
    }
    /// Every registered review provider, in the order the Open panel lists them.
    fn providers(&self) -> Vec<Arc<dyn ReviewRules>> {
        vec![]
    }
    fn provider(&self, id: &ProviderId) -> Option<Arc<dyn ReviewRules>> {
        self.providers().into_iter().find(|p| p.id() == *id)
    }
    fn fresh_id(&self) -> String;
}
