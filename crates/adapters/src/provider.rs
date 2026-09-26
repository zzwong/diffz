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
    fn open(&self, address: &str, cancel: Cancellation) -> Result<Snapshot>;
    fn source(&self, target: &RemoteTarget, path: &str, revision: &str) -> Result<Vec<u8>>;
    fn remote(&self) -> Result<Arc<dyn ReviewRemote>>;
}

pub enum SendOutcome {
    Accepted(Value),
    Rejected(u16),
    Unknown(String),
}

/// Reviews and comments are normalized to GitHub's API shape so the outbox can match them.
pub trait ReviewRemote: Send + Sync {
    fn current(&self, t: &RemoteTarget) -> Result<RemoteTarget>;
    fn reviews(&self, t: &RemoteTarget) -> Result<Vec<Value>>;
    fn comments(&self, t: &RemoteTarget, id: u64) -> Result<Vec<Value>>;
    fn send(&self, p: &PreparedReview) -> SendOutcome;
}
