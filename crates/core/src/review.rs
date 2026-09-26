//! Review payloads are fixed, and the outbox lifecycle rejects unsafe states.
use crate::domain::*;
use crate::provider::ReviewRules;
use serde::{Deserialize, Serialize};
use serde_json::value::RawValue;
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
    /// Raw JSON so its fingerprinted bytes survive storage exactly.
    #[serde(
        rename = "gitlab_position",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    pub position: Option<Box<RawValue>>,
    pub body: String,
    pub side: Side,
    pub start_line: u32,
    pub line: u32,
    #[serde(default)]
    pub file_level: bool,
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
        rules: &dyn ReviewRules,
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
            .ok_or_else(|| fail("offline source; a hosted review target is required"))?;
        if rules.id() != target.provider {
            return Err(fail("this review belongs to another provider"));
        }
        rules.check(verdict, &summary, &drafts)?;
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
            return Err(fail("another pending review exists; handle it separately"));
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
            let position = rules.position(&target, f, &d)?;
            comments.push(PreparedComment {
                position,
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
        p.fingerprint = p.compute_fingerprint(rules);
        Ok(p)
    }
    pub fn compute_fingerprint(&self, rules: &dyn ReviewRules) -> String {
        let payload = serde_json::to_vec(&(
            self.id.clone(),
            self.snapshot.clone(),
            self.target.clone(),
            rules.payload(self),
            self.comments
                .iter()
                .map(|c| (&c.draft, c.version))
                .collect::<Vec<_>>(),
        ))
        .expect("plain serializable review payload");
        digest(&[b"review-v1", &payload])
    }
    pub fn verify(&self, rules: &dyn ReviewRules) -> bool {
        rules.id() == self.target.provider && self.fingerprint == self.compute_fingerprint(rules)
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
