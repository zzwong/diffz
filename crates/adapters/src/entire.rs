//! Read-only Entire Trails. The released `entire` CLI owns login and JSON API
//! transport; curl reads raw blobs because `entire api` reformats JSON bodies.
//! Every read is bounded, and repository placement and immutable refs are checked
//! before a snapshot is installed. No review, approval, or merge endpoint is used.
use crate::{
    AdapterError, Result,
    process::{ProcessRequest, Runner, resolve_program},
    provider::{ProviderOpen, ReviewChoices, ReviewProvider, ReviewRemote},
};
use diffz_core::{
    domain::*,
    patch::{ParseLimits, PatchReport, parse_patch},
    provider::{Cancellation, OpenRequest, ReviewRules, SourceChoice},
    review::{PreparedReview, Verdict},
    source_link::encode,
};
use serde::Deserialize;
use serde_json::{Value, value::RawValue};
use std::{collections::BTreeSet, path::PathBuf, sync::Arc, time::Duration};

mod selection;
pub use selection::{ChangeChoice, TrailChoices};

const READ_ONLY: &str = "Entire Trails are read-only in Diffz. Local drafts and exports remain available; publish reviews in Entire.";
const SOURCE_LINK_UNAVAILABLE: &str =
    "Entire immutable source links are not available yet; open the Trail in your browser.";
const MAX_RESPONSE: usize = 32 * 1024 * 1024;
const CURL_STATUS: &[u8] = b"\nDIFFZ_ENTIRE_HTTP:";

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TrailSelection {
    Project {
        number: u64,
        change: Option<(String, u64)>,
    },
    Repository {
        repo: String,
        number: u64,
    },
}

/// Current project Trails and the older repository-scoped Trail URLs use
/// different numbers. Preserve both namespaces instead of guessing from a number.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TrailAddress {
    pub host: String,
    pub forge: String,
    pub project: String,
    pub selection: TrailSelection,
}
impl TrailAddress {
    pub fn parse(value: &str) -> Result<Self> {
        let value = value.trim();
        let u = strict_https_url(value)?;
        let host = u.host_str().ok_or("Entire URL has no host")?;
        if !matches!(host, "entire.com" | "entire.io") {
            return Err("Entire Trail URLs must use entire.com or entire.io".into());
        }
        if u.query().is_some() {
            return Err("query strings are not allowed in an Entire Trail identity".into());
        }
        let p: Vec<_> = u.path().trim_end_matches('/').split('/').skip(1).collect();
        if p.len() < 5 || !matches!(p[0], "et" | "gh") || !project_name(p[1]) {
            return Err("expected an Entire URL with et/<project> or gh/<owner>".into());
        }
        let selection = if p[2] == "-" && p[3] == "trails" {
            let number = trail_number(p[4])?;
            let change = match p.len() {
                5 => None,
                8 if p[5] == "changes" && repo_name(p[6]) => {
                    Some((p[6].into(), trail_number(p[7])?))
                }
                _ => {
                    return Err(
                        "expected /-/trails/NUMBER or /-/trails/NUMBER/changes/REPO/NUMBER".into(),
                    );
                }
            };
            TrailSelection::Project { number, change }
        } else if matches!(p.len(), 5 | 6)
            && repo_name(p[2])
            && p[3] == "trails"
            && p.get(5).is_none_or(|slug| repo_name(slug))
        {
            TrailSelection::Repository {
                repo: p[2].into(),
                number: trail_number(p[4])?,
            }
        } else {
            return Err("expected an Entire project Trail or repository Change URL".into());
        };
        Ok(Self {
            host: host.into(),
            forge: p[0].into(),
            project: p[1].to_ascii_lowercase(),
            selection,
        })
    }

    fn change_url(&self, repo: &str, number: u64) -> String {
        match self.selection {
            TrailSelection::Project { number: trail, .. } => format!(
                "https://entire.com/{}/{}/-/trails/{trail}/changes/{repo}/{number}",
                self.forge, self.project
            ),
            TrailSelection::Repository { .. } => format!(
                "https://entire.com/{}/{}/{repo}/trails/{number}",
                self.forge, self.project
            ),
        }
    }

    fn canonical_url(&self) -> String {
        match &self.selection {
            TrailSelection::Project {
                number,
                change: None,
            } => format!(
                "https://entire.com/{}/{}/-/trails/{number}",
                self.forge, self.project
            ),
            TrailSelection::Project {
                change: Some((repo, number)),
                ..
            }
            | TrailSelection::Repository { repo, number } => self.change_url(repo, *number),
        }
    }
}

fn project_name(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 39
        && s.as_bytes().first().is_some_and(u8::is_ascii_alphanumeric)
        && s.as_bytes().last().is_some_and(u8::is_ascii_alphanumeric)
        && s.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
}
fn repo_name(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 100
        && !matches!(s, "." | ".." | "-")
        && s.bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"-_.".contains(&b))
}
fn trail_number(s: &str) -> Result<u64> {
    if s.is_empty() || s.len() > 10 || !s.bytes().all(|b| b.is_ascii_digit()) {
        return Err("Entire Trail and Change numbers must be positive integers".into());
    }
    let n = s
        .parse::<u64>()
        .map_err(|_| AdapterError::Message("invalid Entire number".into()))?;
    if n == 0 {
        return Err("Entire Trail and Change numbers must be positive integers".into());
    }
    Ok(n)
}
fn opaque_id(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 128
        && s.bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"_-".contains(&b))
}
fn placement_id(s: &str) -> bool {
    s.len() == 26
        && s.as_bytes()[0] <= b'7'
        && s.bytes()
            .all(|b| b"0123456789ABCDEFGHJKMNPQRSTVWXYZ".contains(&b))
}
fn commit_id(s: &str) -> bool {
    matches!(s.len(), 40 | 64) && s.bytes().all(|b| b.is_ascii_hexdigit())
}

