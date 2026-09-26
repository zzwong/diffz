//! glab transport for a chosen host. Reads have limits; uncertain writes stay one-shot.
use crate::{
    Result,
    github::{HttpResponse, decode_http},
    process::{ProcessRequest, Runner},
    provider::{ReviewProvider, ReviewRemote, SendOutcome},
};
use diffz_core::{
    domain::*,
    patch::{FileChange, ParseLimits, parse_patch},
    provider::{Cancellation, ReviewRules},
    review::{PreparedComment, PreparedReview, ReviewError, Verdict},
    review_details::*,
    source_link::encode,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json, value::RawValue};
use std::{path::PathBuf, sync::Arc};
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MrAddress {
    pub host: String,
    pub project: String,
    pub number: u64,
}
impl MrAddress {
    pub fn parse(input: &str) -> Result<Self> {
        let input = input.trim();
        let (host, project, number) = if input.starts_with("https://") {
            if input.split('/').any(|s| matches!(s, "." | "..")) {
                return Err("URL paths may not contain relative segments".into());
            }
            let u = url::Url::parse(input).map_err(|_| "Invalid merge request URL")?;
            if !u.username().is_empty()
                || u.password().is_some()
                || u.port().is_some()
                || u.query().is_some()
            {
                return Err("HTTPS MR URLs cannot include credentials, ports, or queries".into());
            }
            let (project, tail) = u
                .path()
                .trim_matches('/')
                .split_once("/-/merge_requests/")
                .ok_or("Expected /group/project/-/merge_requests/number")?;
            let mut parts = tail.split('/');
            let n = parts.next().unwrap_or_default();
            if parts
                .next()
                .is_some_and(|p| !matches!(p, "diffs" | "commits" | "pipelines"))
                || parts.next().is_some()
            {
                return Err("Invalid MR URL suffix".into());
            }
            (
                u.host_str().ok_or("Missing GitLab host")?.to_string(),
                project.to_string(),
                n.to_string(),
            )
        } else {
            let (project, n) = input
                .rsplit_once('!')
                .ok_or("Enter group/project!123, or provide an HTTPS URL for the GitLab MR")?;
            ("gitlab.com".into(), project.into(), n.into())
        };
        if project.split('/').count() < 2
            || project.split('/').any(|p| {
                p.is_empty()
                    || matches!(p, "." | "..")
                    || !p
                        .bytes()
                        .all(|c| c.is_ascii_alphanumeric() || b"-_.".contains(&c))
            })
            || !number.bytes().all(|b| b.is_ascii_digit())
        {
            return Err("GitLab project name or merge request number is invalid".into());
        }
        let number = number.parse::<u64>().map_err(|_| "Invalid MR number")?;
        if number == 0 {
            return Err("MR number must be positive".into());
        }
        Ok(Self {
            host,
            project,
            number,
        })
    }
    fn root(&self) -> String {
        format!(
            "projects/{}/merge_requests/{}",
            self.project.replace('/', "%2F"),
            self.number
        )
    }
    fn from_target(t: &RemoteTarget) -> Self {
        Self {
            host: t.repository.host.clone(),
            project: format!("{}/{}", t.repository.owner, t.repository.name),
            number: t.pr,
        }
    }
}
pub struct GitlabReader {
    executable: PathBuf,
}
impl GitlabReader {
    pub fn new(executable: PathBuf) -> Self {
        Self { executable }
    }
    fn request(
        &self,
        a: &MrAddress,
        path: &str,
        method: &str,
        body: Option<&Value>,
        cancel: Cancellation,
    ) -> Result<HttpResponse> {
        let mut req = ProcessRequest::new(self.executable.clone()).args([
            "api",
            "--hostname",
            &a.host,
            "--method",
            method,
            "--include",
            path,
        ]);
        req.cwd = Some(std::env::temp_dir());
        if let Some(body) = body {
            req.args.extend(["--input".into(), "-".into()]);
            req.stdin = serde_json::to_vec(body)?;
        }
        let out = Runner::run(req, cancel)?;
        decode_http(&out.stdout).map_err(|_| crate::github::incomplete_http("glab", &out))
    }
    pub fn source(&self, t: &RemoteTarget, path: &str, revision: &str) -> Result<Vec<u8>> {
        let a = MrAddress::from_target(t);
        let path = format!(
            "projects/{}/repository/files/{}/raw?ref={}",
            crate::github::encode_path(&a.project).replace('/', "%2F"),
            crate::github::encode_path(path).replace('/', "%2F"),
            crate::github::encode_path(revision)
        );
        let r = self.request(&a, &path, "GET", None, Cancellation::default())?;
        if r.status != 200 {
            return Err(format!("Source read returned HTTP {}", r.status).into());
        }
        Ok(r.body)
    }
    fn get(&self, a: &MrAddress, path: &str, c: Cancellation) -> Result<Value> {
        let r = self.request(a, path, "GET", None, c)?;
        if r.status != 200 {
            return Err(format!("GitLab read returned HTTP {}", r.status).into());
        }
        Ok(serde_json::from_slice(&r.body)?)
    }
    fn pages(&self, a: &MrAddress, path: &str, c: Cancellation) -> Result<Vec<Value>> {
        let mut rows = vec![];
        let mut bytes = 0;
        for page in 1..=30 {
            let sep = if path.contains('?') { '&' } else { '?' };
            let r = self.request(
                a,
                &format!("{path}{sep}per_page=100&page={page}"),
                "GET",
                None,
                c.clone(),
            )?;
            if r.status != 200 {
                return Err(format!("GitLab list returned HTTP {}", r.status).into());
            }
            bytes += r.body.len();
            if bytes > 16 * 1024 * 1024 {
                return Err("GitLab collection is over the 16 MiB cap".into());
            }
            let items: Vec<Value> = serde_json::from_slice(&r.body)?;
            let n = items.len();
            rows.extend(items);
            if n < 100 {
                return Ok(rows);
            }
        }
        Err("GitLab collection is over the 3000-item cap".into())
    }
    fn target(&self, a: &MrAddress, c: Cancellation) -> Result<RemoteTarget> {
        let m = self.get(a, &a.root(), c.clone())?;
        let user = self.get(a, "user", c)?;
        target(a, &m, &user)
    }
    pub fn snapshot(&self, a: &MrAddress, c: Cancellation) -> Result<Snapshot> {
        let m = self.get(a, &a.root(), c.clone())?;
        let user = self.get(a, "user", c.clone())?;
        let t = target(a, &m, &user)?;
        let response = self.request(
            a,
            &format!("{}/raw_diffs", a.root()),
            "GET",
            None,
            c.clone(),
        )?;
        if response.status != 200 {
            return Err(format!("GitLab diff returned HTTP {}", response.status).into());
        }
        let patch = parse_patch(&response.body, ParseLimits::default())?;
        drop(response);
        let files = self.pages(a, &format!("{}/diffs", a.root()), c.clone())?;
        let paths: std::collections::BTreeSet<_> =
            patch.files.iter().map(|f| f.display_path()).collect();
        let listed: std::collections::BTreeSet<_> = files
            .iter()
            .map(|f| {
                let new = f["new_path"].as_str().unwrap_or("");
                if paths.contains(new) {
                    new
                } else {
                    f["old_path"].as_str().unwrap_or("")
                }
            })
            .collect();
        if files.len() != patch.files.len()
            || listed.len() != paths.len()
            || listed.iter().any(|p| !paths.contains(*p))
            || files
                .iter()
                .any(|f| f["too_large"] == true || f["collapsed"] == true)
        {
            return Err("GitLab did not provide every diff file; review was refused".into());
        }
        drop(files);
        let discussions = self.pages(a, &format!("{}/discussions", a.root()), c.clone())?;
        // The MR payload never carries a "changes requested" ruling; approvals are all GitLab reports.
        let approved = self
            .get(a, &format!("{}/approvals", a.root()), c.clone())
            .ok()
            .and_then(|v| Some(!v["approved_by"].as_array()?.is_empty()))
            .unwrap_or(false);
        let mut overview = Overview {
            description: m["description"].as_str().map(str::to_owned),
            author: m["author"]["username"].as_str().map(str::to_owned),
            decision: approved.then_some(ReviewDecision::Approved),
            captured_at: Some(
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map_err(|_| "Clock error")?
                    .as_secs(),
            ),
            ..Default::default()
        };
        let mut comments = discussion_comments(&discussions, &mut overview);
        drop(discussions);
        for comment in &mut comments {
            if comment.commit_id != t.head {
                comment.line = None;
                comment.start_line = None;
            }
        }
        match self.pages(a, &format!("{}/pipelines", a.root()), c.clone()) {
            Ok(pipelines) => {
                if pipelines
                    .iter()
                    .filter(|p| p["sha"].as_str() == Some(t.head.as_str()))
                    .count()
                    > 10
                {
                    overview
                        .notices
                        .push("Only the newest 10 pipelines for this revision are shown.".into());
                }
                for p in pipelines
                    .iter()
                    .filter(|p| p["sha"].as_str() == Some(t.head.as_str()))
                    .take(10)
                {
                    if overview.checks.len() >= 500 {
                        break;
                    }
                    overview.checks.push(check(p, "Pipeline"));
                    if let Some(id) = p["id"].as_u64() {
                        match self.pages(
                            a,
                            &format!("projects/{}/pipelines/{id}/jobs", t.repository.id),
                            c.clone(),
                        ) {
                            Ok(jobs) => {
                                overview.checks.extend(jobs.iter().map(|j| check(j, "Job")))
                            }
                            Err(e) => overview.notices.push(e.to_string()),
                        }
                    }
                }
            }
            Err(e) => overview.notices.push(e.to_string()),
        }
        if self.target(a, c)? != t {
            return Err(
                "The merge request or account changed during loading; open it again".into(),
            );
        }
        let mut s = Snapshot::with_origin(
            format!(
                "{} · {} !{}",
                m["title"].as_str().unwrap_or("Merge request"),
                a.project,
                a.number
            ),
            patch,
            Some(t),
            comments,
            format!("gitlab:{}:{}!{}", a.host, a.project, a.number),
        );
        s.overview = overview;
        Ok(s)
    }
}
fn string(v: &Value, key: &str) -> Result<String> {
    v.pointer(key)
        .and_then(Value::as_str)
        .map(str::to_owned)
        .ok_or_else(|| format!("GitLab response missing {key}").into())
}
fn target(a: &MrAddress, m: &Value, user: &Value) -> Result<RemoteTarget> {
    let (owner, name) = a.project.rsplit_once('/').ok_or("Invalid project")?;
    let t = RemoteTarget {
        provider: ProviderId::GITLAB,
        repository: RepositoryKey {
            host: a.host.clone(),
            id: m["project_id"].as_u64().ok_or("Missing project ID")?,
            owner: owner.into(),
            name: name.into(),
        },
        account: string(user, "/username")?,
        pr: a.number,
        target_tip: string(m, "/diff_refs/start_sha")?,
        comparison_base: string(m, "/diff_refs/base_sha")?,
        head: string(m, "/diff_refs/head_sha")?,
        open: m["state"] == "opened",
        draft: m["draft"] == true || m["work_in_progress"] == true,
        pending_review: false,
    };
    for sha in [&t.head, &t.target_tip, &t.comparison_base] {
        if sha.len() != 40 || !sha.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Err("GitLab has not produced usable diff references yet".into());
        }
    }
    if m["iid"].as_u64() != Some(a.number) {
        return Err("GitLab MR identity mismatch".into());
    }
    Ok(t)
}
pub fn check(v: &Value, kind: &str) -> Check {
    let status = v["status"].as_str().unwrap_or("unknown");
    let conclusion = match status {
        "success" => Some("success"),
        "failed" => Some("failure"),
        "canceled" => Some("cancelled"),
        "skipped" => Some("skipped"),
        _ => None,
    };
    Check {
        name: v["name"]
            .as_str()
            .map(str::to_owned)
            .unwrap_or_else(|| format!("Pipeline #{}", v["id"])),
        kind: kind.into(),
        status: if conclusion.is_some() {
            "completed".into()
        } else {
            status.into()
        },
        conclusion: conclusion.map(str::to_owned),
        url: v["web_url"]
            .as_str()
            .filter(|s| s.starts_with("https://"))
            .map(str::to_owned),
    }
}
pub fn discussion_comments(ds: &[Value], overview: &mut Overview) -> Vec<ThreadComment> {
    let mut out = vec![];
    for d in ds {
        let Some(notes) = d["notes"].as_array() else {
            continue;
        };
        let root = notes.iter().find(|n| n["system"] != true);
        let Some(root) = root else { continue };
        let pos = &root["position"];
        for n in notes.iter().filter(|n| n["system"] != true) {
            let Some(id) = n["id"].as_u64() else { continue };
            let body = n["body"].as_str().unwrap_or("").to_string();
            let author = n["author"]["username"]
                .as_str()
                .unwrap_or("unknown")
                .to_string();
            if pos["position_type"] == "text" {
                let right = pos["new_line"].as_u64().is_some();
                let key = if right { "new" } else { "old" };
                let line = pos[format!("{key}_line")]
                    .as_u64()
                    .and_then(|n| n.try_into().ok());
                out.push(ThreadComment {
                    id,
                    root_id: root["id"].as_u64().unwrap_or(id),
                    path: pos[format!("{key}_path")].as_str().unwrap_or("").into(),
                    side: Some(if right { Side::Right } else { Side::Left }),
                    line,
                    start_line: pos["line_range"]["start"][format!("{key}_line")]
                        .as_u64()
                        .and_then(|n| n.try_into().ok()),
                    body,
                    author,
                    commit_id: pos["head_sha"].as_str().unwrap_or("").into(),
                    created_at: n["created_at"].as_str().map(Into::into),
                });
            } else {
                overview.conversation.push(ConversationComment {
                    id,
                    body,
                    author,
                    created_at: n["created_at"].as_str().map(Into::into),
                });
            }
        }
    }
    out
}

