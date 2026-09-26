//! Review payloads are fixed, and the outbox lifecycle rejects unsafe states.
use crate::domain::*;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use thiserror::Error;
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Verdict {
    Approve,
    RequestChanges,
    Comment,
}
impl Verdict {
    pub fn api(self) -> &'static str {
        match self {
            Self::Approve => "APPROVE",
            Self::RequestChanges => "REQUEST_CHANGES",
            Self::Comment => "COMMENT",
        }
    }
    pub fn remote_state(self) -> &'static str {
        match self {
            Self::Approve => "APPROVED",
            Self::RequestChanges => "CHANGES_REQUESTED",
            Self::Comment => "COMMENTED",
        }
    }
}
#[derive(Debug, Clone, Error)]
#[error("review cannot be prepared: {0}")]
pub struct ReviewError(pub String);
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PreparedComment {
    pub draft: DraftId,
    pub version: u64,
    pub path: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gitlab_position: Option<GitlabPosition>,
    pub body: String,
    pub side: Side,
    pub start_line: u32,
    pub line: u32,
    /// A comment that covers the whole file, not a source line.
    /// GitHub publishes it using `"subject_type": "file"`, with no line keys;
    /// on GitLab it becomes a plain note on the MR, not a positioned discussion.
    #[serde(default)]
    pub file_level: bool,
}
/// Where a GitLab discussion attaches. Field order is part of the review fingerprint.
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
// The fingerprint hashes these payloads, so they are structs, never `json!` maps: a map's
// key order follows serde_json's `preserve_order` feature, which any dependency can switch
// on. Field order matches the order the desktop build has always produced.
#[derive(Serialize)]
#[serde(untagged)]
enum Payload<'a> {
    GitHub {
        commit_id: &'a str,
        event: &'static str,
        body: &'a str,
        comments: Vec<GithubComment<'a>>,
    },
    GitLab {
        head: &'a str,
        verdict: Verdict,
        summary: &'a str,
        comments: &'a [PreparedComment],
    },
}
#[derive(Serialize)]
#[serde(untagged)]
enum GithubComment<'a> {
    File {
        path: &'a str,
        body: &'a str,
        subject_type: &'static str,
    },
    Line {
        path: &'a str,
        body: &'a str,
        line: u32,
        side: &'static str,
        #[serde(skip_serializing_if = "Option::is_none")]
        start_line: Option<u32>,
        #[serde(skip_serializing_if = "Option::is_none")]
        start_side: Option<&'static str>,
    },
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PreparedReview {
    pub id: OperationId,
    pub snapshot: SnapshotId,
    pub target: RemoteTarget,
    pub verdict: Verdict,
    pub summary: String,
    pub comments: Vec<PreparedComment>,
    pub fingerprint: String,
}
impl PreparedReview {
    pub fn prepare(
        id: OperationId,
        s: &Snapshot,
        drafts: Vec<Draft>,
        verdict: Verdict,
        summary: String,
    ) -> Result<Self, ReviewError> {
        let fail = |m: &str| ReviewError(m.into());
        let target = s
            .remote
            .clone()
            .ok_or_else(|| fail("offline source; a GitHub review target is required"))?;
        if target.provider == ProviderId::GITLAB && verdict == Verdict::RequestChanges {
            return Err(fail(
                "GitLab permits comments and approval; blocking change requests remain unsupported",
            ));
        }
        if target.provider == ProviderId::GITLAB
            && (summary.lines().any(|l| l.trim_start().starts_with('/'))
                || drafts
                    .iter()
                    .any(|d| d.body.lines().any(|l| l.trim_start().starts_with('/'))))
        {
            return Err(fail(
                "GitLab quick actions are disallowed here; format inline slash-prefixed text before posting",
            ));
        }
        if !s.verify_identity() {
            return Err(fail("snapshot identity failed verification"));
        }
        if !s.warnings.is_empty() {
            return Err(fail(
                "coverage warnings mean the snapshot is incomplete; inspect diagnostics first",
            ));
        }
        if !target.open {
            return Err(fail("pull request is not open"));
        }
        if target.pending_review {
            return Err(fail(
                "another pending GitHub review exists; handle it separately",
            ));
        }
        if verdict != Verdict::Approve && summary.trim().is_empty() {
            return Err(fail("this verdict needs a summary"));
        }
        if summary.len() > 64 * 1024 || drafts.len() > 100 {
            return Err(fail("summary or batch is beyond safety limits"));
        }
        let mut comments = vec![];
        let mut ids = std::collections::HashSet::new();
        for d in drafts {
            let is_file_level = d.is_file_level();
            if !ids.insert(d.id.clone()) {
                return Err(fail("duplicate local draft"));
            }
            if d.snapshot != s.id || d.published {
                return Err(fail(
                    "draft targets another revision or has already been published",
                ));
            }
            if !d.is_saved() {
                return Err(fail("save every selected draft before continuing"));
            }
            if d.body.trim().is_empty() || d.body.len() > 64 * 1024 {
                return Err(fail("empty or oversized draft body"));
            }
            let f = s
                .file(&d.file)
                .ok_or_else(|| fail("draft file is absent from this snapshot"))?;
            if !d.is_file_level() && !f.eligible(d.side, d.start_line, d.line) {
                return Err(fail(
                    "comment range must remain inside one canonical provider hunk for posting",
                ));
            }
            let path = f.path().utf8().map_err(ReviewError)?.to_owned();
            let gitlab_position = if target.provider == ProviderId::GITLAB {
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
                let position = |position_type: &str| GitlabPosition {
                    position_type: position_type.into(),
                    base_sha: target.comparison_base.clone(),
                    start_sha: target.target_tip.clone(),
                    head_sha: target.head.clone(),
                    old_path: old.into(),
                    new_path: new.into(),
                    old_line: None,
                    new_line: None,
                };
                if d.is_file_level() {
                    Some(position("file"))
                } else {
                    if d.start_line != d.line {
                        return Err(fail(
                            "GitLab posting needs each draft to hold one source line for now",
                        ));
                    }
                    let row = f
                        .line(d.side, d.line)
                        .ok_or_else(|| fail("the draft holds no source line to post to GitLab"))?;
                    Some(GitlabPosition {
                        old_line: row.old_line,
                        new_line: row.new_line,
                        ..position("text")
                    })
                }
            } else {
                None
            };
            comments.push(PreparedComment {
                gitlab_position,
                draft: d.id,
                version: d.version,
                path,
                file_level: is_file_level,
                body: d.body,
                side: d.side,
                start_line: d.start_line,
                line: d.line,
            });
        }
        let mut p = Self {
            id,
            snapshot: s.id.clone(),
            target,
            verdict,
            summary,
            comments,
            fingerprint: String::new(),
        };
        p.fingerprint = p.compute_fingerprint();
        Ok(p)
    }
    pub fn payload(&self) -> Value {
        serde_json::to_value(self.payload_repr()).expect("plain serializable review payload")
    }
    fn payload_repr(&self) -> Payload<'_> {
        if self.target.provider == ProviderId::GITLAB {
            return Payload::GitLab {
                head: &self.target.head,
                verdict: self.verdict,
                summary: &self.summary,
                comments: &self.comments,
            };
        }
        let comments = self
            .comments
            .iter()
            .map(|c| {
                if c.file_level {
                    return GithubComment::File {
                        path: &c.path,
                        body: &c.body,
                        subject_type: "file",
                    };
                }
                let range = c.start_line < c.line;
                GithubComment::Line {
                    path: &c.path,
                    body: &c.body,
                    line: c.line,
                    side: c.side.api(),
                    start_line: range.then_some(c.start_line),
                    start_side: range.then(|| c.side.api()),
                }
            })
            .collect();
        Payload::GitHub {
            commit_id: &self.target.head,
            event: self.verdict.api(),
            body: &self.summary,
            comments,
        }
    }
    pub fn compute_fingerprint(&self) -> String {
        let payload = serde_json::to_vec(&(
            self.id.clone(),
            self.snapshot.clone(),
            self.target.clone(),
            self.payload_repr(),
            self.comments
                .iter()
                .map(|c| (&c.draft, c.version))
                .collect::<Vec<_>>(),
        ))
        .expect("plain serializable review payload");
        digest(&[b"review-v1", &payload])
    }
    pub fn verify(&self) -> bool {
        self.fingerprint == self.compute_fingerprint()
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum OutboxState {
    Prepared,
    InFlight,
    Confirmed,
    Rejected,
    UnknownOutcome,
}
impl OutboxState {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Prepared => "prepared",
            Self::InFlight => "in_flight",
            Self::Confirmed => "confirmed",
            Self::Rejected => "rejected",
            Self::UnknownOutcome => "unknown",
        }
    }
    pub fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "prepared" => Self::Prepared,
            "in_flight" => Self::InFlight,
            "confirmed" => Self::Confirmed,
            "rejected" => Self::Rejected,
            "unknown" => Self::UnknownOutcome,
            _ => return None,
        })
    }
    pub fn can_transition(self, to: Self) -> bool {
        matches!(
            (self, to),
            (Self::Prepared, Self::InFlight)
                | (Self::Prepared, Self::Rejected)
                | (Self::InFlight, Self::Confirmed)
                | (Self::InFlight, Self::Rejected)
                | (Self::InFlight, Self::UnknownOutcome)
                | (Self::UnknownOutcome, Self::Confirmed)
        )
    }
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OutboxEntry {
    pub prepared: PreparedReview,
    pub state: OutboxState,
    pub baseline_review_ids: Vec<u64>,
    pub remote_id: Option<u64>,
    pub diagnostic: Option<String>,
}