// Check raw segments before URL parsing: URL normalization otherwise erases a
// `..` segment or the explicitly written default port that we must reject.
fn strict_https_url(raw: &str) -> Result<url::Url> {
    if !raw.starts_with("https://")
        || raw.bytes().any(|b| b.is_ascii_control() || b == b'\\')
        || raw.split('/').any(|p| p == "." || p == "..")
    {
        return Err("Entire URLs must be absolute HTTPS URLs without relative segments".into());
    }
    let authority = raw[8..].split(['/', '?', '#']).next().unwrap_or_default();
    if authority.is_empty() || authority.contains(['@', ':']) {
        return Err("credentials and ports are not allowed in Entire URLs".into());
    }
    let raw_path = raw[8 + authority.len()..]
        .split(['?', '#'])
        .next()
        .unwrap_or_default();
    if raw_path.contains('%') {
        return Err("encoded Entire URL identities are not accepted".into());
    }
    let u = url::Url::parse(raw).map_err(|_| AdapterError::Message("invalid Entire URL".into()))?;
    if !u
        .host_str()
        .is_some_and(|host| authority.eq_ignore_ascii_case(host))
        || !u.username().is_empty()
        || u.password().is_some()
        || u.port().is_some()
        || u.path().contains('%')
    {
        return Err("invalid or encoded Entire URL identity".into());
    }
    Ok(u)
}

/// Only authoritative production cell origins may receive credentials. A
/// directory response cannot introduce an arbitrary credential destination.
fn api_origin(raw: &str) -> Result<String> {
    let u = strict_https_url(raw)?;
    let host = u.host_str().unwrap_or_default();
    if !(host.ends_with(".api.entire.io") || host.ends_with(".api.entire.com"))
        || !matches!(u.path(), "" | "/")
        || u.query().is_some()
        || u.fragment().is_some()
    {
        return Err("Entire did not return a trusted HTTPS API cell origin".into());
    }
    Ok(format!("https://{host}"))
}

// Do not derive Debug for a credential, or attach any of its bytes to errors.
struct Login {
    token: String,
}
impl Login {
    fn from_token(raw: &[u8]) -> Result<Self> {
        let token = std::str::from_utf8(raw)
            .map_err(|_| AdapterError::Message("Entire returned an invalid login token".into()))?
            .trim();
        if token.is_empty()
            || token.len() > 16 * 1024
            || !token
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"._-".contains(&b))
        {
            return Err("Entire returned an invalid login token; run entire login".into());
        }
        let parts: Vec<_> = token.split('.').collect();
        if parts.len() != 3 || parts.iter().any(|p| p.is_empty()) {
            return Err("Entire requires a login or service-account JWT; run entire login".into());
        }
        let payload = decode_base64url(parts[1])?;
        let claims: Value = serde_json::from_slice(&payload)
            .map_err(|_| AdapterError::Message("Entire login claims could not be read".into()))?;
        let audiences: Vec<&str> = match &claims["aud"] {
            Value::String(a) => vec![a],
            Value::Array(a) => a.iter().filter_map(Value::as_str).collect(),
            _ => vec![],
        };
        // Match v0.12.2's first URL-shaped audience rule. Claims are unverified
        // here and only constrain routing; Entire still authenticates the token.
        let audience = audiences
            .into_iter()
            .find(|a| url::Url::parse(a).is_ok_and(|u| !u.scheme().is_empty()))
            .ok_or("Entire login has no control-plane audience")?;
        let u = strict_https_url(audience)?;
        let host = u.host_str().unwrap_or_default();
        // The CLI's EnvToken contract also accepts core.<region> origins.
        // Restrict that compatibility form to a single DNS label under Entire;
        // it does not authorize arbitrary token audiences or custom deployments.
        let core_host = host
            .strip_prefix("core.")
            .and_then(|host| {
                host.strip_suffix(".entire.io")
                    .or_else(|| host.strip_suffix(".entire.com"))
            })
            .is_some_and(project_name);
        if !(core_host
            || matches!(host, "auth.entire.io" | "auth.entire.com")
            || host.ends_with(".auth.entire.io")
            || host.ends_with(".auth.entire.com")
            || host.ends_with(".console.entire.io")
            || host.ends_with(".console.entire.com"))
            || !matches!(u.path(), "" | "/")
            || u.query().is_some()
            || u.fragment().is_some()
        {
            return Err("Select an Entire production login before opening entire.com links".into());
        }
        Ok(Self {
            token: token.into(),
        })
    }
}

fn decode_base64url(s: &str) -> Result<Vec<u8>> {
    let mut out = Vec::with_capacity(s.len() * 3 / 4);
    let mut value = 0u32;
    let mut bits = 0;
    for byte in s.bytes() {
        let digit = match byte {
            b'A'..=b'Z' => byte - b'A',
            b'a'..=b'z' => byte - b'a' + 26,
            b'0'..=b'9' => byte - b'0' + 52,
            b'-' => 62,
            b'_' => 63,
            _ => return Err("Entire login has invalid encoded claims".into()),
        };
        value = (value << 6) | u32::from(digit);
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((value >> bits) as u8);
            value &= (1 << bits) - 1;
        }
    }
    if bits == 6 || value != 0 {
        return Err("Entire login has invalid encoded claims".into());
    }
    Ok(out)
}

#[derive(Clone)]
pub struct EntireReader {
    program: PathBuf,
    curl: Option<PathBuf>,
}
impl EntireReader {
    pub fn new(program: PathBuf) -> Self {
        Self {
            program,
            curl: resolve_program("curl").ok(),
        }
    }
    /// Allows callers with an explicitly resolved curl executable to reuse it.
    pub fn with_curl(program: PathBuf, curl: PathBuf) -> Self {
        Self {
            program,
            curl: Some(curl),
        }
    }

