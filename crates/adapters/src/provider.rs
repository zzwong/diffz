//! Review providers. Each pairs a host's pure rules with the client that reads from it and
//! publishes to it; `Services` holds them in a list keyed by provider id.
use crate::Result;
use diffz_core::{
    domain::{RemoteTarget, Snapshot},
    provider::{Cancellation, ReviewRules},
    review::PreparedReview,
};
use serde_json::Value;
use std::sync::Arc;

pub trait ReviewProvider: Send + Sync {
    fn rules(&self) -> Arc<dyn ReviewRules>;
    /// Fetch an immutable snapshot for any address form this provider accepts.
    fn open(&self, address: &str, cancel: Cancellation) -> Result<Snapshot>;
    fn source(&self, target: &RemoteTarget, path: &str, revision: &str) -> Result<Vec<u8>>;
    /// Reads that confirm or reconcile a publication, and the one-shot send. `Services` only
    /// sends through it after the user opted in to writes for this provider.
    fn remote(&self) -> Result<Arc<dyn ReviewRemote>>;
}

pub enum SendOutcome {
    Accepted(Value),
    Rejected(u16),
    Unknown(String),
}

/// Remote reviews and comments come back normalized to one shape, the one GitHub's API uses,
/// so the outbox can match them without knowing the host: a review has `id`, `commit_id`,
/// `state`, `body` and `user.login`, plus `fingerprint` when the provider marks reviews; a
/// comment has `path`, `body`, `side`, `line` (or GitHub's `original_line`) and optionally
/// `start_line` and `start_side`.
pub trait ReviewRemote: Send + Sync {
    fn current(&self, t: &RemoteTarget) -> Result<RemoteTarget>;
    fn reviews(&self, t: &RemoteTarget) -> Result<Vec<Value>>;
    fn comments(&self, t: &RemoteTarget, id: u64) -> Result<Vec<Value>>;
    fn send(&self, p: &PreparedReview) -> SendOutcome;
}
