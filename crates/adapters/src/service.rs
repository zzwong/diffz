//! Application service wiring. Provider access, storage, and publication meet here.
use crate::{
    Result, fixtures,
    github::{GithubProvider, GithubReader, GithubTarget},
    gitlab::{GitlabProvider, GitlabReader, GitlabTarget},
    local_git::{LocalGit, LocalMode},
    outbox::Outbox,
    process::{ProcessRequest, Runner, read_bounded, resolve_program},
    provider::ReviewProvider,
    store::Store,
};
use diffz_core::{
    domain::*,
    patch::{ParseLimits, parse_patch},
    provider::*,
    review::*,
};
use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::{
        Arc, Condvar, Mutex,
        atomic::{AtomicU64, Ordering},
    },
    time::{SystemTime, UNIX_EPOCH},
};

pub struct Services {
    store: Arc<Store>,
    providers: Vec<Arc<dyn ReviewProvider>>,
    outboxes: HashMap<ProviderId, Outbox>,
    git: Option<LocalGit>,
    ids: AtomicU64,
    nonce: String,
    reads: (Mutex<usize>, Condvar),
    gh: Option<PathBuf>,
    glab: Option<PathBuf>,
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
        let nonce = format!(
            "{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map_err(|_| "clock is before Unix epoch")?
                .as_nanos()
        );
        let mut services = Self {
            store,
            providers: vec![],
            outboxes: HashMap::new(),
            git: resolve_program("git").ok().map(LocalGit::new),
            ids: AtomicU64::new(1),
            nonce,
            reads: (Mutex::new(0), Condvar::new()),
            gh: resolve_program("gh").ok(),
            glab: resolve_program("glab").ok(),
        };
        let gh = services.gh.clone().map(|p| Arc::new(GithubReader::new(p)));
        let glab = services
            .glab
            .clone()
            .map(|p| Arc::new(GitlabReader::new(p)));
        services.register(Arc::new(GithubProvider::new(gh)), writes);
        services.register(Arc::new(GitlabProvider::new(glab)), gitlab_writes);
        Ok(services)
    }
    /// Adds a provider; one registered later for the same id takes the place of the first.
    pub fn register(&mut self, provider: Arc<dyn ReviewProvider>, writes: bool) {
        let rules = provider.rules();
        if writes && let Ok(remote) = provider.remote() {
            self.outboxes
                .insert(rules.id(), Outbox::new(self.store.clone(), rules, remote));
        }
        self.providers.push(provider);
    }
    fn provider_for(&self, id: &ProviderId) -> Result<&Arc<dyn ReviewProvider>> {
        self.providers
            .iter()
            .rev()
            .find(|p| p.rules().id() == *id)
            .ok_or_else(|| format!("No review provider named {id} is available").into())
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
            OpenRequest::Remote { provider, address } => {
                self.provider_for(&provider)?.open(&address, cancel)?
            }
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
        let bytes = self
            .provider_for(&target.provider)
            .and_then(|p| p.source(target, path, revision))
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

    fn blame(
        &self,
        s: Arc<Snapshot>,
        paths: Vec<String>,
        cancel: Cancellation,
    ) -> std::result::Result<diffz_core::review_details::BlameRead, ServiceError> {
        use diffz_core::review_details::{attribute, blame_span};
        let target = s
            .remote
            .as_ref()
            .ok_or("Release attribution needs a hosted compare")?;
        let spans: Vec<_> = paths
            .into_iter()
            .filter_map(|path| {
                let file = s.patch.files.iter().find(|f| f.display_path() == path)?;
                let (first, last) = blame_span(&s.overview, file)?;
                Some((path, first, last))
            })
            .collect();
        let found = {
            let _permit = self
                .permit(&cancel)
                .map_err(|e| ServiceError::from(e.to_string()))?;
            self.provider_for(&target.provider)
                .and_then(|p| p.blame(target, &spans, cancel.clone()))
                .unwrap_or_else(|_| vec![None; spans.len()])
        };
        // A cancelled read fails its files for no reason of theirs, so none of it counts.
        if cancel.cancelled() {
            return Err("Release attribution was cancelled".into());
        }
        Ok(spans
            .into_iter()
            .zip(found)
            .map(|((path, _, _), ranges)| {
                // Added lines always blame to some commit, so no ranges at all is a path the
                // provider could not resolve.
                let ranges = ranges.filter(|r| !r.is_empty());
                (path, ranges.map(|r| attribute(&s.overview.releases, &r)))
            })
            .collect())
    }
    fn save_blame(
        &self,
        id: &SnapshotId,
        releases_key: &str,
        found: &diffz_core::review_details::BlameRead,
    ) -> std::result::Result<(), ServiceError> {
        self.store
            .save_blame(id, releases_key, found)
            .map_err(|e| ServiceError::from(e.to_string()))
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
        let snapshot = self.store.snapshot(id).map_err(ServiceError::from)?;
        let target = snapshot.remote.as_ref().ok_or_else(|| {
            ServiceError::from("offline source; a hosted review target is required")
        })?;
        let rules = self.provider_for(&target.provider)?.rules();
        let p = PreparedReview::prepare(
            &*rules,
            OperationId(self.fresh_id()),
            &snapshot,
            drafts,
            verdict,
            summary,
        )
        .map_err(|e| ServiceError::from(e.to_string()))?;
        self.store
            .insert_prepared(&p, &*rules)
            .map_err(ServiceError::from)?;
        Ok(p)
    }
    fn publish(&self, p: PreparedReview) -> std::result::Result<OutboxEntry, ServiceError> {
        let Some(outbox) = self.outboxes.get(&p.target.provider) else {
            let rules = self.provider_for(&p.target.provider)?.rules();
            return Err(format!(
                "{} publication is disabled; restart with {} to confirm a review",
                rules.name(),
                rules.write_flag()
            )
            .into());
        };
        outbox.publish(p).map_err(Into::into)
    }
    fn outbox(&self) -> std::result::Result<Vec<OutboxEntry>, ServiceError> {
        self.store.outbox().map_err(Into::into)
    }
    fn reconcile(&self, id: OperationId) -> std::result::Result<OutboxEntry, ServiceError> {
        let entry = self.store.operation(&id).map_err(ServiceError::from)?;
        // Reconciliation only reads, so it remains available when publication is disabled.
        let provider = self.provider_for(&entry.prepared.target.provider)?;
        Outbox::new(self.store.clone(), provider.rules(), provider.remote()?)
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
        !self.outboxes.is_empty()
    }
    fn writes_enabled_for(&self, provider: &ProviderId) -> bool {
        self.outboxes.contains_key(provider)
    }
    fn providers(&self) -> Vec<Arc<dyn ReviewRules>> {
        self.providers.iter().map(|p| p.rules()).collect()
    }
    fn detect(&self, input: &str) -> Option<Detected> {
        detect_source(input)
    }
    fn unconfirmed_host(&self, request: &OpenRequest) -> Option<String> {
        let OpenRequest::Remote { provider, address } = request else {
            return None;
        };
        let host = remote_host(provider, address)?;
        if public_host(provider, &host) {
            return None;
        }
        let (program, tokens) = if *provider == ProviderId::GITHUB {
            (&self.gh, GITHUB_TOKENS)
        } else {
            (&self.glab, GITLAB_TOKENS)
        };
        let signed_in = program
            .as_ref()
            .is_some_and(|program| signed_in(program, &host, tokens));
        (!signed_in).then_some(host)
    }
    fn fresh_id(&self) -> String {
        format!(
            "{}-{}",
            self.nonce,
            self.ids.fetch_add(1, Ordering::Relaxed)
        )
    }
    fn releases(
        &self,
        snapshot: &Snapshot,
        cancel: Cancellation,
    ) -> std::result::Result<Snapshot, ServiceError> {
        let mut s = snapshot.clone();
        let Some(t) = s.remote.as_ref().filter(|t| t.compare.is_some()) else {
            return Ok(s);
        };
        let read = self
            .permit(&cancel)
            .and_then(|_permit| self.provider_for(&t.provider)?.releases(t, cancel.clone()));
        // Release metadata only adds to the compare, so failing to read it never fails the open.
        let (mut releases, warnings) = match read {
            Ok(read) => read,
            Err(e) => (
                vec![],
                vec![format!(
                    "The releases in this range could not be listed: {e}"
                )],
            ),
        };
        // A step can carry changes the compare leaves out, such as a merge from the base branch.
        let paths: std::collections::HashSet<String> =
            s.patch.files.iter().map(|f| f.display_path()).collect();
        for r in &mut releases {
            r.files.retain(|f| paths.contains(&f.path));
        }
        s.overview.releases = releases;
        for w in warnings {
            if !s.warnings.contains(&w) {
                s.warnings.push(w);
            }
        }
        if cancel.cancelled() {
            return Err("read cancelled".into());
        }
        self.store.put_snapshot(&s)?;
        Ok(s)
    }
}
const MAX_DETECTED: usize = 4096;
const GITHUB_TOKENS: &[&str] = &[
    "GH_TOKEN",
    "GITHUB_TOKEN",
    "GH_ENTERPRISE_TOKEN",
    "GITHUB_ENTERPRISE_TOKEN",
];
const GITLAB_TOKENS: &[&str] = &["GITLAB_TOKEN", "GITLAB_ACCESS_TOKEN", "OAUTH_TOKEN"];
/// The host an address names, which `gh` or `glab` would be asked to contact.
fn remote_host(provider: &ProviderId, address: &str) -> Option<String> {
    if *provider == ProviderId::GITHUB {
        match GithubTarget::parse(address).ok()? {
            GithubTarget::Pr(a) => Some(a.host),
            GithubTarget::Compare(a) => Some(a.host),
        }
    } else {
        match GitlabTarget::parse(address).ok()? {
            GitlabTarget::Mr(a) => Some(a.host),
            GitlabTarget::Compare(a) => Some(a.host),
        }
    }
}
/// A provider's own public host, which an address may name without the user having signed in.
fn public_host(provider: &ProviderId, host: &str) -> bool {
    let public = if *provider == ProviderId::GITHUB {
        "github.com"
    } else if *provider == ProviderId::GITLAB {
        "gitlab.com"
    } else {
        return false;
    };
    host.eq_ignore_ascii_case(public)
}
/// Whether the CLI holds stored credentials for `host`. Token variables are removed so that a
/// token meant for one host is never offered to, or counted as access to, another.
fn signed_in(program: &Path, host: &str, tokens: &[&str]) -> bool {
    let mut request =
        ProcessRequest::new(program.to_path_buf()).args(["auth", "status", "--hostname", host]);
    request.deadline = std::time::Duration::from_secs(15);
    request.env_remove = tokens.iter().map(Into::into).collect();
    Runner::run(request, Cancellation::default()).is_ok_and(|out| out.status.success())
}
/// What free text names: a patch file that exists, or an address one provider's own parser
/// takes, passed on unchanged. Nothing runs and no store is opened. Multi-line and oversized
/// text is never a source.
pub fn detect_source(input: &str) -> Option<Detected> {
    let input = input.trim();
    if input.is_empty() || input.len() > MAX_DETECTED || input.contains('\n') {
        return None;
    }
    if Path::new(input).is_file() {
        return Some(Detected {
            request: OpenRequest::Patch(
                std::path::absolute(input).unwrap_or_else(|_| PathBuf::from(input)),
            ),
            label: "Patch file".into(),
        });
    }
    let (provider, label) = match (GithubTarget::parse(input), GitlabTarget::parse(input)) {
        (Ok(GithubTarget::Pr(_)), _) => (ProviderId::GITHUB, "GitHub pull request"),
        (Ok(GithubTarget::Compare(_)), _) => (ProviderId::GITHUB, "GitHub compare"),
        (_, Ok(GitlabTarget::Mr(_))) => (ProviderId::GITLAB, "GitLab merge request"),
        (_, Ok(GitlabTarget::Compare(_))) => (ProviderId::GITLAB, "GitLab compare"),
        _ => return None,
    };
    Some(Detected {
        request: OpenRequest::Remote {
            provider,
            address: input.into(),
        },
        label: label.into(),
    })
}
/// The state directory an app bundle carries in `Contents/Resources/state-dir`, so a development
/// bundle keeps its own handoff socket and lock however it is started: from the Dock, through
/// `open`, or from a symlink to its executable. A bundle that carries the file but not a usable
/// path is an error, never a silent fall back to the release state.
#[cfg(any(test, target_os = "macos"))]
fn bundled_state_dir(exe: &Path) -> Result<Option<PathBuf>> {
    let (Some(macos), Some(bundle)) = (exe.parent(), exe.ancestors().nth(3)) else {
        return Ok(None);
    };
    let contents = macos.parent().unwrap_or(macos);
    if macos.file_name() != Some("MacOS".as_ref())
        || contents.file_name() != Some("Contents".as_ref())
        || bundle.extension() != Some("app".as_ref())
    {
        return Ok(None);
    }
    let file = contents.join("Resources/state-dir");
    let text = match std::fs::read_to_string(&file) {
        Ok(text) => text,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => {
            return Err(format!(
                "Contents/Resources/state-dir must hold an absolute path; {}: {e}",
                file.display()
            )
            .into());
        }
    };
    let dir = PathBuf::from(text.trim());
    if !dir.is_absolute() {
        return Err(format!(
            "Contents/Resources/state-dir must hold an absolute path, got '{}'",
            text.trim()
        )
        .into());
    }
    Ok(Some(dir))
}
pub fn default_state_dir() -> Result<PathBuf> {
    #[cfg(target_os = "macos")]
    if let Ok(exe) = std::env::current_exe().and_then(|exe| exe.canonicalize())
        && let Some(dir) = bundled_state_dir(&exe)?
    {
        return Ok(dir);
    }
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
    use diffz_core::provider::OpenRequest;
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

    #[test]
    fn remote_addresses_go_to_the_provider_that_parses_them() {
        use diffz_core::domain::ProviderId;
        for (address, provider, label) in [
            ("owner/repo#12", ProviderId::GITHUB, "GitHub pull request"),
            (
                "https://github.com/owner/repo/pull/12/files",
                ProviderId::GITHUB,
                "GitHub pull request",
            ),
            (
                "https://github.com/owner/repo/compare/v1.0...v2.0",
                ProviderId::GITHUB,
                "GitHub compare",
            ),
            (
                "group/sub/project!7",
                ProviderId::GITLAB,
                "GitLab merge request",
            ),
            (
                "https://gitlab.example.com/group/project/-/merge_requests/7",
                ProviderId::GITLAB,
                "GitLab merge request",
            ),
            (
                "https://gitlab.com/group/project/-/compare/v1.0...v2.0",
                ProviderId::GITLAB,
                "GitLab compare",
            ),
        ] {
            let detected = super::detect_source(&format!("  {address}\n")).expect(address);
            assert_eq!(
                detected.request,
                OpenRequest::Remote {
                    provider,
                    address: address.into()
                },
                "{address}"
            );
            assert_eq!(detected.label, label, "{address}");
        }
    }

    #[test]
    fn existing_files_are_patches_and_other_text_is_nothing() {
        let dir = temp_dir();
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("change.patch");
        std::fs::write(&file, "").unwrap();
        let path = file.to_str().unwrap();

        let detected = super::detect_source(path).unwrap();
        assert_eq!(detected.request, OpenRequest::Patch(file.clone()));
        assert_eq!(detected.label, "Patch file");
        // A directory is a repository for the local modes, never a patch.
        assert!(super::detect_source(dir.to_str().unwrap()).is_none());
        let _ = std::fs::remove_dir_all(&dir);

        for text in [
            "change.patch",
            "owner/repo",
            "",
            "   ",
            "owner/repo#1\nowner/repo#2",
            "https://example.com/",
        ] {
            assert!(super::detect_source(text).is_none(), "{text:?}");
        }
        assert!(super::detect_source(&format!("owner/repo#1 {}", "x".repeat(5000))).is_none());
    }

    #[test]
    fn relative_patch_paths_are_absolutized() {
        let name = format!("diffz-detect-{}.patch", std::process::id());
        std::fs::write(&name, "").unwrap();
        let detected = super::detect_source(&name);
        let _ = std::fs::remove_file(&name);
        let Some(OpenRequest::Patch(path)) = detected.map(|d| d.request) else {
            panic!("not detected as a patch");
        };
        assert!(path.is_absolute() && path.ends_with(&name), "{path:?}");
    }

    #[test]
    fn only_a_providers_public_host_needs_no_sign_in() {
        use diffz_core::domain::ProviderId;
        for (provider, host, public) in [
            (ProviderId::GITHUB, "github.com", true),
            (ProviderId::GITHUB, "GitHub.com", true),
            (ProviderId::GITLAB, "gitlab.com", true),
            (ProviderId::GITHUB, "gitlab.com", false),
            (ProviderId::GITLAB, "github.com", false),
            (ProviderId::GITHUB, "github.example.com", false),
            (ProviderId::GITHUB, "github.com.evil.example", false),
            (ProviderId::GITLAB, "gitlab.example.com", false),
        ] {
            assert_eq!(super::public_host(&provider, host), public, "{host}");
        }
        let github = ProviderId::GITHUB;
        assert_eq!(
            super::remote_host(&github, "https://ghe.example.com/o/r/pull/1").as_deref(),
            Some("ghe.example.com")
        );
        assert_eq!(
            super::remote_host(&ProviderId::GITLAB, "group/project!7"),
            super::remote_host(
                &ProviderId::GITLAB,
                "https://gitlab.com/group/project/-/merge_requests/7"
            ),
        );
    }

    #[test]
    fn a_bundle_state_dir_must_be_usable_when_present() {
        let root = temp_dir();
        let exe = root.join("Diffz Dev.app/Contents/MacOS/diffz");
        let resources = root.join("Diffz Dev.app/Contents/Resources");
        std::fs::create_dir_all(exe.parent().unwrap()).unwrap();
        std::fs::create_dir_all(&resources).unwrap();
        // No file: a release bundle, which uses the default location.
        assert_eq!(super::bundled_state_dir(&exe).unwrap(), None);
        std::fs::write(resources.join("state-dir"), "/tmp/diffz-dev\n").unwrap();
        assert_eq!(
            super::bundled_state_dir(&exe).unwrap(),
            Some(PathBuf::from("/tmp/diffz-dev"))
        );
        for bad in ["relative/dir\n", "\n", ""] {
            std::fs::write(resources.join("state-dir"), bad).unwrap();
            let err = super::bundled_state_dir(&exe).unwrap_err().to_string();
            assert!(err.contains("must hold an absolute path"), "{bad:?}: {err}");
        }
        // Unreadable content is an error too, not a fall back.
        std::fs::remove_file(resources.join("state-dir")).unwrap();
        std::fs::create_dir(resources.join("state-dir")).unwrap();
        assert!(super::bundled_state_dir(&exe).is_err());
        // A directory that is not a `.app` bundle is not one.
        let plain = root.join("Diffz/Contents/MacOS/diffz");
        std::fs::create_dir_all(root.join("Diffz/Contents/Resources")).unwrap();
        std::fs::write(root.join("Diffz/Contents/Resources/state-dir"), "relative").unwrap();
        assert_eq!(super::bundled_state_dir(&plain).unwrap(), None);
        let _ = std::fs::remove_dir_all(&root);
    }

    /// A compare source whose releases name one file outside its diff, and can fail or be
    /// replaced while they are read.
    mod releases {
        use crate::{
            Result,
            provider::{ReviewProvider, ReviewRemote},
            service::Services,
        };
        use diffz_core::{
            domain::*,
            patch::parse_patch,
            provider::*,
            review::PreparedReview,
            review_details::{Release, ReleaseFile},
        };
        use serde_json::value::RawValue;
        use std::sync::{Arc, Mutex};

        #[derive(Default)]
        struct Fake {
            fail: bool,
            /// Cancelled mid-read, as the window does when the snapshot is replaced.
            replaced: Mutex<Option<Cancellation>>,
        }
        struct Rules;
        impl ReviewRules for Rules {
            fn id(&self) -> ProviderId {
                ProviderId::new("Fake")
            }
            fn name(&self) -> &str {
                "Fake"
            }
            fn open_label(&self) -> &str {
                ""
            }
            fn address_label(&self) -> &str {
                ""
            }
            fn address_hint(&self) -> &str {
                ""
            }
            fn address_help(&self) -> &str {
                ""
            }
            fn write_flag(&self) -> &str {
                ""
            }
            fn reopen_address(&self, _: &RemoteTarget) -> String {
                String::new()
            }
            fn line_url(&self, _: &RemoteTarget, _: &str, _: &str, _: u32) -> String {
                String::new()
            }
            fn payload(&self, _: &PreparedReview) -> Box<RawValue> {
                RawValue::from_string("{}".into()).unwrap()
            }
        }
        fn file(path: &str) -> ReleaseFile {
            ReleaseFile {
                path: path.into(),
                additions: 1,
                deletions: 0,
                previous: None,
            }
        }
        fn release(tag: &str, files: Vec<ReleaseFile>) -> Release {
            Release {
                tag: Some(tag.into()),
                commit: "c".repeat(40),
                date: None,
                commits: 1,
                files,
                notes: None,
                url: None,
                shas: vec![],
            }
        }
        impl ReviewProvider for Fake {
            fn rules(&self) -> Arc<dyn ReviewRules> {
                Arc::new(Rules)
            }
            fn open(&self, _: &str, _: Cancellation) -> Result<Snapshot> {
                let patch = parse_patch(
                    b"diff --git a/a.rs b/a.rs\n--- a/a.rs\n+++ b/a.rs\n@@ -1 +1 @@\n-a\n+b\n",
                    Default::default(),
                )?;
                let target = RemoteTarget {
                    provider: ProviderId::new("Fake"),
                    repository: RepositoryKey {
                        host: "example.com".into(),
                        id: 1,
                        owner: "o".into(),
                        name: "r".into(),
                    },
                    account: String::new(),
                    pr: 0,
                    target_tip: "a".repeat(40),
                    comparison_base: "a".repeat(40),
                    head: "c".repeat(40),
                    open: true,
                    draft: false,
                    merged: false,
                    pending_review: false,
                    compare: Some(CompareRefs {
                        base: "v1".into(),
                        head: "v3".into(),
                        direct: false,
                    }),
                };
                Ok(Snapshot::with_origin(
                    "o/r  v1...v3".into(),
                    patch,
                    Some(target),
                    vec![],
                    "fake-compare".into(),
                ))
            }
            fn accepts(&self, _: &str) -> bool {
                true
            }
            fn source(&self, _: &RemoteTarget, _: &str, _: &str) -> Result<Vec<u8>> {
                Err("no source".into())
            }
            fn remote(&self) -> Result<Arc<dyn ReviewRemote>> {
                Err("read-only".into())
            }
            fn releases(
                &self,
                _: &RemoteTarget,
                cancel: Cancellation,
            ) -> Result<(Vec<Release>, Vec<String>)> {
                if self.fail {
                    return Err("HTTP 502".into());
                }
                if let Some(replaced) = self.replaced.lock().unwrap().take() {
                    replaced.cancel();
                    assert!(cancel.cancelled());
                }
                // v3 only merged the base branch in, so its one file is not in the compare.
                Ok((
                    vec![
                        release("v2", vec![file("a.rs"), file("base-only.rs")]),
                        release("v3", vec![file("base-only.rs")]),
                    ],
                    vec!["one warning".into()],
                ))
            }
        }
        fn services(fake: Fake) -> (tempfile::TempDir, Services) {
            let temp = tempfile::tempdir().unwrap();
            let mut services = Services::new(&temp.path().join("db"), false).unwrap();
            services.register(Arc::new(fake), false);
            (temp, services)
        }
        fn open(services: &Services) -> Opened {
            let request = OpenRequest::Remote {
                provider: ProviderId::new("Fake"),
                address: "o/r v1...v3".into(),
            };
            services.open(request, Cancellation::default()).unwrap()
        }
        fn resume(services: &Services, id: &SnapshotId) -> Snapshot {
            services
                .open(OpenRequest::Resume(id.clone()), Cancellation::default())
                .unwrap()
                .snapshot
        }

        #[test]
        fn a_compare_opens_before_its_releases_and_saves_them_after() {
            let (_temp, services) = services(Fake::default());
            let opened = open(&services);
            assert!(opened.snapshot.overview.releases.is_empty());
            let s = services
                .releases(&opened.snapshot, Cancellation::default())
                .unwrap();
            assert_eq!(s.id, opened.snapshot.id);
            // Files outside the compare are dropped, leaving v3 with none to narrow the tree to.
            let files: Vec<Vec<&str>> = s
                .overview
                .releases
                .iter()
                .map(|r| r.files.iter().map(|f| f.path.as_str()).collect())
                .collect();
            assert_eq!(files, [vec!["a.rs"], vec![]]);
            assert_eq!(s.warnings, ["one warning"]);
            assert_eq!(
                resume(&services, &s.id).overview.releases,
                s.overview.releases
            );
            // Reading them again, as a resumed or refreshed compare does, repeats no warning.
            let again = services.releases(&s, Cancellation::default()).unwrap();
            assert_eq!(again.warnings, ["one warning"]);
        }

        #[test]
        fn releases_for_a_replaced_snapshot_are_neither_returned_nor_saved() {
            let cancel = Cancellation::default();
            let (_temp, services) = services(Fake {
                replaced: Mutex::new(Some(cancel.clone())),
                ..Fake::default()
            });
            let opened = open(&services);
            assert!(services.releases(&opened.snapshot, cancel).is_err());
            assert!(
                resume(&services, &opened.snapshot.id)
                    .overview
                    .releases
                    .is_empty()
            );
        }

        #[test]
        fn unreadable_releases_are_a_warning_on_the_open_compare() {
            let (_temp, services) = services(Fake {
                fail: true,
                ..Fake::default()
            });
            let opened = open(&services);
            let s = services
                .releases(&opened.snapshot, Cancellation::default())
                .unwrap();
            assert!(s.overview.releases.is_empty());
            assert_eq!(
                s.warnings,
                ["The releases in this range could not be listed: HTTP 502"]
            );
            assert_eq!(resume(&services, &s.id).warnings, s.warnings);
        }
    }
}