    fn command(&self, args: &[&str]) -> ProcessRequest {
        let mut request = ProcessRequest::new(self.program.clone()).args(args.iter().copied());
        request.deadline = Duration::from_secs(60);
        // A pasted production URL must not silently use an ambient development
        // API host. Explicit authentication context/ENTIRE_TOKEN remains the
        // user's choice; after login we pin that credential for the operation.
        request.env_remove = [
            "ENTIRE_API_BASE_URL",
            "ENTIRE_AUTH_BASE_URL",
            "ENTIRE_WEB_BASE_URL",
        ]
        .into_iter()
        .map(Into::into)
        .collect();
        request
    }
    fn login(&self, cancel: Cancellation) -> Result<Login> {
        let mut request = self.command(&["auth", "token"]);
        request.stdout_limit = 16 * 1024;
        let output = Runner::run(request, cancel)?;
        if !output.status.success() {
            return Err("Entire authentication failed. Install Entire CLI 0.12.2 or newer and run entire login.".into());
        }
        Login::from_token(&output.stdout)
    }
    fn request(
        &self,
        login: &Login,
        origin: Option<&str>,
        endpoint: &str,
        cancel: Cancellation,
    ) -> Result<Vec<u8>> {
        if !endpoint.starts_with("/api/v1/")
            || endpoint.contains(['\r', '\n', '\\', '{', '}'])
            || url::Url::parse(endpoint).is_ok()
        {
            return Err("invalid relative Entire API endpoint".into());
        }
        let mut request = self.command(&[
            "api",
            "--to",
            if origin.is_some() { "cell" } else { "core" },
            "--include",
            "--method",
            "GET",
            endpoint,
        ]);
        request
            .env
            .insert("ENTIRE_TOKEN".into(), login.token.clone().into());
        if let Some(origin) = origin {
            let origin = api_origin(origin)?;
            request
                .env_remove
                .retain(|name| name != "ENTIRE_API_BASE_URL");
            request
                .env
                .insert("ENTIRE_API_BASE_URL".into(), origin.into());
        }
        let output = Runner::run(request, cancel)?;
        // Entire --include writes headers to stderr, unlike gh. Error bodies,
        // stderr and credentials are deliberately excluded from error strings.
        let status = String::from_utf8_lossy(&output.stderr)
            .lines()
            .filter_map(|line| {
                let mut words = line.split_whitespace();
                words.next().filter(|p| p.starts_with("HTTP/"))?;
                words.next()?.parse::<u16>().ok()
            })
            .next_back();
        if !output.status.success() || status.is_none_or(|s| !(200..300).contains(&s)) {
            return Err(match status {
                Some(401 | 403) => "Entire access was denied; check the selected login and repository permissions".into(),
                Some(404) => "Entire could not read this resource at its primary placement; it may be unavailable, inaccessible, or not indexed yet".into(),
                Some(429) => "Entire rate limited this read; retry after its limit resets".into(),
                Some(s) => format!("Entire API read failed (HTTP {s}); the existing session is unchanged").into(),
                None => "Entire API read failed. Check entire login, CLI 0.12.2 or newer, and network access.".into(),
            });
        }
        Ok(output.stdout)
    }
    fn json(
        &self,
        login: &Login,
        origin: Option<&str>,
        endpoint: &str,
        cancel: Cancellation,
    ) -> Result<Value> {
        let bytes = self.request(login, origin, endpoint, cancel)?;
        serde_json::from_slice(&bytes)
            .map_err(|_| "Entire returned invalid JSON for an API read".into())
    }
    fn account(&self, login: &Login, cancel: Cancellation) -> Result<String> {
        let account = self.json(login, None, "/api/v1/me", cancel)?;
        let id = text(&account, "/global/accountId")?;
        if !opaque_id(id) {
            return Err("Entire did not return an account identity".into());
        }
        Ok(id.into())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Repository {
    id: String,
    placement: String,
    origin: String,
    name: String,
}
#[derive(Debug, Clone, PartialEq, Eq)]
struct ProjectTrail {
    origin: String,
    project_id: String,
    id: String,
    endpoint: String,
    forge: String,
    name: String,
    number: u64,
}
#[derive(Debug, Clone)]
struct Selected {
    repo: String,
    number: u64,
    change_id: Option<String>,
    placement: Option<String>,
    project: Option<ProjectTrail>,
    parent_number: Option<u64>,
}
fn text<'a>(v: &'a Value, path: &str) -> Result<&'a str> {
    v.pointer(path)
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| format!("Entire response is missing {path}").into())
}
fn query(path: &str, values: &[(&str, &str)]) -> String {
    let mut encoder = url::form_urlencoded::Serializer::new(String::new());
    for (key, value) in values {
        encoder.append_pair(key, value);
    }
    format!("{path}?{}", encoder.finish())
}

impl EntireReader {
    fn repository(
        &self,
        login: &Login,
        a: &TrailAddress,
        repo: &str,
        cancel: Cancellation,
    ) -> Result<Repository> {
        if !repo_name(repo) {
            return Err("invalid Entire repository name".into());
        }
        let qualified = format!("{}/{}/{repo}", a.forge, a.project);
        // The forge-qualified exact filter does not enumerate the account or
        // paginate. Verify provider AND name: gh/acme/app and et/acme/app differ.
        let directory = self.json(
            login,
            None,
            &query("/api/v1/repos", &[("filter", &qualified)]),
            cancel,
        )?;
        let rows = directory["repos"]
            .as_array()
            .ok_or("Entire repository directory has no repos array")?;
        if directory["truncated"] == true
            || directory["nextPageToken"]
                .as_str()
                .is_some_and(|s| !s.is_empty())
        {
            return Err(
                "Entire's exact repository lookup was incomplete; retry before opening this Change"
                    .into(),
            );
        }
        let expected_provider = if a.forge == "et" { "entire" } else { "github" };
        let matches: Vec<_> = rows
            .iter()
            .filter(|r| directory_name(r, &a.forge, &a.project, repo, expected_provider).is_some())
            .collect();
        if matches.len() != 1 {
            return Err(
                "Entire could not resolve one accessible repository with this forge-qualified name"
                    .into(),
            );
        }
        let row = matches[0];
        let id = text(row, "/id")?;
        if !opaque_id(id) {
            return Err("Entire returned an invalid repository identity".into());
        }
        let primary = text(row, "/primaries/processing")?;
        if !placement_id(primary) {
            return Err("Entire repository has no usable processing-primary placement".into());
        }
        let placements = row["placements"]
            .as_array()
            .ok_or("Entire repository has no placements")?;
        let matches: Vec<_> = placements.iter().filter(|p| p["id"] == primary).collect();
        if matches.len() != 1 {
            return Err(
                "Entire processing primary is absent or ambiguous in its placement directory"
                    .into(),
            );
        }
        let placement = matches[0];
        if placement["status"] != "ready" {
            return Err(
                "Entire's processing-primary placement is not ready; retry when indexing completes"
                    .into(),
            );
        }
        let origin = api_origin(text(placement, "/apiBaseUrl")?)?;
        let canonical = directory_name(row, &a.forge, &a.project, repo, expected_provider)
            .ok_or("invalid Entire repository name")?;
        if !repo_name(canonical) {
            return Err("invalid Entire repository name".into());
        }
        Ok(Repository {
            id: id.into(),
            placement: primary.into(),
            origin,
            name: canonical.into(),
        })
    }

