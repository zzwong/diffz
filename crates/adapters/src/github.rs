//! gh CLI transport: fixed host and account, capped pagination, immutable PR snapshots that stay coherent.
use crate::{
    AdapterError, Result,
    process::{ProcessOutput, ProcessRequest, Runner, stderr_excerpt},
};
use diffz_core::{
    domain::*,
    patch::{ParseLimits, parse_patch},
    provider::Cancellation,
    review::PreparedReview,
};
use serde_json::Value;
use std::{path::PathBuf, sync::Arc, time::Duration};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PrAddress {
    pub host: String,
    pub owner: String,
    pub repo: String,
    pub number: u64,
}
fn identifier(s: &str) -> bool {
    !s.is_empty()
        && s != "."
        && s != ".."
        && s.len() <= 100
        && s.bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"-_.".contains(&b))
}
impl PrAddress {
    pub fn parse(value: &str) -> Result<Self> {
        let value = value.trim();
        if value.starts_with("https://") {
            if value.split('/').any(|p| p == ".." || p == ".") {
                return Err("relative URL segments are not accepted as a PR identity".into());
            }
            let u = url::Url::parse(value)
                .map_err(|_| AdapterError::Message("invalid PR URL".into()))?;
            if !u.username().is_empty()
                || u.password().is_some()
                || u.port().is_some()
                || u.query().is_some()
            {
                return Err(
                    "credentials, ports, and query strings are not allowed in a PR URL".into(),
                );
            }
            let host = u
                .host_str()
                .ok_or("PR URL has no host")?
                .to_ascii_lowercase();
            let parts: Vec<&str> = u.path().trim_end_matches('/').split('/').skip(1).collect();
            if parts.len() < 4
                || parts.len() > 5
                || parts[2] != "pull"
                || parts
                    .get(4)
                    .is_some_and(|p| !matches!(*p, "files" | "commits"))
            {
                return Err("the URL must look like https://HOST/OWNER/REPO/pull/NUMBER".into());
            }
            Self::checked(host, parts[0], parts[1], parts[3])
        } else {
            let (slug, n) = value.split_once('#').ok_or(
                "pass an owner/repo#number or a full PR URL; a bare number alone is ambiguous",
            )?;
            let (o, r) = slug.split_once('/').ok_or("expected owner/repo#number")?;
            Self::checked("github.com".into(), o, r, n)
        }
    }
    fn checked(host: String, owner: &str, repo: &str, n: &str) -> Result<Self> {
        if !identifier(owner) || !identifier(repo) || !n.bytes().all(|c| c.is_ascii_digit()) {
            return Err("bad repository identity, or a bad PR number".into());
        }
        let number = n
            .parse::<u64>()
            .map_err(|_| AdapterError::Message("invalid PR number".into()))?;
        if number == 0 {
            return Err("PR number must be positive".into());
        }
        Ok(Self {
            host,
            owner: owner.into(),
            repo: repo.into(),
            number,
        })
    }
    pub fn from_target(t: &RemoteTarget) -> Self {
        Self {
            host: t.repository.host.clone(),
            owner: t.repository.owner.clone(),
            repo: t.repository.name.clone(),
            number: t.pr,
        }
    }
    pub fn root(&self) -> String {
        format!("repos/{}/{}/pulls/{}", self.owner, self.repo, self.number)
    }
}
#[derive(Debug)]
pub struct HttpResponse {
    pub status: u16,
    pub headers: String,
    pub body: Vec<u8>,
}
pub fn decode_http(bytes: &[u8]) -> Result<HttpResponse> {
    let (at, skip) = if let Some(i) = bytes.windows(4).position(|w| w == b"\r\n\r\n") {
        (i, 4)
    } else if let Some(i) = bytes.windows(2).position(|w| w == b"\n\n") {
        (i, 2)
    } else {
        return Err("no HTTP headers in the gh output could be parsed".into());
    };
    let headers = std::str::from_utf8(&bytes[..at])
        .map_err(|_| AdapterError::Message("invalid HTTP header encoding".into()))?;
    let mut first = headers
        .lines()
        .next()
        .ok_or("missing HTTP status")?
        .split_whitespace();
    if !first.next().unwrap_or_default().starts_with("HTTP/") {
        return Err("gh did not print an HTTP response".into());
    }
    let status = first
        .next()
        .and_then(|s| s.parse::<u16>().ok())
        .filter(|s| (100..600).contains(s))
        .ok_or("invalid HTTP status")?;
    // Proxy and interim headers get skipped here instead of being parsed as the response body.
    if status < 200 {
        return decode_http(&bytes[at + skip..]);
    }
    Ok(HttpResponse {
        status,
        headers: headers.into(),
        body: bytes[at + skip..].to_vec(),
    })
}
/// Explains a CLI run without a parseable response, including what the tool itself reported.
pub(crate) fn incomplete_http(tool: &str, output: &ProcessOutput) -> AdapterError {
    let said = stderr_excerpt(&output.stderr)
        .map(|e| format!("; {tool} reported: {e}"))
        .unwrap_or_default();
    AdapterError::Message(format!(
        "{tool} exited before a full HTTP response arrived (exit {:?}){said}; verify {tool} auth status covers this host",
        output.status.code()
    ))
}
/// Percent-encode repo paths so they fit inside a URL.
pub fn encode_path(path: &str) -> String {
    const HEX: &[u8; 16] = b"0123456789ABCDEF";
    let mut out = String::with_capacity(path.len());
    for b in path.bytes() {
        let unreserved = b.is_ascii_alphanumeric() || matches!(b, b'-' | b'.' | b'_' | b'~' | b'/');
        if unreserved {
            out.push(b as char);
        } else {
            out.push('%');
            out.push(HEX[(b >> 4) as usize] as char);
            out.push(HEX[(b & 0x0f) as usize] as char);
        }
    }
    out
}
fn text<'a>(v: &'a Value, path: &str) -> Result<&'a str> {
    v.pointer(path)
        .and_then(Value::as_str)
        .ok_or_else(|| format!("GitHub response is missing {path}").into())
}
fn oid(v: &Value, path: &str) -> Result<String> {
    let s = text(v, path)?;
    if !matches!(s.len(), 40 | 64) || !s.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err("the object ID GitHub sent is not valid".into());
    }
    Ok(s.to_ascii_lowercase())
}
fn number(v: &Value, path: &str) -> Result<u64> {
    v.pointer(path)
        .and_then(Value::as_u64)
        .ok_or_else(|| format!("GitHub response is missing {path}").into())
}

