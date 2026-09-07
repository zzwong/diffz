//! Application service wiring. Provider access, storage, and publication meet here.
use crate::{
    Result, fixtures,
    github::{GithubReader, GithubWriter, PrAddress},
    local_git::{LocalGit, LocalMode},
    outbox::Outbox,
    process::{read_bounded, resolve_program},
    store::Store,
};
use diffz_core::{
    domain::*,
    patch::{ParseLimits, parse_patch},
    provider::*,
    review::*,
};
use std::{
    path::{Path, PathBuf},
    sync::{
        Arc, Condvar, Mutex,
        atomic::{AtomicU64, Ordering},
    },
    time::{SystemTime, UNIX_EPOCH},
};

pub struct Services {
    store: Arc<Store>,
    github: Option<Arc<GithubReader>>,
    gitlab: Option<Arc<crate::gitlab::GitlabReader>>,
    git: Option<LocalGit>,
    outbox: Option<Outbox>,
    gitlab_outbox: Option<Outbox>,
    ids: AtomicU64,
    nonce: String,
    reads: (Mutex<usize>, Condvar),
}
struct Permit<'a>(&'a (Mutex<usize>, Condvar));
impl Drop for Permit<'_> {
    fn drop(&mut self) {
        if let Ok(mut n) = self.0.0.lock() {
            *n -= 1;
            self.0.1.notify_one();
        }
    }
}
impl Services {
    pub fn new(state: &Path, writes: bool) -> Result<Self> {
        Self::new_with_providers(state, writes, false)
    }
    pub fn new_with_providers(state: &Path, writes: bool, gitlab_writes: bool) -> Result<Self> {
        let store = Arc::new(Store::open(state)?);
        let github = resolve_program("gh")
            .ok()
            .map(|p| Arc::new(GithubReader::new(p)));
        let gitlab = resolve_program("glab")
            .ok()
            .map(|p| Arc::new(crate::gitlab::GitlabReader::new(p)));
        let gitlab_outbox = if gitlab_writes {
            gitlab.as_ref().map(|r| {
                Outbox::new(
                    store.clone(),
                    Arc::new(crate::gitlab::GitlabWriter::new(r.clone())),
                )
            })
        } else {
            None
        };
        let git = resolve_program("git").ok().map(LocalGit::new);
        let outbox = if writes {
            github
                .as_ref()
                .map(|g| Outbox::new(store.clone(), Arc::new(GithubWriter::new(g.clone()))))
        } else {
            None
        };
        let nonce = format!(
            "{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map_err(|_| "clock is before Unix epoch")?
                .as_nanos()
        );
        Ok(Self {
            store,
            github,
            gitlab,
            gitlab_outbox,
            git,
            outbox,
            ids: AtomicU64::new(1),
            nonce,
            reads: (Mutex::new(0), Condvar::new()),
        })
    }
    fn permit(&self, cancel: &Cancellation) -> Result<Permit<'_>> {
        let mut count = self.reads.0.lock().map_err(|_| "read gate poisoned")?;
        while *count >= 4 {
            if cancel.cancelled() {
                return Err("read cancelled before execution".into());
            }
            count = self
                .reads
                .1
                .wait_timeout(count, std::time::Duration::from_millis(50))
                .map_err(|_| "read gate poisoned")?
                .0;
        }
        *count += 1;
        Ok(Permit(&self.reads))
    }
    fn load(&self, request: OpenRequest, cancel: Cancellation) -> Result<Opened> {
        let _permit = self.permit(&cancel)?;
        if cancel.cancelled() {
            return Err("read cancelled".into());
        }
        let snapshot = match request {
            OpenRequest::Resume(id) => return self.store.opened(&id),
            OpenRequest::Fixture(id) => {
                let mut snapshot = Snapshot::with_origin(
                    format!("Fixture {id} · offline"),
                    parse_patch(
                        fixtures::patch(&id).ok_or("unknown/non-patch fixture")?,
                        ParseLimits::default(),
                    )?,
                    None,
                    vec![],
                    format!("fixture:{id}"),
                );
                fixtures::decorate(&mut snapshot, &id);
                snapshot
            }
            OpenRequest::Patch(path) => {
                let path = path.canonicalize()?;
                Snapshot::with_origin(
                    path.display().to_string(),
                    parse_patch(
                        &read_bounded(&path, 32 * 1024 * 1024)?,
                        ParseLimits::default(),
                    )?,
                    None,
                    vec![],
                    format!("patch:{path:?}"),
                )
            }
            OpenRequest::GitLab(value) => self
                .gitlab
                .as_ref()
                .ok_or("Install glab, then authenticate for this GitLab host")?
                .snapshot(&crate::gitlab::MrAddress::parse(&value)?, cancel)?,
            OpenRequest::GitHub(value) => self
                .github
                .as_ref()
                .ok_or("Install gh, then run gh auth login for this host")?
                .snapshot(&PrAddress::parse(&value)?, cancel)?,
            OpenRequest::LocalGit { root, base, head } => self
                .git
                .as_ref()
                .ok_or("Git was not found")?
                .snapshot(&root, LocalMode::Compare { base, head }, cancel)?,
            OpenRequest::LocalIndex(root) => self
                .git
                .as_ref()
                .ok_or("Git was not found")?
                .snapshot(&root, LocalMode::Staged, cancel)?,
            OpenRequest::LocalWorktree(root) => self
                .git
                .as_ref()
                .ok_or("Git was not found")?
                .snapshot(&root, LocalMode::WorkingTree, cancel)?,
        };
        self.store.put_snapshot(&snapshot)?;
        self.store.show_recent(&snapshot.id)?;
        Ok(Opened {
            drafts: self.store.drafts(&snapshot.id)?,
            view: self.store.view(&snapshot.id)?,
            snapshot,
        })
    }
}
impl WorkbenchServices for Services {
    fn source_lines(
        &self,
        target: &RemoteTarget,
        path: &str,
        revision: &str,
        start: u32,
        count: u32,
    ) -> std::result::Result<Vec<String>, ServiceError> {
        if start == 0
            || count > 1000
            || (revision != target.head && revision != target.comparison_base)
        {
            return Err("Invalid context request".into());
        }
        RepoPath::new(path.as_bytes().to_vec()).map_err(ServiceError::from)?;
        let cancel = Cancellation::default();
        let _permit = self
            .permit(&cancel)
            .map_err(|e| ServiceError::from(e.to_string()))?;
        let bytes = match target.provider {
            ProviderKind::GitHub => self
                .github
                .as_ref()
                .ok_or_else(|| ServiceError::from("gh unavailable"))?
                .source(target, path, revision),
            ProviderKind::GitLab => self
                .gitlab
                .as_ref()
                .ok_or_else(|| ServiceError::from("glab unavailable"))?
                .source(target, path, revision),
        }
        .map_err(|e| ServiceError::from(e.to_string()))?;
        if bytes.len() > 2 * 1024 * 1024 {
            return Err("Source context is over the 2 MiB cap".into());
        }
        let text = std::str::from_utf8(&bytes)
            .map_err(|_| ServiceError::from("Source is not UTF-8 text"))?;
        if text.contains('\0') {
            return Err("Binary input does not provide source lines".into());
        }
        Ok(text
            .lines()
            .skip((start - 1) as usize)
            .take(count as usize)
            .map(str::to_owned)
            .collect())
    }