    fn project(
        &self,
        login: &Login,
        a: &TrailAddress,
        number: u64,
        cancel: Cancellation,
    ) -> Result<(ProjectTrail, Value)> {
        let resolved = self.json(
            login,
            None,
            &format!("/api/v1/projects/resolve/{}/{}", a.forge, a.project),
            cancel.clone(),
        )?;
        if resolved["reference"]["host"] != a.forge
            || !text(&resolved, "/reference/project")?.eq_ignore_ascii_case(&a.project)
        {
            return Err("Entire resolved a different project identity".into());
        }
        let project_id = text(&resolved, "/project/id")?;
        if !placement_id(project_id) {
            return Err("Entire project has no stable identity".into());
        }
        text(&resolved, "/project/primaryProcessingCell")?;
        text(&resolved, "/project/region")?;
        let origin = api_origin(text(&resolved, "/project/apiUrl")?)?;
        let endpoint = query(
            &format!("/api/v1/{}/{}/trails/{number}", a.forge, a.project),
            &[("project_id", project_id)],
        );
        let detail = self.json(login, Some(&origin), &endpoint, cancel)?;
        if detail["projectId"] != project_id || detail["number"].as_u64() != Some(number) {
            return Err("Entire returned a different project Trail".into());
        }
        let id = text(&detail, "/id")?;
        if !placement_id(id) {
            return Err("Entire project Trail has no stable identity".into());
        }
        Ok((
            ProjectTrail {
                origin,
                project_id: project_id.into(),
                id: id.into(),
                endpoint,
                forge: a.forge.clone(),
                name: a.project.clone(),
                number,
            },
            detail,
        ))
    }

    fn select(&self, login: &Login, a: &TrailAddress, cancel: Cancellation) -> Result<Selected> {
        match &a.selection {
            TrailSelection::Repository { repo, number } => Ok(Selected {
                repo: repo.clone(),
                number: *number,
                change_id: None,
                placement: None,
                project: None,
                parent_number: None,
            }),
            TrailSelection::Project {
                number: parent_number,
                change: Some((repo, number)),
            } => Ok(Selected {
                // Repository readers can inspect this Change without permission
                // to enumerate every Change in its project Trail. The returned
                // parent is verified against the pasted project number below.
                repo: repo.clone(),
                number: *number,
                change_id: None,
                placement: None,
                project: None,
                parent_number: Some(*parent_number),
            }),
            TrailSelection::Project {
                number,
                change: None,
            } => {
                let (project, detail) = self.project(login, a, *number, cancel)?;
                let (repo, number, id, placement) = select_project_change(a, &detail, None)?;
                Ok(Selected {
                    repo,
                    number,
                    change_id: Some(id),
                    placement: Some(placement),
                    parent_number: Some(project.number),
                    project: Some(project),
                })
            }
        }
    }

    /// Discover readable Changes without fetching diffs or installing a snapshot.
    /// Only bare project Trail addresses need discovery. Explicit repository
    /// URLs retain their repository-only permission path through `snapshot`.
    pub fn choices(&self, a: &TrailAddress, cancel: Cancellation) -> Result<TrailChoices> {
        let TrailSelection::Project {
            number,
            change: None,
        } = a.selection
        else {
            return Err("Change discovery requires a bare Entire project Trail URL".into());
        };
        let login = self.login(cancel.clone())?;
        let account = self.account(&login, cancel.clone())?;
        let (project, detail) = self.project(&login, a, number, cancel.clone())?;
        let choices = TrailChoices::read(a.clone(), account.clone(), project, &detail)?;
        let still = self.login(cancel.clone())?;
        if self.account(&still, cancel.clone())? != account {
            return Err("The Entire account changed during Change discovery".into());
        }
        if cancel.cancelled() {
            return Err("Entire Change discovery was cancelled".into());
        }
        Ok(choices)
    }

    /// Open the exact discovery-time choice, not a reconstructed URL. Account,
    /// project, Change and placement identity are checked before diff reads and
    /// again by the normal immutable snapshot capture path.
    pub fn snapshot_choice(
        &self,
        choices: &TrailChoices,
        index: usize,
        cancel: Cancellation,
    ) -> Result<Snapshot> {
        let choice = choices.choice(index)?;
        let login = self.login(cancel.clone())?;
        let account = self.account(&login, cancel.clone())?;
        if account != choices.account {
            return Err(
                "The Entire account changed since Change discovery; reopen the Trail".into(),
            );
        }
        let a = &choices.address;
        let (project, detail) = self.project(&login, a, choices.project.number, cancel.clone())?;
        if project != choices.project {
            return Err("Entire project identity or routing changed since Change discovery; reopen the Trail".into());
        }
        // Parse every row to reject duplicate IDs/keys before selecting one.
        let current = TrailChoices::read(a.clone(), account.clone(), project.clone(), &detail)?;
        let wanted = choice.key();
        let found = current
            .changes()
            .iter()
            .find(|c| c.key() == wanted)
            .ok_or("The selected Entire Change is no longer readable; reopen the Trail")?;
        if found != choice {
            return Err("The selected Entire Change identity changed; reopen the Trail".into());
        }
        let selected = Selected {
            repo: wanted.0.clone(),
            number: wanted.1,
            change_id: Some(choice.id.clone()),
            placement: Some(choice.placement.clone()),
            parent_number: Some(project.number),
            project: Some(project),
        };
        let mut explicit = a.clone();
        explicit.selection = TrailSelection::Project {
            number: choices.project.number,
            change: Some(wanted),
        };
        self.snapshot_selected(&explicit, &login, account, selected, cancel)
    }

