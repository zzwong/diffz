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