pub struct GitlabRules;

/// Field order is part of the review fingerprint.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GitlabPosition {
    pub position_type: String,
    pub base_sha: String,
    pub start_sha: String,
    pub head_sha: String,
    pub old_path: String,
    pub new_path: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub old_line: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub new_line: Option<u32>,
}

// Fingerprinted: keep this field order.
#[derive(Serialize)]
struct Payload<'a> {
    head: &'a str,
    verdict: Verdict,
    summary: &'a str,
    comments: &'a [PreparedComment],
}

impl ReviewRules for GitlabRules {
    fn id(&self) -> ProviderId {
        ProviderId::GITLAB
    }
    fn name(&self) -> &str {
        "GitLab"
    }
    fn open_label(&self) -> &str {
        "GitLab MR"
    }
    fn address_label(&self) -> &str {
        "Merge request"
    }
    fn address_hint(&self) -> &str {
        "Enter group/project!123 or a GitLab merge request URL"
    }
    fn address_help(&self) -> &str {
        "Uses glab with GitLab.com or a self-managed HTTPS server."
    }
    fn write_flag(&self) -> &str {
        "--allow-gitlab-writes"
    }
    fn preview_note(&self) -> Option<&str> {
        Some("GitLab accepts comments and approval. Choose one source line for each inline draft.")
    }
    fn reopen_address(&self, t: &RemoteTarget) -> String {
        let r = &t.repository;
        format!(
            "https://{}/{}/{}/-/merge_requests/{}",
            r.host, r.owner, r.name, t.pr
        )
    }
    fn line_url(&self, t: &RemoteTarget, path: &str, revision: &str, line: u32) -> String {
        let r = &t.repository;
        format!(
            "https://{}/{}/{}/-/blob/{}/{}#L{line}",
            r.host,
            encode(&r.owner),
            encode(&r.name),
            encode(revision),
            encode(path)
        )
    }
    fn supports(&self, verdict: Verdict) -> bool {
        verdict != Verdict::RequestChanges
    }
    fn check(
        &self,
        verdict: Verdict,
        summary: &str,
        drafts: &[Draft],
    ) -> std::result::Result<(), ReviewError> {
        if !self.supports(verdict) {
            return Err(ReviewError(
                "GitLab permits comments and approval; blocking change requests remain unsupported"
                    .into(),
            ));
        }
        let quick_action = |text: &str| text.lines().any(|l| l.trim_start().starts_with('/'));
        if quick_action(summary) || drafts.iter().any(|d| quick_action(&d.body)) {
            return Err(ReviewError(
                "GitLab quick actions are disallowed here; format inline slash-prefixed text before posting"
                    .into(),
            ));
        }
        Ok(())
    }
    fn position(
        &self,
        target: &RemoteTarget,
        f: &FileChange,
        d: &Draft,
    ) -> std::result::Result<Option<Box<RawValue>>, ReviewError> {
        let fail = |m: &str| ReviewError(m.into());
        let old = f
            .old_path
            .as_ref()
            .unwrap_or(f.path())
            .utf8()
            .map_err(ReviewError)?;
        let new = f
            .new_path
            .as_ref()
            .unwrap_or(f.path())
            .utf8()
            .map_err(ReviewError)?;
        let mut position = GitlabPosition {
            position_type: "file".into(),
            base_sha: target.comparison_base.clone(),
            start_sha: target.target_tip.clone(),
            head_sha: target.head.clone(),
            old_path: old.into(),
            new_path: new.into(),
            old_line: None,
            new_line: None,
        };
        if !d.is_file_level() {
            if d.start_line != d.line {
                return Err(fail(
                    "GitLab posting needs each draft to hold one source line for now",
                ));
            }
            let row = f
                .line(d.side, d.line)
                .ok_or_else(|| fail("the draft holds no source line to post to GitLab"))?;
            position.position_type = "text".into();
            position.old_line = row.old_line;
            position.new_line = row.new_line;
        }
        serde_json::value::to_raw_value(&position)
            .map(Some)
            .map_err(|e| ReviewError(e.to_string()))
    }
    fn payload(&self, p: &PreparedReview) -> Box<RawValue> {
        serde_json::value::to_raw_value(&Payload {
            head: &p.target.head,
            verdict: p.verdict,
            summary: &p.summary,
            comments: &p.comments,
        })
        .expect("plain serializable review payload")
    }
    fn marks_reviews(&self) -> bool {
        true
    }
}