    pub fn snapshot(&self, a: &TrailAddress, cancel: Cancellation) -> Result<Snapshot> {
        let login = self.login(cancel.clone())?;
        let account = self.account(&login, cancel.clone())?;
        let selected = self.select(&login, a, cancel.clone())?;
        self.snapshot_selected(a, &login, account, selected, cancel)
    }

    fn snapshot_selected(
        &self,
        a: &TrailAddress,
        login: &Login,
        account: String,
        selected: Selected,
        cancel: Cancellation,
    ) -> Result<Snapshot> {
        let repository = self.repository(login, a, &selected.repo, cancel.clone())?;
        if selected
            .placement
            .as_ref()
            .is_some_and(|p| p != &repository.placement)
        {
            return Err("Entire's project Change and repository directory disagree on the repository placement".into());
        }
        let endpoint = format!(
            "/api/v1/repos/{}/trails/{}",
            repository.placement, selected.number
        );
        for _ in 0..3 {
            let before = self.json(login, Some(&repository.origin), &endpoint, cancel.clone())?;
            let pins = Pins::read(&before, &selected, a)?;
            let compare_endpoint = query(
                &format!("/api/v1/git/repos/{}/compare", repository.placement),
                &[
                    ("base", &pins.merge_base),
                    ("head", &pins.head),
                    ("includeStats", "true"),
                ],
            );
            let compare = self.json(
                login,
                Some(&repository.origin),
                &compare_endpoint,
                cancel.clone(),
            )?;
            let coverage: Comparison = serde_json::from_value(compare).map_err(|_| {
                AdapterError::Message("Entire comparison metadata has an unsupported shape".into())
            })?;
            coverage.validate()?;
            let diff_endpoint = query(
                &format!("/api/v1/git/repos/{}/diff", repository.placement),
                &[("base", &pins.merge_base), ("head", &pins.head)],
            );
            let raw = self.request(
                login,
                Some(&repository.origin),
                &diff_endpoint,
                cancel.clone(),
            )?;
            let patch = parse_patch(&raw, ParseLimits::default())?;
            if patch.files.is_empty() && (!coverage.files.is_empty() || coverage.truncated) {
                return Err("Entire returned an empty diff for an incomplete or nonempty comparison; no snapshot was installed".into());
            }
            let sessions = self.json(
                login,
                Some(&repository.origin),
                &query(
                    &format!("/api/v1/repos/{}/sessions", repository.placement),
                    &[
                        ("base", &pins.merge_base),
                        ("head", &pins.head),
                        ("limit", "10"),
                    ],
                ),
                cancel.clone(),
            );
            // Optional context must never hide a user's cancellation. Other
            // failures leave an explicit unavailable notice and a usable diff.
            if cancel.cancelled() {
                return Err("Entire capture was cancelled".into());
            }
            let after = self.json(login, Some(&repository.origin), &endpoint, cancel.clone())?;
            if Pins::read(&after, &selected, a)? != pins {
                continue;
            }
            if self.repository(login, a, &selected.repo, cancel.clone())? != repository {
                return Err("Entire repository identity or primary placement changed during capture; reopen the Trail".into());
            }
            if let Some(project) = &selected.project {
                let current = self.json(
                    login,
                    Some(&project.origin),
                    &project.endpoint,
                    cancel.clone(),
                )?;
                if current["id"] != project.id || current["projectId"] != project.project_id {
                    return Err("Entire project Trail identity changed during capture".into());
                }
                // Discovery choices must remain unambiguous at the final read,
                // including duplicate opaque IDs under different display keys.
                TrailChoices::read(a.clone(), account.clone(), project.clone(), &current)?;
                let wanted = match &a.selection {
                    TrailSelection::Project { change, .. } => change.as_ref(),
                    _ => None,
                };
                let (repo, number, id, placement) = select_project_change(a, &current, wanted)?;
                if !repo.eq_ignore_ascii_case(&selected.repo)
                    || number != selected.number
                    || Some(&id) != selected.change_id.as_ref()
                    || Some(&placement) != selected.placement.as_ref()
                {
                    return Err(
                        "Entire project Trail changed its selected Change during capture".into(),
                    );
                }
            }
            // Re-read the acting login separately: don't install a snapshot for
            // an account the user switched away from while it was loading.
            let still = self.login(cancel.clone())?;
            if self.account(&still, cancel.clone())? != account {
                return Err("The Entire account changed while the snapshot was captured".into());
            }
            let address = pins
                .parent
                .as_ref()
                .map(|parent| {
                    format!(
                        "https://entire.com/{}/{}/-/trails/{}/changes/{}/{}",
                        parent.forge,
                        parent.project,
                        parent.number,
                        repository.name,
                        selected.number,
                    )
                })
                .unwrap_or_else(|| a.change_url(&repository.name, selected.number));
            let entire = EntireTarget {
                forge: a.forge.clone(),
                repository_id: repository.id.clone(),
                placement_id: repository.placement.clone(),
                change_id: pins.id.clone(),
                api_origin: repository.origin.clone(),
                address,
                project_trail_id: pins.parent.as_ref().map(|p| p.id.clone()),
            };
            let origin = format!(
                "entire:{}:{}:{}:{}",
                a.forge, entire.repository_id, entire.placement_id, entire.change_id
            );
            let remote = RemoteTarget {
                provider: ProviderId::ENTIRE,
                repository: RepositoryKey {
                    host: "entire.com".into(),
                    id: 0,
                    owner: a.project.clone(),
                    name: repository.name.clone(),
                },
                account,
                pr: selected.number,
                target_tip: pins.base.clone(),
                comparison_base: pins.merge_base.clone(),
                head: pins.head.clone(),
                open: matches!(pins.status.as_str(), "open" | "draft"),
                draft: pins.status == "draft" || after["ready_for_review"] == false,
                merged: pins.status == "merged",
                pending_review: false,
                compare: None,
                entire: Some(entire),
            };
            let mut snapshot = Snapshot::with_origin(
                format!(
                    "{} · {}/{}/{} Change #{}",
                    text(&after, "/title")?,
                    a.forge,
                    a.project,
                    repository.name,
                    selected.number
                ),
                patch,
                Some(remote),
                vec![],
                origin,
            );
            snapshot.warnings = coverage.warnings(&snapshot.patch);
            snapshot.overview.description = after
                .pointer("/body_document/text_snapshot")
                .and_then(Value::as_str)
                .map(Into::into);
            snapshot.overview.author = after
                .pointer("/author/login")
                .and_then(Value::as_str)
                .map(Into::into);
            snapshot.overview.captured_at = Some(
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_secs(),
            );
            crate::entire_context::apply(
                &mut snapshot.overview,
                &after["mergeability"],
                &pins.head,
            );
            match sessions {
                Ok(response) => crate::entire_sessions::apply(&mut snapshot.overview, &response),
                Err(_) => crate::entire_sessions::unavailable(&mut snapshot.overview),
            }
            snapshot.overview.notices.push(READ_ONLY.into());
            snapshot.overview.notices.push(
                "Entire findings, discussions, and full session transcripts are not imported yet."
                    .into(),
            );
            return Ok(snapshot);
        }
        Err("The Entire Change moved throughout capture; the existing session and its drafts are unchanged".into())
    }

