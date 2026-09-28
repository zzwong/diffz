//! glab transport for a chosen host. Reads have limits; uncertain writes stay one-shot.
use crate::{
    Result,
    github::{HttpResponse, decode_http, decode_path, quote_path, ref_name},
    process::{ProcessRequest, Runner},
    provider::{Blame, ReviewProvider, ReviewRemote, SendOutcome, Span},
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
/// What a GitLab address names: a merge request, or a read-only compare of two refs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GitlabTarget {
    Mr(MrAddress),
    Compare(GitlabCompare),
}
impl GitlabTarget {
    pub fn parse(input: &str) -> Result<Self> {
        let input = input.trim();
        let compare = input.starts_with("https://")
            && url::Url::parse(input).is_ok_and(|u| {
                let path = u.path().trim_end_matches('/');
                path.ends_with("/-/compare") || path.contains("/-/compare/")
            });
        if compare {
            GitlabCompare::parse(input).map(Self::Compare)
        } else {
            MrAddress::parse(input).map(Self::Mr)
        }
    }
}
/// `https://HOST/GROUP/PROJECT/-/compare/FROM...TO`, or the `?from=&to=&straight=` form.
/// `FROM..TO` and `straight=true` ask for a direct diff instead of one against the merge base.
/// GitLab's Compare button adds `from_project_id`; it is kept so a cross-project compare can be refused.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GitlabCompare {
    pub host: String,
    pub project: String,
    pub refs: CompareRefs,
    pub from_project_id: Option<u64>,
}
impl GitlabCompare {
    pub fn parse(input: &str) -> Result<Self> {
        let input = input.trim();
        if input.split('/').any(|s| matches!(s, "." | "..")) {
            return Err("URL paths may not contain relative segments".into());
        }
        let u = url::Url::parse(input).map_err(|_| "Invalid compare URL")?;
        if u.scheme() != "https"
            || !u.username().is_empty()
            || u.password().is_some()
            || u.port().is_some()
        {
            return Err("HTTPS compare URLs cannot include credentials or ports".into());
        }
        let path = u.path().trim_matches('/');
        let (project, tail) = path
            .split_once("/-/compare")
            .ok_or("Expected /group/project/-/compare/from...to")?;
        let tail = match tail {
            "" => None,
            t => Some(t.strip_prefix('/').ok_or("Invalid compare URL suffix")?),
        };
        let mut from_project_id = None;
        let mut straight = None;
        let mut project_id = |value: &str| -> Result<()> {
            from_project_id = Some(
                value
                    .parse()
                    .map_err(|_| "from_project_id must be a number")?,
            );
            Ok(())
        };
        let (from, to, direct) = match tail {
            Some(range) => {
                for (key, value) in u.query_pairs() {
                    match &*key {
                        "from_project_id" => project_id(&value)?,
                        "straight" => straight = Some(&*value == "true"),
                        _ => {
                            return Err(
                                "Give the refs in the path or in the query, not both".into()
                            );
                        }
                    }
                }
                let range = decode_path(range)?;
                let (from, to, direct) = match range.split_once("...") {
                    Some((from, to)) => (from, to, false),
                    None => {
                        let (from, to) = range
                            .split_once("..")
                            .ok_or("A compare needs two refs: from...to, or from..to")?;
                        (from, to, true)
                    }
                };
                if straight.is_some_and(|s| s != direct) {
                    return Err("straight disagrees with the dots in the compare path".into());
                }
                (from.to_owned(), to.to_owned(), direct)
            }
            None => {
                let (mut from, mut to, mut direct) = (None, None, false);
                for (key, value) in u.query_pairs() {
                    match &*key {
                        "from" => from = Some(value.into_owned()),
                        "to" => to = Some(value.into_owned()),
                        "from_project_id" => project_id(&value)?,
                        "straight" => {
                            direct = match &*value {
                                "true" => true,
                                "false" => false,
                                _ => return Err("straight must be true or false".into()),
                            }
                        }
                        _ => {
                            return Err(
                                "Only from, to, straight, and from_project_id are accepted".into(),
                            );
                        }
                    }
                }
                (
                    from.ok_or("The compare query needs from and to")?,
                    to.ok_or("The compare query needs from and to")?,
                    direct,
                )
            }
        };
        if project.split('/').count() < 2
            || project.split('/').any(|p| {
                p.is_empty()
                    || matches!(p, "." | "..")
                    || !p
                        .bytes()
                        .all(|c| c.is_ascii_alphanumeric() || b"-_.".contains(&c))
            })
        {
            return Err("GitLab project name is invalid".into());
        }
        if !ref_name(&from) || !ref_name(&to) {
            return Err("The compare names a ref that Git cannot use".into());
        }
        Ok(Self {
            host: u.host_str().ok_or("Missing GitLab host")?.to_string(),
            project: project.into(),
            refs: CompareRefs {
                base: from,
                head: to,
                direct,
            },
            from_project_id,
        })
    }
}
/// GitLab's default cap on the files one compare lists; an instance may raise it to 3000.
const DIFF_FILES: usize = 1000;
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
            req.args.extend([
                "--header".into(),
                "Content-Type: application/json".into(),
                "--input".into(),
                "-".into(),
            ]);
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
    /// A compare is read-only: no MR, discussions, or account, only two resolved commits.
    pub fn compare(&self, a: &GitlabCompare, c: Cancellation) -> Result<Snapshot> {
        // The transport only needs the host from an address; the number is never used.
        let h = MrAddress {
            host: a.host.clone(),
            project: a.project.clone(),
            number: 0,
        };
        let project = format!("projects/{}", a.project.replace('/', "%2F"));
        let commit = |r: &str| {
            format!(
                "{project}/repository/commits/{}",
                crate::github::encode_path(r).replace('/', "%2F")
            )
        };
        // Refs move, so resolve them once here and pin every later read to those commits.
        let (meta, from, to) = std::thread::scope(|s| {
            let meta = s.spawn(|| self.get(&h, &project, c.clone()));
            let from = s.spawn(|| self.get(&h, &commit(&a.refs.base), c.clone()));
            let to = self.get(&h, &commit(&a.refs.head), c.clone());
            (joined(meta), joined(from), to)
        });
        let (meta, from, to) = (meta?, from?, to?);
        let (from, to) = (sha(&from, "/id")?, sha(&to, "/id")?);
        let (compare, base) = std::thread::scope(|s| {
            let compare = s.spawn(|| {
                self.get(
                    &h,
                    &format!(
                        "{project}/repository/compare?from={from}&to={to}&straight={}",
                        a.refs.direct
                    ),
                    c.clone(),
                )
            });
            let base = if a.refs.direct {
                Ok(from.clone())
            } else {
                self.get(
                    &h,
                    &format!("{project}/repository/merge_base?refs[]={from}&refs[]={to}"),
                    c.clone(),
                )
                .and_then(|v| sha(&v, "/id"))
            };
            (joined(compare), base)
        });
        let (compare, base) = (compare?, base?);
        let diffs = compare["diffs"].as_array().map_or(&[][..], Vec::as_slice);
        let patch = parse_patch(&patch_from_diffs(diffs), ParseLimits::default())?;
        let id = meta["id"].as_u64().ok_or("Missing project ID")?;
        if a.from_project_id.is_some_and(|from| from != id) {
            return Err("cross-project compares are not supported".into());
        }
        let path = string(&meta, "/path_with_namespace")?;
        let (owner, name) = path.rsplit_once('/').ok_or("Invalid project")?;
        let remote = RemoteTarget {
            provider: ProviderId::GITLAB,
            repository: RepositoryKey {
                host: a.host.clone(),
                id,
                owner: owner.into(),
                name: name.into(),
            },
            account: String::new(),
            pr: 0,
            target_tip: from,
            comparison_base: base,
            head: to,
            open: true,
            draft: false,
            pending_review: false,
            compare: Some(a.refs.clone()),
        };
        let label = a.refs.label();
        let mut s = Snapshot::with_origin(
            format!("{path}  {label}"),
            patch,
            Some(remote),
            vec![],
            format!("gitlab-compare:{}:{path}:{label}", a.host),
        );
        if compare["compare_timeout"] == true || diffs.len() >= DIFF_FILES {
            s.warnings.push(
                "GitLab timed out computing this compare or reached its file limit, so its file list, release timeline, and release attribution may be incomplete"
                    .into(),
            );
        }
        if s.patch.files.len() != diffs.len() {
            s.warnings.push(format!(
                "coverage disagreement: file listing {}, patch count {}",
                diffs.len(),
                s.patch.files.len()
            ));
        }
        for d in diffs {
            if d["too_large"] == true || d["collapsed"] == true {
                s.warnings.push(format!(
                    "GitLab left out the text of {} because it is too large or collapsed",
                    d["new_path"].as_str().unwrap_or_default()
                ));
            }
        }
        s.overview.description = Some(
            compare["commits"]
                .as_array()
                .map_or(&[][..], Vec::as_slice)
                .iter()
                .map(|c| {
                    format!(
                        "- `{}` {}\n",
                        c["short_id"].as_str().unwrap_or_default(),
                        c["title"].as_str().unwrap_or_default()
                    )
                })
                .collect(),
        );
        s.overview.captured_at = Some(
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_err(|_| "Clock error")?
                .as_secs(),
        );
        Ok(s)
    }
    /// The release tags on a compare's range and what each changed since the one before.
    /// Read after the compare opens, since a long range takes many more requests.
    pub fn releases(
        &self,
        t: &RemoteTarget,
        c: Cancellation,
    ) -> Result<(Vec<diffz_core::review_details::Release>, Vec<String>)> {
        use crate::releases::{TAG_PAGES, bounded, folded_warning, members, steps};
        use diffz_core::review_details::{Release, ReleaseFile};
        let refs = t.compare.as_ref().ok_or("only a compare has releases")?;
        let path = format!("{}/{}", t.repository.owner, t.repository.name);
        let h = MrAddress {
            host: t.repository.host.clone(),
            project: path.clone(),
            number: 0,
        };
        let project = format!("projects/{}", path.replace('/', "%2F"));
        let (from, to) = (t.target_tip.as_str(), t.head.as_str());
        // Every step uses the compare's own mode. Later steps start at an ancestor of where they
        // end, where both modes agree; the first starts at FROM, like the compare itself.
        let straight = refs.direct;
        let mut warnings = vec![];
        let (compare, listed) = std::thread::scope(|s| {
            let compare = s.spawn(|| {
                self.get(
                    &h,
                    &format!(
                        "{project}/repository/compare?from={from}&to={to}&straight={straight}"
                    ),
                    c.clone(),
                )
            });
            // A tag lists its release's notes, so no separate release call is needed.
            let listed = (|| {
                let (mut tags, mut notes) = (vec![], std::collections::HashMap::new());
                for page in 1..=TAG_PAGES {
                    let v = self.get(
                        &h,
                        &format!("{project}/repository/tags?per_page=100&page={page}"),
                        c.clone(),
                    )?;
                    let rows = v.as_array().ok_or("Expected a GitLab tag list")?;
                    for t in rows {
                        // A tag on a tree or blob names no commit.
                        let Ok(commit) = sha(t, "/commit/id") else {
                            continue;
                        };
                        let name = string(t, "/name")?;
                        if let Some(body) = t["release"]["description"].as_str() {
                            notes.insert(name.clone(), body.to_owned());
                        }
                        tags.push((name, commit));
                    }
                    if rows.len() < 100 {
                        return Ok((tags, notes, false));
                    }
                }
                Ok::<_, crate::AdapterError>((tags, notes, true))
            })();
            (joined(compare), listed)
        });
        let (compare, (tags, notes, truncated)) = (compare?, listed?);
        if truncated {
            warnings.push(format!(
                "Only the first {} tags were checked for releases in this range.",
                TAG_PAGES * 100
            ));
        }
        let commits = compare["commits"].as_array().map_or(&[][..], Vec::as_slice);
        let range = commits
            .iter()
            .map(|v| Ok((sha(v, "/id")?, sha(v, "/parent_ids/0").ok())))
            .collect::<Result<Vec<_>>>()?;
        let released = notes.keys().cloned().collect();
        let (steps, folded) = steps(&range, from, to, &tags, &released);
        warnings.extend(folded_warning(&folded));
        let graph: Vec<(String, Vec<String>)> = commits
            .iter()
            .zip(&range)
            .map(|(v, (id, _))| {
                let parents = v["parent_ids"].as_array().map_or(&[][..], Vec::as_slice);
                (
                    id.clone(),
                    (0..parents.len())
                        .filter_map(|i| sha(v, &format!("/parent_ids/{i}")).ok())
                        .collect(),
                )
            })
            .collect();
        let shas = members(&graph, &steps);
        let stats = |v: &Value| {
            let diffs = v["diffs"].as_array().map_or(&[][..], Vec::as_slice);
            (
                v["compare_timeout"] == true || diffs.len() >= DIFF_FILES,
                v["commits"].as_array().map_or(0, |c| c.len() as u64),
                diffs
                    .iter()
                    .map(|d| {
                        let lines = d["diff"].as_str().unwrap_or_default().lines();
                        let (mut additions, mut deletions) = (0, 0);
                        for line in lines {
                            match line.as_bytes().first() {
                                Some(b'+') => additions += 1,
                                Some(b'-') => deletions += 1,
                                _ => {}
                            }
                        }
                        ReleaseFile {
                            path: d["new_path"].as_str().unwrap_or_default().into(),
                            additions,
                            deletions,
                            previous: d["old_path"]
                                .as_str()
                                .filter(|_| d["renamed_file"] == true)
                                .map(str::to_owned),
                        }
                    })
                    .collect::<Vec<_>>(),
            )
        };
        // A single step spanning the whole compare is the compare already read.
        let stats = match &steps[..] {
            // The compare's own warnings already say when its file list was cut.
            [only] if only.from == from && only.to == to => {
                let (_, commits, files) = stats(&compare);
                vec![(false, commits, files)]
            }
            _ => bounded(&steps, |step| {
                self.get(
                    &h,
                    &format!(
                        "{project}/repository/compare?from={}&to={}&straight={straight}",
                        step.from, step.to
                    ),
                    c.clone(),
                )
                .map(|v| stats(&v))
            })?,
        };
        let dates: std::collections::HashMap<&str, &str> = commits
            .iter()
            .filter_map(|v| Some((v["id"].as_str()?, v["created_at"].as_str()?)))
            .collect();
        let releases = steps
            .into_iter()
            .zip(stats)
            .zip(shas)
            .map(|((step, (cut, commits, files)), shas)| {
                if cut {
                    warnings.push(format!(
                        "GitLab timed out or reached its file limit comparing {}, so the file tree may leave some of its files out.",
                        step.tag.as_deref().unwrap_or(&refs.head)
                    ));
                }
                let notes = step.tag.as_ref().and_then(|t| notes.get(t));
                Release {
                    url: step.tag.as_deref().map(|t| {
                        format!(
                            "https://{}/{path}/-/{}/{}",
                            h.host,
                            if notes.is_some() { "releases" } else { "tags" },
                            crate::github::encode_path(t)
                        )
                    }),
                    notes: notes.filter(|b| !b.trim().is_empty()).cloned(),
                    date: dates.get(step.to.as_str()).map(|d| d.to_string()),
                    tag: step.tag,
                    commit: step.to,
                    commits,
                    files,
                    shas,
                }
            })
            .collect();
        Ok((releases, warnings))
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
        let approved = self
            .get(a, &format!("{}/approvals", a.root()), c.clone())
            .ok()
            .and_then(|v| Some(!v["approved_by"].as_array()?.is_empty()))
            .unwrap_or(false);
        let requested_changes = self
            .get(a, &format!("{}/reviewers", a.root()), c.clone())
            .ok()
            .and_then(|v| {
                Some(
                    v.as_array()?
                        .iter()
                        .any(|reviewer| reviewer["state"] == "requested_changes"),
                )
            })
            .unwrap_or(false);
        let mut overview = Overview {
            description: m["description"].as_str().map(str::to_owned),
            author: m["author"]["username"].as_str().map(str::to_owned),
            decision: if requested_changes {
                Some(ReviewDecision::ChangesRequested)
            } else {
                approved.then_some(ReviewDecision::Approved)
            },
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
impl GitlabReader {
    /// Head blame for each span, one call per file asking only for the span's lines.
    /// A file that cannot be read is `None`.
    pub fn blame(
        &self,
        t: &RemoteTarget,
        spans: &[Span],
        cancel: Cancellation,
    ) -> Result<Vec<Option<Blame>>> {
        let a = MrAddress::from_target(t);
        let project = crate::github::encode_path(&a.project).replace('/', "%2F");
        crate::releases::bounded(spans, |(path, first, last)| {
            let v = self.get(
                &a,
                &format!(
                    "projects/{project}/repository/files/{}/blame?ref={}&range[start]={first}&range[end]={last}",
                    crate::github::encode_path(path).replace('/', "%2F"),
                    t.head
                ),
                cancel.clone(),
            );
            // GitLab groups consecutive lines by commit, numbered from the span's first line.
            let mut line = *first;
            Ok(v.ok().and_then(|v| {
                v.as_array()?
                    .iter()
                    .map(|group| {
                        let n = u32::try_from(group["lines"].as_array()?.len()).ok()?;
                        let range = (
                            line,
                            line + n.checked_sub(1)?,
                            sha(group, "/commit/id").ok()?,
                        );
                        line += n;
                        Some(range)
                    })
                    .collect()
            }))
        })
    }
}
fn string(v: &Value, key: &str) -> Result<String> {
    v.pointer(key)
        .and_then(Value::as_str)
        .map(str::to_owned)
        .ok_or_else(|| format!("GitLab response missing {key}").into())
}
fn sha(v: &Value, key: &str) -> Result<String> {
    let s = string(v, key)?;
    if !matches!(s.len(), 40 | 64) || !s.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err("the object ID GitLab sent is not valid".into());
    }
    Ok(s.to_ascii_lowercase())
}
/// GitLab's compare lists hunks without file headers; rebuild a unified diff `parse_patch` reads.
fn patch_from_diffs(diffs: &[Value]) -> Vec<u8> {
    let mut out = Vec::new();
    for d in diffs {
        let (Some(old), Some(new)) = (d["old_path"].as_str(), d["new_path"].as_str()) else {
            continue;
        };
        let mode = |key: &str| d[key].as_str().unwrap_or("100644");
        out.extend_from_slice(
            format!(
                "diff --git {} {}\n",
                quote_path("a/", old),
                quote_path("b/", new)
            )
            .as_bytes(),
        );
        if d["new_file"] == true {
            out.extend_from_slice(format!("new file mode {}\n", mode("b_mode")).as_bytes());
        } else if d["deleted_file"] == true {
            out.extend_from_slice(format!("deleted file mode {}\n", mode("a_mode")).as_bytes());
        } else if mode("a_mode") != mode("b_mode") {
            out.extend_from_slice(
                format!("old mode {}\nnew mode {}\n", mode("a_mode"), mode("b_mode")).as_bytes(),
            );
        }
        if d["renamed_file"] == true && old != new {
            out.extend_from_slice(
                format!(
                    "rename from {}\nrename to {}\n",
                    quote_path("", old),
                    quote_path("", new)
                )
                .as_bytes(),
            );
        }
        let text = d["diff"].as_str().unwrap_or_default();
        let omitted = text.is_empty() && (d["too_large"] == true || d["collapsed"] == true);
        if omitted {
            out.extend_from_slice(b"GitLab omitted its text\n");
        }
        if !text.is_empty() || omitted {
            let a = if d["new_file"] == true {
                "/dev/null".to_string()
            } else {
                quote_path("a/", old)
            };
            let b = if d["deleted_file"] == true {
                "/dev/null".to_string()
            } else {
                quote_path("b/", new)
            };
            out.extend_from_slice(format!("--- {a}\n+++ {b}\n").as_bytes());
        }
        out.extend_from_slice(text.as_bytes());
        if !text.is_empty() && !text.ends_with('\n') {
            out.push(b'\n');
        }
    }
    out
}
fn joined<T>(handle: std::thread::ScopedJoinHandle<'_, Result<T>>) -> Result<T> {
    handle
        .join()
        .unwrap_or_else(|_| Err("a GitLab read stopped unexpectedly".into()))
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
        compare: None,
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
            if pos["position_type"] == "text" || pos["position_type"] == "file" {
                let file_level = pos["position_type"] == "file";
                let right = file_level || pos["new_line"].as_u64().is_some();
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
                    file_level: Some(file_level),
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub line_range: Option<GitlabLineRange>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GitlabLineRange {
    pub start: GitlabRangeLine,
    pub end: GitlabRangeLine,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GitlabRangeLine {
    pub line_code: String,
    #[serde(rename = "type")]
    pub side: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub old_line: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub new_line: Option<u32>,
}

impl GitlabRangeLine {
    fn new(path_hash: &str, side: Side, row: &diffz_core::patch::PatchRow) -> Self {
        Self {
            line_code: format!(
                "{path_hash}_{}_{}",
                row.old_line.unwrap_or(0),
                row.new_line.unwrap_or(0)
            ),
            side: if side == Side::Right { "new" } else { "old" }.into(),
            old_line: row.old_line,
            new_line: row.new_line,
        }
    }
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
        "GitLab"
    }
    fn address_label(&self) -> &str {
        "Merge request or compare URL"
    }
    fn address_hint(&self) -> &str {
        "Enter group/project!123, a GitLab merge request URL, or a compare URL"
    }
    fn address_help(&self) -> &str {
        "Uses glab with GitLab.com or a self-managed HTTPS server. A compare URL opens read-only."
    }
    fn write_flag(&self) -> &str {
        "--allow-gitlab-writes"
    }
    fn preview_note(&self) -> Option<&str> {
        Some(
            "GitLab accepts comments, approval, and change requests. Inline drafts can cover lines in one diff hunk.",
        )
    }
    fn reopen_address(&self, t: &RemoteTarget) -> String {
        let r = &t.repository;
        if let Some(c) = &t.compare {
            let dots = if c.direct { ".." } else { "..." };
            return format!(
                "https://{}/{}/{}/-/compare/{}{dots}{}",
                r.host,
                r.owner,
                r.name,
                crate::github::encode_path(&c.base),
                crate::github::encode_path(&c.head)
            );
        }
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
    fn check(
        &self,
        _verdict: Verdict,
        summary: &str,
        drafts: &[Draft],
    ) -> std::result::Result<(), ReviewError> {
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
            line_range: None,
        };
        if !d.is_file_level() {
            let row = f
                .line(d.side, d.line)
                .ok_or_else(|| fail("the draft holds no source line to post to GitLab"))?;
            position.position_type = "text".into();
            position.old_line = row.old_line;
            position.new_line = row.new_line;
            if d.start_line != d.line {
                use sha1::{Digest, Sha1};
                let start = f
                    .line(d.side, d.start_line)
                    .ok_or_else(|| fail("the draft starts outside the GitLab diff"))?;
                let path_hash = Sha1::digest(new.as_bytes())
                    .iter()
                    .map(|byte| format!("{byte:02x}"))
                    .collect::<String>();
                position.line_range = Some(GitlabLineRange {
                    start: GitlabRangeLine::new(&path_hash, d.side, start),
                    end: GitlabRangeLine::new(&path_hash, d.side, row),
                });
            }
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
        match GitlabTarget::parse(address)? {
            GitlabTarget::Mr(a) => self.reader()?.snapshot(&a, cancel),
            GitlabTarget::Compare(a) => self.reader()?.compare(&a, cancel),
        }
    }
    fn accepts(&self, address: &str) -> bool {
        GitlabTarget::parse(address).is_ok()
    }
    fn source(&self, t: &RemoteTarget, path: &str, revision: &str) -> Result<Vec<u8>> {
        self.reader()?.source(t, path, revision)
    }
    fn remote(&self) -> Result<Arc<dyn ReviewRemote>> {
        Ok(Arc::new(GitlabWriter::new(self.reader()?.clone())))
    }
    fn releases(
        &self,
        t: &RemoteTarget,
        cancel: Cancellation,
    ) -> Result<(Vec<diffz_core::review_details::Release>, Vec<String>)> {
        self.reader()?.releases(t, cancel)
    }
    fn blame(
        &self,
        t: &RemoteTarget,
        spans: &[Span],
        cancel: Cancellation,
    ) -> Result<Vec<Option<Blame>>> {
        self.reader()?.blame(t, spans, cancel)
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
/// Recognize file-level notes written before positioned file discussions were supported.
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
        } else if review["state"] == "CHANGES_REQUESTED" {
            let reviewers = self.reader.get(
                &a,
                &format!("{}/reviewers", a.root()),
                Cancellation::default(),
            )?;
            if !reviewers.as_array().is_some_and(|users| {
                users.iter().any(|u| {
                    u["user"]["username"] == t.account && u["state"] == "requested_changes"
                })
            }) {
                return Err("GitLab change request could not be verified".into());
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
                    if n["position"]["position_type"] != "text"
                        && n["position"]["position_type"] != "file"
                    {
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
                    if pos["position_type"] == "file" {
                        result.push(json!({"body":marked["body"],"path":pos["new_path"],"line":0,"start_line":0,"side":"RIGHT"}));
                        continue;
                    }
                    let right = marked["side"]
                        .as_str()
                        .map_or(pos["new_line"].as_u64().is_some(), |s| s == "RIGHT");
                    let key = if right { "new" } else { "old" };
                    let line = &pos[format!("{key}_line")];
                    let start_line = pos["line_range"]["start"][format!("{key}_line")]
                        .as_u64()
                        .map_or_else(|| line.clone(), |n| json!(n));
                    result.push(json!({"body":marked["body"],"path":pos[format!("{key}_path")],"line":line,"start_line":start_line,"side":if right{"RIGHT"}else{"LEFT"}}));
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
        if !p.verify(&GitlabRules) {
            return SendOutcome::Rejected(422);
        }
        let a = MrAddress::from_target(&p.target);
        let tag = marker(p);
        let mut sent = false;
        let result = (|| -> Result<Value> {
            if p.verdict == Verdict::RequestChanges {
                if self.current(&p.target)? != p.target {
                    return Err("MR changed before requesting changes".into());
                }
                sent = true;
                let response = self.reader.request(
                    &a,
                    "graphql",
                    "POST",
                    Some(&json!({
                        "query": "mutation($projectPath: ID!, $iid: String!) { mergeRequestRequestChanges(input: { projectPath: $projectPath, iid: $iid }) { errors mergeRequest { iid } } }",
                        "variables": {"projectPath": a.project, "iid": a.number.to_string()}
                    })),
                    Cancellation::default(),
                )?;
                if response.status != 200 {
                    return Err(
                        format!("GitLab change request returned HTTP {}", response.status).into(),
                    );
                }
                let body: Value = serde_json::from_slice(&response.body)?;
                let result = &body["data"]["mergeRequestRequestChanges"];
                if body["errors"]
                    .as_array()
                    .is_some_and(|errors| !errors.is_empty())
                    || result["errors"]
                        .as_array()
                        .is_some_and(|errors| !errors.is_empty())
                    || result["mergeRequest"]["iid"]
                        .as_str()
                        .and_then(|iid| iid.parse::<u64>().ok())
                        != Some(a.number)
                {
                    return Err("GitLab did not confirm the change request".into());
                }
            }
            for comment in &p.comments {
                if self.current(&p.target)? != p.target {
                    return Err("MR changed during publication".into());
                }
                let pos = comment
                    .position
                    .as_ref()
                    .ok_or("Frozen GitLab position is absent")?;
                let endpoint = format!("{}/discussions", a.root());
                let payload = json!({"body":format!("{}{}:{} -->",comment.body,tag.trim_end_matches(" -->"),comment.side.api()),"position":pos});
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
    fn positioned_file_discussion_keeps_its_anchor_kind() {
        let discussion = serde_json::json!({"notes": [{
            "id": 1,
            "body": "note",
            "author": {"username": "reviewer"},
            "position": {
                "position_type": "file",
                "new_path": "src/review.rs",
                "head_sha": "head"
            }
        }]});
        let comments = discussion_comments(&[discussion], &mut Overview::default());
        assert_eq!(comments.len(), 1);
        assert_eq!(comments[0].file_level, Some(true));
        assert_eq!(comments[0].line, None);
    }
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
    #[test]
    fn patch_from_diffs_keeps_binary_and_rename_only_entries() {
        use diffz_core::patch::ChangeKind;
        let diffs = vec![
            serde_json::json!({"old_path":"logo.png","new_path":"logo.png","a_mode":"100644","b_mode":"100644","diff":""}),
            serde_json::json!({"old_path":"old name.txt","new_path":"new name.txt","a_mode":"100644","b_mode":"100644","renamed_file":true,"diff":""}),
        ];
        let patch = parse_patch(&patch_from_diffs(&diffs), ParseLimits::default()).unwrap();
        assert_eq!(patch.files.len(), 2);
        assert_eq!(patch.files[0].display_path(), "logo.png");
        assert!(patch.files.iter().all(|f| f.hunks.is_empty()));
        assert!(
            matches!(patch.files[1].kind, ChangeKind::Renamed),
            "{:?}",
            patch.files[1].kind
        );
        assert_eq!(patch.files[1].display_path(), "new name.txt");
    }
}
