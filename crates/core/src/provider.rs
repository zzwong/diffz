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
pub trait ReviewRules: Send + Sync {
    fn id(&self) -> ProviderId;
    fn name(&self) -> &str;
    fn open_label(&self) -> &str;
    fn address_label(&self) -> &str;
    fn address_hint(&self) -> &str;
    fn address_help(&self) -> &str;
    fn write_flag(&self) -> &str;
    fn preview_note(&self) -> Option<&str> {
        None
    }
    fn reopen_address(&self, target: &RemoteTarget) -> String;
    fn line_url(&self, target: &RemoteTarget, path: &str, revision: &str, line: u32) -> String;
    fn supports(&self, _verdict: Verdict) -> bool {
        true
    }
    fn check(
        &self,
        _verdict: Verdict,
        _summary: &str,
        _drafts: &[Draft],
    ) -> Result<(), ReviewError> {
        Ok(())
    }
    fn position(
        &self,
        _target: &RemoteTarget,
        _file: &FileChange,
        _draft: &Draft,
    ) -> Result<Option<Box<RawValue>>, ReviewError> {
        Ok(None)
    }
    /// Fingerprinted: serialize structs, not maps, and never reorder existing fields.
    fn payload(&self, review: &PreparedReview) -> Box<RawValue>;
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
    fn providers(&self) -> Vec<Arc<dyn ReviewRules>> {
        vec![]
    }
    fn provider(&self, id: &ProviderId) -> Option<Arc<dyn ReviewRules>> {
        self.providers().into_iter().find(|p| p.id() == *id)
    }
    fn fresh_id(&self) -> String;
}