#[derive(Clone)]
pub struct GithubReader {
    program: PathBuf,
}
impl GithubReader {
    pub fn new(program: PathBuf) -> Self {
        Self { program }
    }
    fn request(
        &self,
        host: &str,
        endpoint: &str,
        method: &str,
        body: Option<&Value>,
        accept: &str,
        cancel: Cancellation,
    ) -> Result<HttpResponse> {
        // Every endpoint is built from a validated identity plus numeric IDs; no call site passes a remote link.
        if endpoint.starts_with('-') || endpoint.starts_with('/') || endpoint.contains("://") {
            return Err("invalid relative GitHub API endpoint".into());
        }
        let mut r = ProcessRequest::new(self.program.clone()).args([
            "api",
            "--hostname",
            host,
            "--include",
            "--method",
            method,
            "-H",
            accept,
            "-H",
            "X-GitHub-Api-Version: 2026-03-10",
            endpoint,
        ]);
        r.deadline = Duration::from_secs(60);
        if let Some(body) = body {
            r.args.extend(["--input".into(), "-".into()]);
            r.stdin = serde_json::to_vec(body)?;
        }
        let output = Runner::run(r, cancel)?;
        decode_http(&output.stdout).map_err(|_| incomplete_http("gh", &output))
    }
    pub fn source(&self, t: &RemoteTarget, path: &str, revision: &str) -> Result<Vec<u8>> {
        let endpoint = format!(
            "repos/{}/{}/contents/{}?ref={}",
            t.repository.owner,
            t.repository.name,
            encode_path(path),
            encode_path(revision)
        );
        let r = self.request(
            &t.repository.host,
            &endpoint,
            "GET",
            None,
            "Accept: application/vnd.github.raw+json",
            Cancellation::default(),
        )?;
        if r.status != 200 {
            return Err(format!("Source read returned HTTP {}", r.status).into());
        }
        Ok(r.body)
    }
    pub fn get_json(&self, host: &str, endpoint: &str, cancel: Cancellation) -> Result<Value> {
        let r = self.request(
            host,
            endpoint,
            "GET",
            None,
            "Accept: application/vnd.github+json",
            cancel,
        )?;
        if r.status != 200 {
            return Err(format!(
                "GitHub read failed with HTTP {}; no credentials or content were logged",
                r.status
            )
            .into());
        }
        Ok(serde_json::from_slice(&r.body)?)
    }
    pub fn pages(&self, host: &str, endpoint: &str, cancel: Cancellation) -> Result<Vec<Value>> {
        let mut all = vec![];
        let mut bytes = 0;
        for page in 1..=100 {
            let sep = if endpoint.contains('?') { "&" } else { "?" };
            let value = self.get_json(
                host,
                &format!("{endpoint}{sep}per_page=100&page={page}"),
                cancel.clone(),
            )?;
            bytes += serde_json::to_vec(&value)?.len();
            if bytes > 64 * 1024 * 1024 {
                return Err(
                    "paged GitHub data passed 64 MiB, so completeness is not guaranteed".into(),
                );
            }
            let items = value.as_array().ok_or("expected GitHub array page")?;
            let done = items.len() < 100;
            all.extend(items.iter().cloned());
            if done {
                return Ok(all);
            }
        }
        Err("GitHub pagination limit reached; truncated responses are never accepted".into())
    }
    pub fn account(&self, host: &str, cancel: Cancellation) -> Result<String> {
        Ok(text(&self.get_json(host, "user", cancel)?, "/login")?.into())
    }
    pub fn metadata(&self, a: &PrAddress, cancel: Cancellation) -> Result<Value> {
        self.get_json(&a.host, &a.root(), cancel)
    }
    pub fn reviews(&self, t: &RemoteTarget, cancel: Cancellation) -> Result<Vec<Value>> {
        let a = PrAddress::from_target(t);
        self.pages(&a.host, &format!("{}/reviews", a.root()), cancel)
    }
    pub fn review_comments(
        &self,
        t: &RemoteTarget,
        id: u64,
        cancel: Cancellation,
    ) -> Result<Vec<Value>> {
        let a = PrAddress::from_target(t);
        self.pages(
            &a.host,
            &format!("{}/reviews/{id}/comments", a.root()),
            cancel,
        )
    }
    pub fn current_target(&self, t: &RemoteTarget, cancel: Cancellation) -> Result<RemoteTarget> {
        let a = PrAddress::from_target(t);
        let account = self.account(&a.host, cancel.clone())?;
        let meta = self.metadata(&a, cancel.clone())?;
        let base = oid(&meta, "/base/sha")?;
        let head = oid(&meta, "/head/sha")?;
        let comparison = self.get_json(
            &a.host,
            &format!("repos/{}/{}/compare/{base}...{head}", a.owner, a.repo),
            cancel.clone(),
        )?;
        let mut target = target(
            &a,
            &meta,
            account,
            oid(&comparison, "/merge_base_commit/sha")?,
        )?;
        target.pending_review = self
            .reviews(&target, cancel)?
            .iter()
            .any(|r| r["state"] == "PENDING" && r["user"]["login"] == target.account);
        Ok(target)
    }
    fn named_pages(
        &self,
        host: &str,
        endpoint: &str,
        field: &str,
        cancel: Cancellation,
    ) -> Result<Vec<Value>> {
        let mut all = Vec::new();
        let mut bytes = 0;
        for page in 1..=10 {
            let sep = if endpoint.contains('?') { "&" } else { "?" };
            let v = self.get_json(
                host,
                &format!("{endpoint}{sep}per_page=100&page={page}"),
                cancel.clone(),
            )?;
            bytes += serde_json::to_vec(&v)?.len();
            if bytes > 16 * 1024 * 1024 {
                return Err("check metadata exceeds 16 MiB".into());
            }
            let rows = v[field]
                .as_array()
                .ok_or("missing check metadata collection")?;
            all.extend(rows.iter().cloned());
            if rows.len() < 100 {
                return Ok(all);
            }
        }
        Err("check metadata exceeds 1,000 entries".into())
    }
    fn overview(
        &self,
        a: &PrAddress,
        metadata: &Value,
        head: &str,
        cancel: Cancellation,
    ) -> diffz_core::review_details::Overview {
        use diffz_core::review_details::*;
        let mut overview = Overview {
            description: metadata["body"].as_str().map(str::to_owned),
            author: metadata["user"]["login"].as_str().map(str::to_owned),
            captured_at: Some(
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_secs(),
            ),
            ..Default::default()
        };
        let repo = format!("repos/{}/{}", a.owner, a.repo);
        for (kind, endpoint, field) in [
            (
                "Check",
                format!("{repo}/commits/{head}/check-runs"),
                "check_runs",
            ),
            (
                "Workflow",
                format!("{repo}/actions/runs?head_sha={head}"),
                "workflow_runs",
            ),
            (
                "Status",
                format!("{repo}/commits/{head}/status"),
                "statuses",
            ),
        ] {
            match self.named_pages(&a.host, &endpoint, field, cancel.clone()) {
                Ok(rows) => overview
                    .checks
                    .extend(rows.iter().map(|v| check_row(v, kind))),
                Err(e) => overview
                    .notices
                    .push(format!("{kind} metadata unavailable: {e}")),
            }
        }
        match self.pages(
            &a.host,
            &format!("{repo}/issues/{}/comments", a.number),
            cancel,
        ) {
            Ok(rows) => {
                overview.conversation = rows
                    .iter()
                    .filter_map(|v| {
                        Some(ConversationComment {
                            id: v["id"].as_u64()?,
                            author: v["user"]["login"].as_str()?.into(),
                            body: v["body"].as_str()?.into(),
                            created_at: v["created_at"].as_str().map(Into::into),
                        })
                    })
                    .collect()
            }
            Err(e) => overview
                .notices
                .push(format!("Conversation unavailable: {e}")),
        }
        overview
    }
    pub fn snapshot(&self, a: &PrAddress, cancel: Cancellation) -> Result<Snapshot> {
        let account = self.account(&a.host, cancel.clone())?;
        for _ in 0..3 {
            let before = self.metadata(a, cancel.clone())?;
            let base = oid(&before, "/base/sha")?;
            let head = oid(&before, "/head/sha")?;
            let compare = self.get_json(
                &a.host,
                &format!("repos/{}/{}/compare/{base}...{head}", a.owner, a.repo),
                cancel.clone(),
            )?;
            let mut remote = target(
                a,
                &before,
                account.clone(),
                oid(&compare, "/merge_base_commit/sha")?,
            )?;
            let files = self.pages(&a.host, &format!("{}/files", a.root()), cancel.clone())?;
            let raw = self.request(
                &a.host,
                &a.root(),
                "GET",
                None,
                "Accept: application/vnd.github.diff",
                cancel.clone(),
            )?;
            let from_files = raw.status == 406;
            let body = match raw.status {
                200 => raw.body,
                // The unified diff stops at 300 files on GitHub; hunks stay available per file.
                406 => patch_from_files(&files),
                status => {
                    return Err(format!(
                        "GitHub diff failed (HTTP {status}); the existing session stays as is"
                    )
                    .into());
                }
            };
            let patch = parse_patch(&body, ParseLimits::default())?;
            drop(body);
            let comments =
                self.pages(&a.host, &format!("{}/comments", a.root()), cancel.clone())?;
            let reviews = self.reviews(&remote, cancel.clone())?;
            remote.pending_review = reviews
                .iter()
                .any(|r| r["state"] == "PENDING" && r["user"]["login"] == account);
            let after = self.metadata(a, cancel.clone())?;
            if base != oid(&after, "/base/sha")?
                || head != oid(&after, "/head/sha")?
                || before["base"]["repo"]["id"] != after["base"]["repo"]["id"]
            {
                continue;
            }
            remote.open = after["state"] == "open";
            remote.draft = after["draft"] == true;
            // Which account was used is frozen into the review identity too.
            if self.account(&a.host, cancel.clone())? != account {
                return Err("the GitHub account changed while the snapshot was captured".into());
            }
            let mut s = Snapshot::new(
                format!(
                    "{} · {}/{} #{}",
                    text(&after, "/title")?,
                    a.owner,
                    a.repo,
                    a.number
                ),
                patch,
                Some(remote),
                comments.iter().map(thread).collect::<Result<_>>()?,
            );
            if from_files {
                let omitted = files
                    .iter()
                    .filter(|f| f["patch"].is_null() && f["changes"].as_u64().unwrap_or(0) > 0)
                    .count();
                s.warnings.push(format!(
                    "GitHub serves no unified diff past 300 files. This review then comes via the per-file endpoint, and GitHub had already omitted the text for {omitted} of those files."
                ));
            }
            s.overview = self.overview(a, &after, &head, cancel.clone());
            s.overview.decision = diffz_core::review_details::review_decision(
                reviews
                    .iter()
                    .filter_map(|r| Some((r["user"]["login"].as_str()?, r["state"].as_str()?))),
            );
            s.overview
                .conversation
                .extend(reviews.iter().filter_map(|v| {
                    let body = v["body"].as_str().filter(|body| !body.trim().is_empty())?;
                    Some(diffz_core::review_details::ConversationComment {
                        id: v["id"].as_u64()?,
                        author: v["user"]["login"].as_str()?.into(),
                        body: body.into(),
                        created_at: v["submitted_at"].as_str().map(Into::into),
                    })
                }));
            if self.account(&a.host, cancel.clone())? != account {
                return Err("the GitHub account changed while the overview was captured".into());
            }
            let expected = number(&after, "/changed_files")? as usize;
            if expected != files.len() || expected != s.patch.files.len() {
                s.warnings.push(format!(
                    "coverage disagreement: metadata {expected}, file count {}, patch count {}",
                    files.len(),
                    s.patch.files.len()
                ));
            }
            let expected_paths: std::collections::BTreeSet<String> = files
                .iter()
                .filter_map(|f| f["filename"].as_str().map(str::to_owned))
                .collect();
            let actual_paths: std::collections::BTreeSet<String> = s
                .patch
                .files
                .iter()
                .filter_map(|f| f.path().utf8().ok().map(str::to_owned))
                .collect();
            if expected_paths != actual_paths {
                s.warnings.push(
                    "file paths from the provider disagree with the canonical patch paths".into(),
                )
            }
            // Binary and mode-only files are fine; the coverage warning covers missing text patches only.
            for f in &files {
                if f["patch"].is_null() && f["changes"].as_u64().unwrap_or(0) > 0 {
                    let name = f["filename"].as_str().unwrap_or_default();
                    let captured = s.patch.files.iter().any(|p| {
                        p.path().utf8().ok() == Some(name)
                            && (!p.hunks.is_empty()
                                || p.content != diffz_core::patch::ContentKind::Text)
                    });
                    if !captured {
                        s.warnings.push(format!("provider omitted text for {name}"));
                    }
                }
            }
            return Ok(s);
        }
        Err(
            "the PR moved throughout the snapshot attempt; the old session and its drafts are kept"
                .into(),
        )
    }
}
pub fn check_row(v: &Value, kind: &str) -> diffz_core::review_details::Check {
    let legacy = kind == "Status";
    let state = v["state"].as_str().unwrap_or("unknown");
    diffz_core::review_details::Check {
        name: v[if legacy { "context" } else { "name" }]
            .as_str()
            .unwrap_or("Unnamed check")
            .into(),
        kind: kind.into(),
        status: if legacy {
            if state == "pending" {
                "queued"
            } else {
                "completed"
            }
        } else {
            v["status"].as_str().unwrap_or("unknown")
        }
        .into(),
        conclusion: if legacy {
            Some(state.into())
        } else {
            v["conclusion"].as_str().map(str::to_owned)
        },
        url: v[if legacy { "target_url" } else { "html_url" }]
            .as_str()
            .filter(|u| u.starts_with("https://"))
            .map(str::to_owned),
    }
}