    pub fn source(&self, target: &RemoteTarget, path: &str, revision: &str) -> Result<Vec<u8>> {
        if target.provider != ProviderId::ENTIRE {
            return Err("source target is not Entire".into());
        }
        let entire = target
            .entire
            .as_ref()
            .ok_or("Entire source identity is missing; reopen the Trail")?;
        let origin = api_origin(&entire.api_origin)?;
        if !placement_id(&entire.placement_id)
            || !commit_id(revision)
            || (revision != target.head && revision != target.comparison_base)
        {
            return Err("Entire source must use this snapshot's exact immutable revision".into());
        }
        RepoPath::new(path.as_bytes().to_vec())?;
        if path.contains('\\') {
            return Err("backslashes in Entire source paths are not supported".into());
        }
        let curl = self
            .curl
            .as_ref()
            .ok_or("Install curl to read byte-exact Entire source context")?;
        let cancel = Cancellation::default();
        let login = self.login(cancel.clone())?;
        if self.account(&login, cancel.clone())? != target.account {
            return Err(
                "The selected Entire account differs from this snapshot; reopen the Trail".into(),
            );
        }
        let endpoint = query(
            &format!(
                "/api/v1/git/repos/{}/raw/{}",
                entire.placement_id,
                encode(path)
            ),
            &[("ref", revision)],
        );
        let url = format!("{origin}{endpoint}");
        let mut request = ProcessRequest::new(curl.clone()).args([
            "--disable",
            "--silent",
            "--show-error",
            "--globoff",
            "--path-as-is",
            "--proto",
            "=https",
            "--proto-redir",
            "=https",
            "--max-redirs",
            "0",
            "--connect-timeout",
            "10",
            "--max-time",
            "60",
            "--max-filesize",
            "33554432",
            "--request",
            "GET",
            "--header",
            "Accept: application/octet-stream",
            "--config",
            "-",
            "--url",
            &url,
            "--write-out",
            "\nDIFFZ_ENTIRE_HTTP:%{http_code}",
        ]);
        // --disable is first, so user curlrc cannot enable redirects, tracing,
        // output files, or another URL. No token is passed in argv or a file.
        request.stdin =
            format!("header = \"Authorization: Bearer {}\"\n", login.token).into_bytes();
        request.deadline = Duration::from_secs(65);
        request.stdout_limit = MAX_RESPONSE + CURL_STATUS.len() + 3;
        let output = Runner::run(request, cancel)?;
        if !output.status.success() {
            return Err(
                "Entire raw source read failed; check access and network connectivity".into(),
            );
        }
        raw_body(output.stdout)
    }
}

// The directory has returned both owner/repo and forge/owner/repo. A qualified
// name proves its namespace, but a conflicting explicit provider is never
// ignored. An unqualified name cannot distinguish same-named et and gh repos.
fn directory_name<'a>(
    row: &'a Value,
    forge: &str,
    project: &str,
    repo: &str,
    provider: &str,
) -> Option<&'a str> {
    let parts: Vec<_> = row["full_name"].as_str()?.split('/').collect();
    if row
        .get("provider")
        .is_some_and(|v| v.as_str() != Some(provider))
    {
        return None;
    }
    let (owner, name) = match parts.as_slice() {
        [owner, name] if row["provider"] == provider => (*owner, *name),
        [prefix, owner, name] if *prefix == forge => (*owner, *name),
        _ => return None,
    };
    (owner.eq_ignore_ascii_case(project) && name.eq_ignore_ascii_case(repo)).then_some(name)
}

fn select_project_change(
    a: &TrailAddress,
    detail: &Value,
    wanted: Option<&(String, u64)>,
) -> Result<(String, u64, String, String)> {
    let changes = detail["changes"]
        .as_array()
        .ok_or("Entire project Trail did not provide its readable Changes")?;
    if changes.len() > 10_000 {
        return Err("Entire project Trail exceeds the Change safety limit".into());
    }
    // v0.12.2's ProjectTrailResource uses bool + omitempty: absence is its
    // documented false value, while an unrecognized value is never complete.
    let possibly_partial = match detail.get("isPossiblyPartial") {
        None | Some(Value::Bool(false)) => false,
        Some(Value::Bool(true)) => true,
        Some(_) => {
            return Err("Entire project Trail has an invalid access-filtering marker".into());
        }
    };
    if wanted.is_none() && (changes.len() != 1 || possibly_partial) {
        let urls: Vec<_> = changes
            .iter()
            .filter_map(|c| {
                let repo = c["repository"].as_str().filter(|r| repo_name(r))?;
                let number = c["number"]
                    .as_u64()
                    .filter(|n| *n > 0 && *n < 10_000_000_000)?;
                Some(a.change_url(repo, number))
            })
            .take(10)
            .collect();
        return Err(format!(
            "This Entire Trail has multiple, missing, or potentially access-filtered Changes. Open an explicit Change URL{}",
            if urls.is_empty() { ".".into() } else { format!(": {}", urls.join(" ; ")) }
        ).into());
    }
    let candidates: Vec<_> = changes
        .iter()
        .filter(|c| {
            wanted.is_none_or(|(repo, number)| {
                c["repository"]
                    .as_str()
                    .is_some_and(|r| r.eq_ignore_ascii_case(repo))
                    && c["number"].as_u64() == Some(*number)
            })
        })
        .collect();
    if candidates.len() != 1 {
        return Err("This project Trail does not expose exactly one matching Change; open the Change from Entire".into());
    }
    let c = candidates[0];
    let repo = text(c, "/repository")?;
    let id = text(c, "/id")?;
    let placement = text(c, "/repositoryId")?;
    let number = c["number"]
        .as_u64()
        .filter(|n| *n > 0 && *n < 10_000_000_000)
        .ok_or("Entire Change has no valid number")?;
    if !repo_name(repo) || !opaque_id(id) || !placement_id(placement) {
        return Err("Entire Change has an unsupported repository or Change identity".into());
    }
    Ok((repo.into(), number, id.into(), placement.into()))
}

