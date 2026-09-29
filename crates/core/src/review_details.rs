use crate::domain::{Side, ThreadComment};
use serde::{Deserialize, Serialize};
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Overview {
    pub description: Option<String>,
    /// Username of whoever created the merge or pull request.
    #[serde(default)]
    pub author: Option<String>,
    /// The review verdict when the snapshot was taken, if any reviewer ruled.
    #[serde(default)]
    pub decision: Option<ReviewDecision>,
    pub checks: Vec<Check>,
    pub notices: Vec<String>,
    pub captured_at: Option<u64>,
    pub conversation: Vec<ConversationComment>,
    /// A compare's release tags in range order, each with what changed since the one before.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub releases: Vec<Release>,
    /// Head-side lines per path, attributed to the releases that last changed them. Filled one
    /// file at a time as files are shown; `None` where the blame could not be read, which is
    /// kept for this session only, so a later one reads it again.
    #[serde(
        default,
        skip_serializing_if = "BlameRead::is_empty",
        serialize_with = "read_blame"
    )]
    pub blame: BlameRead,
}
/// Per path, the attributed head lines, or `None` where the blame could not be read.
pub type BlameRead = std::collections::BTreeMap<String, Option<Vec<Blamed>>>;
fn read_blame<S: serde::Serializer>(blame: &BlameRead, s: S) -> Result<S::Ok, S::Error> {
    s.collect_map(blame.iter().filter(|(_, b)| b.is_some()))
}
/// One step of a compare: from the previous release (or the merge base) to `commit`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Release {
    /// `None` for the untagged commits after the last tag, up to the compare's head.
    pub tag: Option<String>,
    pub commit: String,
    /// Commit date of `commit`, RFC 3339.
    pub date: Option<String>,
    pub commits: u64,
    pub files: Vec<ReleaseFile>,
    /// Body of the provider's release for this tag, if one exists.
    pub notes: Option<String>,
    pub url: Option<String>,
    /// Every commit this step adds, merged branches included, so blamed lines find their release.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub shas: Vec<String>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReleaseFile {
    pub path: String,
    pub additions: u64,
    pub deletions: u64,
    /// The path this release renamed the file from.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub previous: Option<String>,
}
/// Head lines `start..=end`, last changed by `commit`, which release `release` added.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Blamed {
    pub start: u32,
    pub end: u32,
    pub release: usize,
    pub commit: String,
}
impl Release {
    /// The tag, or `head` for the untagged tail.
    pub fn name<'a>(&'a self, head: &'a str) -> &'a str {
        self.tag.as_deref().unwrap_or(head)
    }
}
/// For each path at the head, the indexes of the releases that touched it, oldest first.
/// A file renamed inside the range is found under its head path in the releases before the rename.
pub fn releases_by_path(releases: &[Release]) -> std::collections::HashMap<&str, Vec<usize>> {
    let mut paths: std::collections::HashMap<&str, Vec<usize>> = Default::default();
    // Newest first, each earlier name maps to the name the file has at the head.
    let mut renamed: std::collections::HashMap<&str, &str> = Default::default();
    for (i, r) in releases.iter().enumerate().rev() {
        for f in &r.files {
            let head = renamed.get(f.path.as_str()).copied().unwrap_or(&f.path);
            paths.entry(head).or_default().push(i);
            if let Some(previous) = &f.previous {
                renamed.insert(previous, head);
            }
        }
    }
    for touched in paths.values_mut() {
        touched.sort_unstable();
        touched.dedup();
    }
    paths
}
/// Keeps the blamed `(first line, last line, commit)` ranges whose commit one of `releases`
/// added. Lines last changed before the range, the unchanged context, are left out.
pub fn attribute(releases: &[Release], ranges: &[(u32, u32, String)]) -> Vec<Blamed> {
    let release: std::collections::HashMap<&str, usize> = releases
        .iter()
        .enumerate()
        .flat_map(|(i, r)| r.shas.iter().map(move |sha| (sha.as_str(), i)))
        .collect();
    let mut found: Vec<Blamed> = ranges
        .iter()
        .filter_map(|(start, end, commit)| {
            Some(Blamed {
                start: *start,
                end: *end,
                release: *release.get(commit.as_str())?,
                commit: commit.clone(),
            })
        })
        .collect();
    found.sort_unstable_by_key(|b| b.start);
    found
}
/// Opens the one warning that names every file whose blame could not be read.
pub const UNBLAMED: &str = "Release attribution is unavailable for";
/// Head lines `first..=last` around `file`'s added lines, while a compare of two releases or
/// more has not yet read that file's blame. Files without added lines need none, and neither
/// do releases saved before they carried their commits, since no line could be attributed.
pub fn blame_span(overview: &Overview, file: &crate::patch::FileChange) -> Option<(u32, u32)> {
    if overview.releases.len() < 2
        || overview.releases.iter().all(|r| r.shas.is_empty())
        || overview.blame.contains_key(&file.display_path())
    {
        return None;
    }
    file.hunks
        .iter()
        .flat_map(|h| &h.rows)
        .filter(|r| r.kind == crate::patch::RowKind::Added)
        .filter_map(|r| r.new_line)
        .fold(None, |span, n| {
            Some(span.map_or((n, n), |(first, last): (u32, u32)| {
                (first.min(n), last.max(n))
            }))
        })
}
/// `from` with `found` laid over its blame. It copies the snapshot, so it belongs off the UI thread.
pub fn with_blame(from: &crate::domain::Snapshot, found: BlameRead) -> crate::domain::Snapshot {
    let mut s = from.clone();
    s.overview.blame.extend(found);
    s
}
/// Whether blame read from `from` may still be laid over `current`: only while `current` is
/// `from` itself. A reload replaces it even under the same id, and may number the releases anew.
pub fn blame_applies(
    current: &std::sync::Arc<crate::domain::Snapshot>,
    from: &std::sync::Arc<crate::domain::Snapshot>,
) -> bool {
    std::sync::Arc::ptr_eq(current, from)
}
impl Overview {
    /// Names the files whose blame could not be read, if any.
    pub fn unblamed_warning(&self) -> Option<String> {
        let failed: Vec<&str> = self
            .blame
            .iter()
            .filter(|(_, b)| b.is_none())
            .map(|(path, _)| path.as_str())
            .collect();
        let shown = failed
            .iter()
            .take(3)
            .copied()
            .collect::<Vec<_>>()
            .join(", ");
        match failed.len() {
            0 => None,
            1 => Some(format!(
                "{UNBLAMED} {shown}: its blame could not be read, so its lines show no release."
            )),
            n => Some(format!(
                "{UNBLAMED} {n} files whose blame could not be read, so their lines show no release: {shown}{}.",
                if n > 3 { ", …" } else { "" }
            )),
        }
    }
    /// The blamed range holding head line `line` of `path`, once that file's blame is read.
    pub fn blamed(&self, path: &str, line: u32) -> Option<&Blamed> {
        let ranges = self.blame.get(path)?.as_deref()?;
        let at = ranges.partition_point(|b| b.end < line);
        ranges.get(at).filter(|b| b.start <= line)
    }
    /// The release an added row of `path` came from. Removed and unchanged rows have none:
    /// blame at the head cannot see a removal, and context predates the range.
    pub fn row_release(&self, path: &str, row: &crate::patch::PatchRow) -> Option<&Blamed> {
        if row.kind != crate::patch::RowKind::Added {
            return None;
        }
        self.blamed(path, row.new_line?)
    }
}
/// What can be said of a removed line, given the names of the releases that touched its file.
pub fn removed_in(names: &[&str]) -> String {
    match names {
        [] => "Removed in this range".into(),
        [one] => format!("Removed in {one}"),
        [first, .., last] if names.len() > 4 => format!(
            "Removed in one of {} releases, {first} to {last}",
            names.len()
        ),
        many => format!("Removed in one of: {}", many.join(", ")),
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ReviewDecision {
    Approved,
    ChangesRequested,
}
impl ReviewDecision {
    pub fn label(self) -> &'static str {
        match self {
            ReviewDecision::Approved => "approved",
            ReviewDecision::ChangesRequested => "changes requested",
        }
    }
}
/// Computes a verdict from provider events reported as `(reviewer, state)` pairs in arrival order.
/// Each reviewer contributes only their latest APPROVED, CHANGES_REQUESTED, or DISMISSED state.
/// COMMENTED and PENDING are ignored; one active change request outweighs every approval.
pub fn review_decision<'a>(
    reviews: impl IntoIterator<Item = (&'a str, &'a str)>,
) -> Option<ReviewDecision> {
    let mut latest = std::collections::BTreeMap::new();
    for (user, state) in reviews {
        if matches!(state, "APPROVED" | "CHANGES_REQUESTED" | "DISMISSED") {
            latest.insert(user, state);
        }
    }
    if latest.values().any(|s| *s == "CHANGES_REQUESTED") {
        Some(ReviewDecision::ChangesRequested)
    } else if latest.values().any(|s| *s == "APPROVED") {
        Some(ReviewDecision::Approved)
    } else {
        None
    }
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Check {
    pub name: String,
    pub kind: String,
    pub status: String,
    pub conclusion: Option<String>,
    pub url: Option<String>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConversationComment {
    pub id: u64,
    pub author: String,
    pub body: String,
    /// Creation timestamp from the provider, RFC 3339. Snapshots made before this field existed leave it empty.
    #[serde(default)]
    pub created_at: Option<String>,
}
/// Converts `2026-09-04T14:22:31Z` to `2026-09-04 14:22` (UTC). Other input is returned unchanged.
pub fn short_timestamp(rfc3339: &str) -> String {
    let b = rfc3339.as_bytes();
    if b.len() >= 16 && b[10] == b'T' && b[4] == b'-' && b[13] == b':' {
        format!("{} {}", &rfc3339[..10], &rfc3339[11..16])
    } else {
        rfc3339.to_owned()
    }
}
pub fn threads_at(comments: &[ThreadComment], path: &str, side: Side, line: u32) -> Vec<u64> {
    let mut roots = Vec::new();
    for c in comments {
        if c.path == path
            && c.side == Some(side)
            && c.line
                .is_some_and(|end| (c.start_line.unwrap_or(end)..=end).contains(&line))
            && !roots.contains(&c.root_id)
        {
            roots.push(c.root_id);
        }
    }
    roots
}
/// Counts root threads with `id == root_id`. Comments whose `id != root_id` are replies and do not affect path totals.
pub fn thread_counts(comments: &[ThreadComment]) -> std::collections::BTreeMap<String, usize> {
    let mut counts = std::collections::BTreeMap::new();
    for c in comments {
        if c.id == c.root_id {
            *counts.entry(c.path.clone()).or_insert(0) += 1;
        }
    }
    counts
}
/// `(seen, total)`: `seen` is how many keys in `viewed` map to `true`.
pub fn reviewed_progress(
    viewed: &std::collections::BTreeMap<String, bool>,
    files: &[String],
) -> (usize, usize) {
    (
        files
            .iter()
            .filter(|f| viewed.get(f.as_str()).copied() == Some(true))
            .count(),
        files.len(),
    )
}
/// Use this field solely for display; retain the unmodified provider body separately for export and identity.
pub fn visible_markdown(body: &str) -> String {
    let mut out = String::new();
    let mut hidden = false;
    let mut fence: Option<&str> = None;
    for line in body.split_inclusive('\n') {
        let marker = if line.trim_start().starts_with("```") {
            Some("```")
        } else if line.trim_start().starts_with("~~~") {
            Some("~~~")
        } else {
            None
        };
        if !hidden && (fence.is_some() || marker.is_some()) {
            out.push_str(line);
            if let Some(m) = marker {
                if fence == Some(m) {
                    fence = None;
                } else if fence.is_none() {
                    fence = Some(m);
                }
            }
            continue;
        }
        let mut rest = line;
        while !rest.is_empty() {
            if hidden {
                if let Some(i) = rest.find("-->") {
                    rest = &rest[i + 3..];
                    hidden = false;
                } else {
                    break;
                }
            } else if rest.starts_with("<!--") {
                hidden = true;
                rest = &rest[4..];
            } else if rest
                .get(..4)
                .is_some_and(|s| s.eq_ignore_ascii_case("<img"))
            {
                if let Some(i) = rest.find('>') {
                    out.push_str("[image]");
                    rest = &rest[i + 1..];
                } else {
                    break;
                }
            } else {
                let c = rest.chars().next().unwrap();
                out.push(c);
                rest = &rest[c.len_utf8()..];
            }
        }
    }
    out.replace("![", "[image: ").trim().to_string()
}
pub use crate::scroll::BoundaryScroll;

pub fn thread_roots_at(
    snapshot: &crate::domain::Snapshot,
    point: &crate::domain::SourcePoint,
) -> Vec<u64> {
    let Some(file) = snapshot.file(&point.file) else {
        return vec![];
    };
    let path = file.display_path();
    let mut roots = threads_at(&snapshot.comments, &path, point.side, point.line);
    if let Some(row) = file
        .line(point.side, point.line)
        .filter(|r| r.kind == crate::patch::RowKind::Context)
    {
        let other = match point.side {
            Side::Left => row.new_line.map(|n| (Side::Right, n)),
            Side::Right => row.old_line.map(|n| (Side::Left, n)),
        };
        if let Some((side, line)) = other {
            for root in threads_at(&snapshot.comments, &path, side, line) {
                if !roots.contains(&root) {
                    roots.push(root);
                }
            }
        }
    }
    roots
}