fn target(
    a: &PrAddress,
    m: &Value,
    account: String,
    comparison_base: String,
) -> Result<RemoteTarget> {
    let owner = text(m, "/base/repo/owner/login")?;
    let name = text(m, "/base/repo/name")?;
    if !owner.eq_ignore_ascii_case(&a.owner) || !name.eq_ignore_ascii_case(&a.repo) {
        return Err(
            "the repository moved to another identity; open the PR at its canonical URL".into(),
        );
    }
    Ok(RemoteTarget {
        provider: diffz_core::domain::ProviderKind::GitHub,
        repository: RepositoryKey {
            host: a.host.clone(),
            id: number(m, "/base/repo/id")?,
            owner: owner.into(),
            name: name.into(),
        },
        account,
        pr: a.number,
        target_tip: oid(m, "/base/sha")?,
        comparison_base,
        head: oid(m, "/head/sha")?,
        open: m["state"] == "open",
        draft: m["draft"] == true,
        pending_review: false,
    })
}
fn thread(v: &Value) -> Result<ThreadComment> {
    let id = number(v, "/id")?;
    Ok(ThreadComment {
        id,
        root_id: v["in_reply_to_id"].as_u64().unwrap_or(id),
        path: text(v, "/path")?.into(),
        side: match v["side"].as_str() {
            Some("LEFT") => Some(Side::Left),
            Some("RIGHT") => Some(Side::Right),
            _ => None,
        },
        line: v["line"].as_u64().and_then(|n| n.try_into().ok()),
        start_line: v["start_line"].as_u64().and_then(|n| n.try_into().ok()),
        body: text(v, "/body")?.into(),
        author: text(v, "/user/login")?.into(),
        commit_id: text(v, "/commit_id")?.into(),
        created_at: v["created_at"].as_str().map(Into::into),
    })
}