#[derive(Debug, PartialEq, Eq)]
struct Parent {
    id: String,
    project_id: String,
    forge: String,
    project: String,
    number: u64,
}
impl Parent {
    fn read(v: &Value, selected: &Selected, address: &TrailAddress) -> Result<Option<Self>> {
        let Some(parent) = v.get("parent").filter(|p| !p.is_null()) else {
            if selected.parent_number.is_some() {
                return Err("Entire Change does not expose the parent needed to verify this project Trail URL; open its repository-scoped URL instead".into());
            }
            // Absence does not mean parentless: a repository-only reader may
            // not discover its project. Preserve a valid legacy entry point.
            return Ok(None);
        };
        let id = text(parent, "/id")?;
        let project_id = text(parent, "/project_id")?;
        let forge = text(parent, "/host")?;
        let project = text(parent, "/project")?;
        let canonical_project = project.to_ascii_lowercase();
        let number = parent["number"]
            .as_u64()
            .filter(|n| *n > 0 && *n < 10_000_000_000)
            .ok_or("Entire Change has an invalid project Trail number")?;
        if !placement_id(id)
            || !placement_id(project_id)
            || forge != address.forge
            || !project.eq_ignore_ascii_case(&address.project)
            || !project_name(project)
            || text(parent, "/path")? != format!("/api/v1/{forge}/{canonical_project}/trails/{id}")
        {
            return Err("Entire Change has inconsistent project parent identity".into());
        }
        if selected
            .parent_number
            .is_some_and(|expected| expected != number)
        {
            return Err("Entire Change has a different project Trail number".into());
        }
        if let Some(expected) = &selected.project
            && (id != expected.id
                || project_id != expected.project_id
                || forge != expected.forge
                || !project.eq_ignore_ascii_case(&expected.name)
                || number != expected.number)
        {
            return Err(
                "Entire Change does not confirm its selected project parent identity".into(),
            );
        }
        Ok(Some(Self {
            id: id.into(),
            project_id: project_id.into(),
            forge: forge.into(),
            project: canonical_project,
            number,
        }))
    }
}

#[derive(Debug, PartialEq, Eq)]
struct Pins {
    id: String,
    base: String,
    head: String,
    merge_base: String,
    status: String,
    parent: Option<Parent>,
}
impl Pins {
    fn read(v: &Value, selected: &Selected, address: &TrailAddress) -> Result<Self> {
        let id = text(v, "/id")?;
        if !opaque_id(id)
            || v["number"].as_u64() != Some(selected.number)
            || selected
                .change_id
                .as_ref()
                .is_some_and(|expected| expected != id)
        {
            return Err("Entire returned a different Change identity".into());
        }
        let parent = Parent::read(v, selected, address)?;
        let base = text(v, "/base_sha")?;
        let head = text(v, "/head_sha")?;
        let merge_base = text(v, "/merge_base_sha")?;
        if ![base, head, merge_base].into_iter().all(commit_id)
            || base.len() != head.len()
            || base.len() != merge_base.len()
        {
            return Err("Entire has not supplied a complete immutable Change snapshot; retry after indexing completes".into());
        }
        let status = text(v, "/status")?;
        if !matches!(status, "open" | "draft" | "closed" | "merged") {
            return Err("Entire returned an unsupported Change lifecycle status".into());
        }
        Ok(Self {
            id: id.into(),
            base: base.into(),
            head: head.into(),
            merge_base: merge_base.into(),
            status: status.into(),
            parent,
        })
    }
}

