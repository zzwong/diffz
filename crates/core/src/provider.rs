//! The GUI receives typed services instead of subprocesses or SQL handles.
use crate::{anchor::ViewportAnchor, domain::*, review::*};
use serde::{Deserialize, Serialize};
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
    GitHub(String),
    GitLab(String),
    LocalGit {
        root: PathBuf,
        base: String,
        head: String,
    },
    LocalIndex(PathBuf),
    LocalWorktree(PathBuf),
    Resume(SnapshotId),
}
impl RemoteTarget {
    /// The request that originally fetched this target, reconstructed purely from its
    /// identity, which lets a review reopened from Recents still be refreshed against
    /// the provider.
    pub fn open_request(&self) -> OpenRequest {
        let RepositoryKey {
            host, owner, name, ..
        } = &self.repository;
        match self.provider {
            ProviderKind::GitHub => {
                OpenRequest::GitHub(format!("https://{host}/{owner}/{name}/pull/{}", self.pr))
            }
            ProviderKind::GitLab => OpenRequest::GitLab(format!(
                "https://{host}/{owner}/{name}/-/merge_requests/{}",
                self.pr
            )),
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
    fn writes_enabled_for(&self, provider: ProviderKind) -> bool {
        provider == ProviderKind::GitHub && self.writes_enabled()
    }
    fn fresh_id(&self) -> String;
}