pub struct GitlabProvider {
    reader: Option<Arc<GitlabReader>>,
}
impl GitlabProvider {
    pub fn new(reader: Option<Arc<GitlabReader>>) -> Self {
        Self { reader }
    }
    fn reader(&self) -> Result<&Arc<GitlabReader>> {
        self.reader
            .as_ref()
            .ok_or_else(|| "Install glab, then authenticate for this GitLab host".into())
    }
}
impl ReviewProvider for GitlabProvider {
    fn rules(&self) -> Arc<dyn ReviewRules> {
        Arc::new(GitlabRules)
    }
    fn open(&self, address: &str, cancel: Cancellation) -> Result<Snapshot> {
        self.reader()?.snapshot(&MrAddress::parse(address)?, cancel)
    }
    fn source(&self, t: &RemoteTarget, path: &str, revision: &str) -> Result<Vec<u8>> {
        self.reader()?.source(t, path, revision)
    }
    fn remote(&self) -> Result<Arc<dyn ReviewRemote>> {
        Ok(Arc::new(GitlabWriter::new(self.reader()?.clone())))
    }
}

pub struct GitlabWriter {
    reader: Arc<GitlabReader>,
}
impl GitlabWriter {
    pub fn new(reader: Arc<GitlabReader>) -> Self {
        Self { reader }
    }
}
fn marker(p: &PreparedReview) -> String {
    format!(
        "\n\n<!-- diffz:{}:{}:{} -->",
        p.fingerprint,
        p.target.head,
        p.verdict.remote_state()
    )
}
fn marked_note(n: &Value) -> Option<Value> {
    let body = n["body"].as_str()?;
    let (body, mark) = body.rsplit_once("\n\n<!-- diffz:")?;
    let fields: Vec<_> = mark.strip_suffix(" -->")?.split(':').collect();
    if !(3..=4).contains(&fields.len()) {
        return None;
    }
    Some(
        json!({"id":n["id"],"body":body,"commit_id":fields[1],"state":fields[2],"fingerprint":fields[0],"side":fields.get(3),"user":{"login":n["author"]["username"]}}),
    )
}
/// Find file-level notes that diffz wrote within the MR's ordinary notes. These notes carry the
/// review marker and a body shaped like `"**<path>**\n\n<body>"`, that the diffz
/// writer relies on to tell them apart from the note that holds the summary.
fn file_level_note(n: &Value, fingerprint: &str) -> Option<Value> {
    let marked = marked_note(n)?;
    if marked["fingerprint"].as_str() != Some(fingerprint) {
        return None;
    }
    let body = marked["body"].as_str()?;
    let (path, rest) = body.strip_prefix("**")?.split_once("**\n\n")?;
    if path.is_empty() || rest.is_empty() {
        return None;
    }
    Some(json!({"body":rest,"path":path,"line":0,"side":"RIGHT"}))
}
impl ReviewRemote for GitlabWriter {
    fn current(&self, t: &RemoteTarget) -> Result<RemoteTarget> {
        self.reader
            .target(&MrAddress::from_target(t), Cancellation::default())
    }
    fn reviews(&self, t: &RemoteTarget) -> Result<Vec<Value>> {
        let a = MrAddress::from_target(t);
        let notes =
            self.reader
                .pages(&a, &format!("{}/notes", a.root()), Cancellation::default())?;
        Ok(notes
            .iter()
            .filter(|n| n["system"] != true && n["type"] != "DiffNote")
            .filter_map(marked_note)
            .collect())
    }
    fn comments(&self, t: &RemoteTarget, id: u64) -> Result<Vec<Value>> {
        let a = MrAddress::from_target(t);
        let note = self.reader.get(
            &a,
            &format!("{}/notes/{id}", a.root()),
            Cancellation::default(),
        )?;
        let review = marked_note(&note).ok_or("Review marker unavailable")?;
        let fingerprint = review["fingerprint"]
            .as_str()
            .ok_or("Missing fingerprint")?;
        if review["state"] == "APPROVED" {
            let approved = self.reader.get(
                &a,
                &format!("{}/approvals", a.root()),
                Cancellation::default(),
            )?;
            if !approved["approved_by"]
                .as_array()
                .is_some_and(|users| users.iter().any(|u| u["user"]["username"] == t.account))
            {
                return Err("Approval could not be verified".into());
            }
        }
        let ds = self.reader.pages(
            &a,
            &format!("{}/discussions", a.root()),
            Cancellation::default(),
        )?;
        let mut result = vec![];
        for d in ds {
            if let Some(notes) = d["notes"].as_array() {
                for n in notes {
                    if n["position"]["position_type"] != "text" {
                        continue;
                    }
                    let Some(marked) = marked_note(n) else {
                        continue;
                    };
                    if marked["fingerprint"] != fingerprint {
                        continue;
                    }
                    let pos = &n["position"];
                    if pos["head_sha"] != t.head
                        || pos["base_sha"] != t.comparison_base
                        || pos["start_sha"] != t.target_tip
                        || n["author"]["username"] != t.account
                    {
                        return Err("Discussion identity mismatch".into());
                    }
                    let right = marked["side"]
                        .as_str()
                        .map_or(pos["new_line"].as_u64().is_some(), |s| s == "RIGHT");
                    let key = if right { "new" } else { "old" };
                    result.push(json!({"body":marked["body"],"path":pos[format!("{key}_path")],"line":pos[format!("{key}_line")],"side":if right{"RIGHT"}else{"LEFT"}}));
                }
            }
        }
        let notes =
            self.reader
                .pages(&a, &format!("{}/notes", a.root()), Cancellation::default())?;
        for n in notes
            .iter()
            .filter(|n| n["system"] != true && n["type"] != "DiffNote")
        {
            if let Some(file_level) = file_level_note(n, fingerprint) {
                result.push(file_level);
            }
        }
        Ok(result)
    }
    fn send(&self, p: &PreparedReview) -> SendOutcome {
        if !p.verify(&GitlabRules) || p.verdict == Verdict::RequestChanges {
            return SendOutcome::Rejected(422);
        }
        let a = MrAddress::from_target(&p.target);
        let tag = marker(p);
        let mut sent = false;
        let result = (|| -> Result<Value> {
            for comment in &p.comments {
                if self.current(&p.target)? != p.target {
                    return Err("MR changed during publication".into());
                }
                let (endpoint, payload) = if comment.file_level {
                    (
                        format!("{}/notes", a.root()),
                        json!({"body": format!("**{}**\n\n{}{}", comment.path, comment.body, tag)}),
                    )
                } else {
                    let pos = comment
                        .position
                        .as_ref()
                        .ok_or("Frozen GitLab position is absent")?;
                    (
                        format!("{}/discussions", a.root()),
                        json!({"body":format!("{}{}:{} -->",comment.body,tag.trim_end_matches(" -->"),comment.side.api()),"position":pos}),
                    )
                };
                sent = true;
                let r = self.reader.request(
                    &a,
                    &endpoint,
                    "POST",
                    Some(&payload),
                    Cancellation::default(),
                )?;
                if r.status != 201 {
                    return Err(
                        format!("GitLab rejected the comment with HTTP {}", r.status).into(),
                    );
                }
            }
            if p.verdict == Verdict::Approve {
                if self.current(&p.target)? != p.target {
                    return Err("MR changed before approval".into());
                }
                sent = true;
                let r = self.reader.request(
                    &a,
                    &format!("{}/approve", a.root()),
                    "POST",
                    Some(&json!({"sha":p.target.head})),
                    Cancellation::default(),
                )?;
                if r.status != 201 && r.status != 200 {
                    return Err("GitLab approval did not confirm".into());
                }
            }
            if self.current(&p.target)? != p.target {
                return Err("MR changed before review summary".into());
            }
            sent = true;
            let r = self.reader.request(
                &a,
                &format!("{}/notes", a.root()),
                "POST",
                Some(&json!({"body":format!("{}{}",p.summary,tag)})),
                Cancellation::default(),
            )?;
            if r.status != 201 {
                return Err(format!("GitLab summary write produced HTTP {}", r.status).into());
            }
            let n: Value = serde_json::from_slice(&r.body)?;
            marked_note(&n).ok_or_else(|| "Unverifiable GitLab summary".into())
        })();
        match result {
            Ok(v) => SendOutcome::Accepted(v),
            Err(e) if sent => SendOutcome::Unknown(format!(
                "{e}. Comments may exist remotely; inspect and reconcile before any resend."
            )),
            Err(_) => SendOutcome::Rejected(422),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn target_marks_draft_and_work_in_progress_mrs() {
        fn address() -> MrAddress {
            MrAddress {
                host: "git.example.com".into(),
                project: "o/r".into(),
                number: 7,
            }
        }
        fn meta(draft: bool, wip: bool) -> serde_json::Value {
            serde_json::json!({
                "id": 1,
                "iid": 7,
                "project_id": 11,
                "state": "opened",
                "draft": draft,
                "work_in_progress": wip,
                "diff_refs": {
                    "start_sha": "a".repeat(40),
                    "base_sha": "b".repeat(40),
                    "head_sha": "c".repeat(40),
                },
            })
        }
        let user = serde_json::json!({ "username": "alice" });
        let draft_mr = target(&address(), &meta(true, false), &user).unwrap();
        assert!(draft_mr.draft);
        let wip_mr = target(&address(), &meta(false, true), &user).unwrap();
        assert!(wip_mr.draft);
        let ready_mr = target(&address(), &meta(false, false), &user).unwrap();
        assert!(!ready_mr.draft);
    }
}