pub enum SendOutcome {
    Accepted(Value),
    Rejected(u16),
    Unknown(String),
}
/// Distinct from the read-only GithubReader on purpose; the application builds one only after a write opt-in.
pub struct GithubWriter {
    reader: Arc<GithubReader>,
}
impl GithubWriter {
    pub fn new(reader: Arc<GithubReader>) -> Self {
        Self { reader }
    }
}
pub trait ReviewRemote: Send + Sync {
    fn current(&self, t: &RemoteTarget) -> Result<RemoteTarget>;
    fn reviews(&self, t: &RemoteTarget) -> Result<Vec<Value>>;
    fn comments(&self, t: &RemoteTarget, id: u64) -> Result<Vec<Value>>;
    fn send(&self, p: &PreparedReview) -> SendOutcome;
}
impl ReviewRemote for GithubWriter {
    fn current(&self, t: &RemoteTarget) -> Result<RemoteTarget> {
        self.reader.current_target(t, Cancellation::default())
    }
    fn reviews(&self, t: &RemoteTarget) -> Result<Vec<Value>> {
        self.reader.reviews(t, Cancellation::default())
    }
    fn comments(&self, t: &RemoteTarget, id: u64) -> Result<Vec<Value>> {
        self.reader.review_comments(t, id, Cancellation::default())
    }
    fn send(&self, p: &PreparedReview) -> SendOutcome {
        if !p.verify() || p.target.provider != ProviderKind::GitHub {
            return SendOutcome::Rejected(422);
        }
        let a = PrAddress::from_target(&p.target);
        match self.reader.request(
            &a.host,
            &format!("{}/reviews", a.root()),
            "POST",
            Some(&p.payload()),
            "Accept: application/vnd.github+json",
            Cancellation::default(),
        ) {
            Ok(r) if r.status == 200 || r.status == 201 => match serde_json::from_slice(&r.body) {
                Ok(v) => SendOutcome::Accepted(v),
                Err(_) => SendOutcome::Unknown(
                    "the server reported success but the response body could not be read".into(),
                ),
            },
            Ok(r) if (400..500).contains(&r.status) && r.status != 408 => {
                SendOutcome::Rejected(r.status)
            }
            Ok(r) => SendOutcome::Unknown(format!(
                "the write got HTTP {}; retrying it automatically is not safe",
                r.status
            )),
            Err(e) => SendOutcome::Unknown(e.to_string()),
        }
    }
}