    fn open(&self, r: OpenRequest, c: Cancellation) -> std::result::Result<Opened, ServiceError> {
        self.load(r, c).map_err(Into::into)
    }
    fn save_draft(&self, d: Draft) -> std::result::Result<u64, ServiceError> {
        self.store.save_draft(d).map_err(Into::into)
    }
    fn discard_draft(&self, id: DraftId, version: u64) -> std::result::Result<(), ServiceError> {
        self.store.discard_draft(&id, version).map_err(Into::into)
    }
    fn save_view(&self, id: &SnapshotId, v: SavedView) -> std::result::Result<(), ServiceError> {
        self.store.save_view(id, &v).map_err(Into::into)
    }
    fn settings(&self) -> std::result::Result<Settings, ServiceError> {
        self.store.settings().map_err(Into::into)
    }
    fn save_settings(&self, settings: Settings) -> std::result::Result<(), ServiceError> {
        self.store.save_settings(&settings).map_err(Into::into)
    }
    fn recent(&self) -> std::result::Result<Vec<RecentSession>, ServiceError> {
        self.store.recent().map_err(Into::into)
    }
    fn hide_recent(&self, id: Option<SnapshotId>) -> std::result::Result<(), ServiceError> {
        self.store.hide_recent(id.as_ref()).map_err(Into::into)
    }
    fn prepare(
        &self,
        id: &SnapshotId,
        drafts: Vec<Draft>,
        verdict: Verdict,
        summary: String,
    ) -> std::result::Result<PreparedReview, ServiceError> {
        let p = PreparedReview::prepare(
            OperationId(self.fresh_id()),
            &self.store.snapshot(id).map_err(ServiceError::from)?,
            drafts,
            verdict,
            summary,
        )
        .map_err(|e| ServiceError::from(e.to_string()))?;
        self.store.insert_prepared(&p).map_err(ServiceError::from)?;
        Ok(p)
    }
    fn publish(&self, p: PreparedReview) -> std::result::Result<OutboxEntry, ServiceError> {
        if p.target.provider == ProviderKind::GitLab {
            return self
                .gitlab_outbox
                .as_ref()
                .ok_or_else(|| {
                    ServiceError::from(
                        "GitLab publication is disabled; restart with --allow-gitlab-writes",
                    )
                })?
                .publish(p)
                .map_err(Into::into);
        }
        self.outbox.as_ref().ok_or_else(||ServiceError::from("GitHub publication is disabled; restart with --allow-github-writes to confirm a review"))?.publish(p).map_err(Into::into)
    }
    fn outbox(&self) -> std::result::Result<Vec<OutboxEntry>, ServiceError> {
        self.store.outbox().map_err(Into::into)
    }
    fn reconcile(&self, id: OperationId) -> std::result::Result<OutboxEntry, ServiceError> {
        let entry = self.store.operation(&id).map_err(ServiceError::from)?;
        if entry.prepared.target.provider == ProviderKind::GitLab {
            let reader = self
                .gitlab
                .as_ref()
                .ok_or_else(|| ServiceError::from("glab is required for reconciliation"))?;
            return Outbox::new(
                self.store.clone(),
                Arc::new(crate::gitlab::GitlabWriter::new(reader.clone())),
            )
            .reconcile(&id)
            .map_err(Into::into);
        }
        // Reconciliation only reads, so it remains available when publication is disabled.
        let reader = self
            .github
            .as_ref()
            .ok_or_else(|| ServiceError::from("gh is needed for reconciliation reads"))?;
        Outbox::new(
            self.store.clone(),
            Arc::new(GithubWriter::new(reader.clone())),
        )
        .reconcile(&id)
        .map_err(Into::into)
    }
    fn export(
        &self,
        c: diffz_core::export::ContextExport,
        p: PathBuf,
    ) -> std::result::Result<(), ServiceError> {
        crate::export::write_private_json(&p, &c).map_err(Into::into)
    }
    fn writes_enabled(&self) -> bool {
        self.outbox.is_some() || self.gitlab_outbox.is_some()
    }
    fn writes_enabled_for(&self, provider: ProviderKind) -> bool {
        match provider {
            ProviderKind::GitHub => self.outbox.is_some(),
            ProviderKind::GitLab => self.gitlab_outbox.is_some(),
        }
    }
    fn fresh_id(&self) -> String {
        format!(
            "{}-{}",
            self.nonce,
            self.ids.fetch_add(1, Ordering::Relaxed)
        )
    }
}
pub fn default_state_dir() -> Result<PathBuf> {
    #[cfg(target_os = "macos")]
    if let Some(home) = std::env::var_os("HOME") {
        let support = PathBuf::from(home).join("Library/Application Support");
        return Ok(migrated_state_dir(
            support.join("diffz"),
            migrated_state_dir(support.join("diffr"), support.join("ReviewWorkbench")),
        ));
    }
    if let Some(p) = std::env::var_os("XDG_STATE_HOME") {
        let p = PathBuf::from(p);
        if p.is_absolute() {
            return Ok(migrated_state_dir(
                p.join("diffz"),
                migrated_state_dir(p.join("diffr"), p.join("review-workbench")),
            ));
        }
    }
    let state = PathBuf::from(std::env::var_os("HOME").ok_or("HOME unset; specify --state-dir")?)
        .join(".local/state");
    Ok(migrated_state_dir(
        state.join("diffz"),
        migrated_state_dir(state.join("diffr"), state.join("review-workbench")),
    ))
}
/// Move state from the former directory name once. If that fails, retain the old location
/// so existing drafts and saved reviews remain available.
fn migrated_state_dir(new: PathBuf, old: PathBuf) -> PathBuf {
    if new.exists() || !old.exists() {
        return new;
    }
    match std::fs::rename(&old, &new) {
        Ok(()) => new,
        Err(_) => old,
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn temp_dir() -> PathBuf {
        std::env::temp_dir().join(format!(
            "diffz-migration-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ))
    }

    #[test]
    fn migrates_old_diffr_dir_to_diffz() {
        let root = temp_dir();
        std::fs::create_dir_all(root.join("diffr")).unwrap();

        let migrated = super::migrated_state_dir(root.join("diffz"), root.join("diffr"));

        assert_eq!(migrated, root.join("diffz"));
        assert!(root.join("diffz").is_dir());
        assert!(!root.join("diffr").exists());

        let _ = std::fs::remove_dir_all(&root);
    }
}