#[derive(Deserialize)]
struct Comparison {
    #[serde(deserialize_with = "nullable_files")]
    files: Vec<ChangedFile>,
    truncated: bool,
}
fn nullable_files<'de, D: serde::Deserializer<'de>>(
    d: D,
) -> std::result::Result<Vec<ChangedFile>, D::Error> {
    Ok(Option::<Vec<ChangedFile>>::deserialize(d)?.unwrap_or_default())
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ChangedFile {
    path: String,
    #[serde(default)]
    previous_path: Option<String>,
    status: String,
    #[serde(default)]
    binary: bool,
    #[serde(default)]
    too_large: bool,
}
impl Comparison {
    fn validate(&self) -> Result<()> {
        if self.files.len() > 10_000 {
            return Err("Entire comparison exceeds the file safety limit".into());
        }
        let mut names = BTreeSet::new();
        for file in &self.files {
            RepoPath::new(file.path.as_bytes().to_vec())?;
            if !names.insert(&file.path) {
                return Err("Entire comparison repeats a file path".into());
            }
            if !matches!(
                file.status.as_str(),
                "added" | "modified" | "removed" | "renamed"
            ) {
                return Err("Entire comparison contains an unsupported file status".into());
            }
            if file.status == "renamed" {
                RepoPath::new(
                    file.previous_path
                        .as_ref()
                        .ok_or("Entire rename has no previousPath")?
                        .as_bytes()
                        .to_vec(),
                )?;
            }
        }
        Ok(())
    }
    fn warnings(&self, patch: &PatchReport) -> Vec<String> {
        let mut warnings = vec![];
        if self.truncated {
            warnings.push("Entire truncated its comparison metadata; this snapshot cannot claim complete file coverage.".into());
        }
        let expected: BTreeSet<_> = self.files.iter().map(|f| f.path.as_str()).collect();
        let actual: BTreeSet<_> = patch
            .files
            .iter()
            .filter_map(|f| f.path().utf8().ok())
            .collect();
        if expected != actual {
            warnings.push(format!("Entire file coverage disagrees: {} listed files, {} files in the diff. Some changed files may be unavailable.", expected.len(), actual.len()));
        }
        for file in &self.files {
            let path = RepoPath::new(file.path.as_bytes().to_vec())
                .expect("validated path")
                .display();
            if file.too_large {
                warnings.push(format!("Entire omitted size-limited comparison data for {path}; its full changes are not promised."));
            }
            if file.binary
                && !patch
                    .files
                    .iter()
                    .any(|p| p.path().utf8().ok() == Some(&file.path))
            {
                warnings.push(format!(
                    "Entire lists a binary change absent from the diff: {path}"
                ));
            }
        }
        warnings
    }
}

fn raw_body(mut bytes: Vec<u8>) -> Result<Vec<u8>> {
    let suffix = CURL_STATUS.len() + 3;
    if bytes.len() < suffix {
        return Err("Entire raw source returned no HTTP status".into());
    }
    let start = bytes.len() - suffix;
    if &bytes[start..start + CURL_STATUS.len()] != CURL_STATUS {
        return Err("Entire raw source returned an incomplete HTTP response".into());
    }
    if &bytes[bytes.len() - 3..] != b"200" {
        return Err("Entire raw source was unavailable at the captured revision; reopen the Trail or check repository access".into());
    }
    bytes.truncate(start);
    if bytes.len() > MAX_RESPONSE {
        return Err("Entire source exceeds the 32 MiB safety limit".into());
    }
    Ok(bytes)
}

pub struct EntireRules;
impl ReviewRules for EntireRules {
    fn id(&self) -> ProviderId {
        ProviderId::ENTIRE
    }
    fn name(&self) -> &str {
        "Entire"
    }
    fn open_label(&self) -> &str {
        "Entire"
    }
    fn address_label(&self) -> &str {
        "Entire Trail or Change URL"
    }
    fn address_hint(&self) -> &str {
        "https://entire.com/et/project/-/trails/42/changes/repo/7"
    }
    fn address_help(&self) -> &str {
        "Open a Change URL, or a Trail with one complete readable Change. Sign in with entire login."
    }
    fn write_flag(&self) -> &str {
        ""
    }
    fn read_only_reason(&self) -> Option<&str> {
        Some(READ_ONLY)
    }
    fn source_link_unavailable_reason(&self) -> Option<&str> {
        Some(SOURCE_LINK_UNAVAILABLE)
    }
    fn reopen_address(&self, target: &RemoteTarget) -> String {
        target
            .entire
            .as_ref()
            .and_then(|e| TrailAddress::parse(&e.address).ok())
            .map(|a| a.canonical_url())
            .unwrap_or_default()
    }
    fn line_url(&self, _target: &RemoteTarget, _path: &str, _revision: &str, _line: u32) -> String {
        // The source-link entry point checks source_link_unavailable_reason.
        // No guessed web revision selector or line anchor is returned.
        String::new()
    }
    fn supports(&self, _verdict: Verdict) -> bool {
        false
    }
    fn payload(&self, _review: &PreparedReview) -> Box<RawValue> {
        RawValue::from_string("null".into()).expect("literal JSON")
    }
}

pub struct EntireProvider {
    reader: Option<Arc<EntireReader>>,
}
impl EntireProvider {
    pub fn new(reader: Option<Arc<EntireReader>>) -> Self {
        Self { reader }
    }
    fn reader(&self) -> Result<&Arc<EntireReader>> {
        self.reader
            .as_ref()
            .ok_or_else(|| "Install Entire CLI 0.12.2 or newer, then run entire login".into())
    }
}
impl ReviewProvider for EntireProvider {
    fn rules(&self) -> Arc<dyn ReviewRules> {
        Arc::new(EntireRules)
    }
    fn accepts(&self, address: &str) -> bool {
        TrailAddress::parse(address).is_ok()
    }
    fn open(&self, address: &str, cancel: Cancellation) -> Result<Snapshot> {
        self.reader()?
            .snapshot(&TrailAddress::parse(address)?, cancel)
    }
    fn open_or_select(&self, address: &str, cancel: Cancellation) -> Result<ProviderOpen> {
        let address = TrailAddress::parse(address)?;
        if !matches!(
            address.selection,
            TrailSelection::Project { change: None, .. }
        ) {
            return self
                .reader()?
                .snapshot(&address, cancel)
                .map(Box::new)
                .map(ProviderOpen::Snapshot);
        }
        let reader = self.reader()?.clone();
        let choices = reader.choices(&address, cancel.clone())?;
        if choices.changes().len() == 1 && !choices.possibly_partial() {
            return reader
                .snapshot_choice(&choices, 0, cancel)
                .map(Box::new)
                .map(ProviderOpen::Snapshot);
        }
        let entries = choices
            .changes()
            .iter()
            .map(|choice| SourceChoice {
                label: format!(
                    "{} · {} · Change #{}",
                    choice.repository(),
                    choice.branch().unwrap_or("branch unavailable"),
                    choice.number()
                ),
                request: OpenRequest::Remote {
                    provider: ProviderId::ENTIRE,
                    address: choice.address().into(),
                },
            })
            .collect();
        Ok(ProviderOpen::Choices(Arc::new(EntireChoices {
            reader,
            choices,
            entries,
        })))
    }
    fn source(&self, target: &RemoteTarget, path: &str, revision: &str) -> Result<Vec<u8>> {
        self.reader()?.source(target, path, revision)
    }
    fn remote(&self) -> Result<Arc<dyn ReviewRemote>> {
        Err(READ_ONLY.into())
    }
}

struct EntireChoices {
    reader: Arc<EntireReader>,
    choices: TrailChoices,
    entries: Vec<SourceChoice>,
}
impl std::fmt::Debug for EntireChoices {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("EntireChoices")
    }
}
impl ReviewChoices for EntireChoices {
    fn title(&self) -> &str {
        self.choices.title()
    }
    fn possibly_partial(&self) -> bool {
        self.choices.possibly_partial()
    }
    fn entries(&self) -> &[SourceChoice] {
        &self.entries
    }
    fn open(&self, index: usize, cancel: Cancellation) -> Result<Snapshot> {
        self.reader.snapshot_choice(&self.choices, index, cancel)
    }
}