/// GitHub rejects the unified diff once a pull request passes 300 files (HTTP 406).
/// Rebuild an equal patch using the per-file API, that keeps hunk text for every
/// file aside from the ones GitHub marks too large; those stay header-only.
pub fn patch_from_files(files: &[Value]) -> Vec<u8> {
    let mut out = Vec::new();
    for f in files {
        let Some(name) = f["filename"].as_str() else {
            continue;
        };
        let prev = f["previous_filename"].as_str().unwrap_or(name);
        let status = f["status"].as_str().unwrap_or("modified");
        let changes = f["changes"].as_u64().unwrap_or(0);
        let patch = f["patch"].as_str();
        out.extend_from_slice(
            format!(
                "diff --git {} {}\n",
                quote_path("a/", prev),
                quote_path("b/", name)
            )
            .as_bytes(),
        );
        match status {
            "added" => out.extend_from_slice(b"new file mode 100644\n"),
            "removed" => out.extend_from_slice(b"deleted file mode 100644\n"),
            "renamed" | "copied" if prev != name => {
                let verb = if status == "copied" { "copy" } else { "rename" };
                out.extend_from_slice(
                    format!(
                        "{verb} from {}\n{verb} to {}\n",
                        quote_path("", prev),
                        quote_path("", name)
                    )
                    .as_bytes(),
                );
            }
            _ => {}
        }
        if patch.is_none() && changes > 0 {
            out.extend_from_slice(
                format!("{changes}-line change; GitHub omitted its text\n").as_bytes(),
            );
        }
        if patch.is_some() || changes > 0 {
            let old = if status == "added" {
                "/dev/null".to_string()
            } else {
                quote_path("a/", prev)
            };
            let new = if status == "removed" {
                "/dev/null".to_string()
            } else {
                quote_path("b/", name)
            };
            out.extend_from_slice(format!("--- {old}\n+++ {new}\n").as_bytes());
        }
        if let Some(p) = patch {
            out.extend_from_slice(p.as_bytes());
            if !p.ends_with('\n') {
                out.push(b'\n');
            }
        }
    }
    out
}
/// The quoting Git uses in C style, for paths holding quotes, backslashes, control bytes, or bytes outside ASCII.
fn quote_path(prefix: &str, path: &str) -> String {
    let plain = path
        .bytes()
        .all(|b| (0x20..0x7f).contains(&b) && b != b'"' && b != b'\\');
    if plain {
        return format!("{prefix}{path}");
    }
    let mut s = format!("\"{prefix}");
    for b in path.bytes() {
        match b {
            b'"' => s.push_str("\\\""),
            b'\\' => s.push_str("\\\\"),
            b'\t' => s.push_str("\\t"),
            b'\n' => s.push_str("\\n"),
            0x20..=0x7e => s.push(b as char),
            _ => s.push_str(&format!("\\{b:03o}")),
        }
    }
    s.push('"');
    s
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn patch_rebuilt_from_file_api_parses_like_a_unified_diff() {
        use diffz_core::patch::ChangeKind;
        let files = vec![
            serde_json::json!({"filename":"src/new.rs","status":"added","changes":2,"patch":"@@ -0,0 +1,2 @@\n+a\n+b"}),
            serde_json::json!({"filename":"dir with space/new name.txt","previous_filename":"dir with space/older.txt","status":"renamed","changes":1,"patch":"@@ -1 +1 @@\n-x\n+y"}),
            serde_json::json!({"filename":"gone.md","status":"removed","changes":1,"patch":"@@ -1 +0,0 @@\n-bye"}),
            serde_json::json!({"filename":"vendor/huge.lock","status":"modified","changes":4035}),
            serde_json::json!({"filename":"empty","status":"added","changes":0}),
            serde_json::json!({"filename":"docs/naïve \"q\".md","status":"modified","changes":1,"patch":"@@ -1 +1 @@\n-x\n+y"}),
        ];
        let report = parse_patch(&patch_from_files(&files), ParseLimits::default()).unwrap();
        assert_eq!(report.files.len(), 6);
        assert_eq!(report.files[0].kind, ChangeKind::Added);
        assert_eq!(report.files[0].additions(), 2);
        assert_eq!(report.files[1].kind, ChangeKind::Renamed);
        assert_eq!(
            report.files[1].display_path(),
            "dir with space/new name.txt"
        );
        assert_eq!(
            report.files[1].old_path.as_ref().unwrap().utf8().unwrap(),
            "dir with space/older.txt"
        );
        assert_eq!(report.files[2].kind, ChangeKind::Deleted);
        assert!(report.files[3].hunks.is_empty());
        assert!(
            report.files[3]
                .metadata
                .iter()
                .any(|m| m.contains("omitted"))
        );
        assert!(report.files[4].hunks.is_empty());
        assert_eq!(report.files[5].display_path(), "docs/naïve \"q\".md");
        assert_eq!(report.files[5].additions(), 1);
    }
    #[test]
    fn target_marks_draft_and_ready_prs() {
        fn address() -> PrAddress {
            PrAddress {
                host: "github.com".into(),
                owner: "o".into(),
                repo: "r".into(),
                number: 1,
            }
        }
        fn meta(draft: bool) -> serde_json::Value {
            serde_json::json!({
                "base": {
                    "repo": {"owner": {"login": "o"}, "name": "r", "id": 1},
                    "sha": "a".repeat(40),
                },
                "head": {"sha": "b".repeat(40)},
                "state": "open",
                "draft": draft,
            })
        }
        let pledge = "c".repeat(40);
        let draft_pr = target(&address(), &meta(true), "me".into(), pledge.clone()).unwrap();
        assert!(draft_pr.draft);
        let ready_pr = target(&address(), &meta(false), "me".into(), pledge).unwrap();
        assert!(!ready_pr.draft);
    }
}
