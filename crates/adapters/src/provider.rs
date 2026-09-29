use crate::Result;
use diffz_core::{
    domain::{RemoteTarget, Snapshot},
    provider::{Cancellation, ReviewRules},
    review::PreparedReview,
    review_details::Release,
};
use serde_json::Value;
use std::sync::Arc;

pub trait ReviewProvider: Send + Sync {
    fn rules(&self) -> Arc<dyn ReviewRules>;
    fn open(&self, address: &str, cancel: Cancellation) -> Result<Snapshot>;
    /// Whether `open` would take this address. Only parses; nothing runs.
    fn accepts(&self, address: &str) -> bool;
    fn source(&self, target: &RemoteTarget, path: &str, revision: &str) -> Result<Vec<u8>>;
    fn remote(&self) -> Result<Arc<dyn ReviewRemote>>;
    /// A compare's releases and the warnings reading them raised; nothing for other targets.
    fn releases(
        &self,
        _target: &RemoteTarget,
        _cancel: Cancellation,
    ) -> Result<(Vec<Release>, Vec<String>)> {
        Ok((vec![], vec![]))
    }
    /// Who last changed lines `first..=last` of each path at the head, one entry per path.
    /// A path whose blame could not be read is `None`; the rest still count.
    fn blame(
        &self,
        _target: &RemoteTarget,
        _spans: &[Span],
        _cancel: Cancellation,
    ) -> Result<Vec<Option<Blame>>> {
        Err("This provider cannot read blame".into())
    }
}

/// A path and the head lines `first..=last` whose blame is wanted.
pub type Span = (String, u32, u32);
/// Blamed `(first line, last line, commit)` ranges, in line order.
pub type Blame = Vec<(u32, u32, String)>;

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
