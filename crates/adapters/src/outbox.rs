//! Publication records durable outcomes conservatively. No state permits an automatic retry.
use crate::{
    Result,
    provider::{ReviewRemote, SendOutcome},
    store::Store,
};
use diffz_core::{domain::*, provider::ReviewRules, review::*};
use serde_json::Value;
use std::sync::{Arc, Mutex};

pub struct Outbox {
    store: Arc<Store>,
    rules: Arc<dyn ReviewRules>,
    remote: Arc<dyn ReviewRemote>,
    gate: Mutex<()>,
}
impl Outbox {
    pub fn new(
        store: Arc<Store>,
        rules: Arc<dyn ReviewRules>,
        remote: Arc<dyn ReviewRemote>,
    ) -> Self {
        Self {
            store,
            rules,
            remote,
            gate: Mutex::new(()),
        }
    }
    pub fn publish(&self, p: PreparedReview) -> Result<OutboxEntry> {
        let _guard = self.gate.lock().map_err(|_| "publication gate poisoned")?;
        let mut entry = self.store.operation(&p.id)?;
        if entry.state != OutboxState::Prepared
            || entry.prepared.fingerprint != p.fingerprint
            || !p.verify(&*self.rules)
        {
            return Err(
                "the operation differs from its prepared review; an uncertain send cannot be retried"
                    .into(),
            );
        }
        // A pending result for this PR and account also blocks a new UUID from sending again.
        if self.store.outbox()?.iter().any(|x| {
            x.prepared.id != p.id
                && same_pr(&x.prepared.target, &p.target)
                && matches!(x.state, OutboxState::UnknownOutcome | OutboxState::InFlight)
        }) {
            return Err("this PR already has an operation whose result is unresolved".into());
        }
        let current = self.remote.current(&p.target)?;
        if current != p.target || !current.open || current.pending_review {
            entry.state = OutboxState::Rejected;
            entry.diagnostic =
                Some("The PR revision, repository, account, or pending-review state no longer matches the preview.".into());
            self.store.transition(&entry, &*self.rules)?;
            return Ok(entry);
        }
        // Preview approval excludes later draft edits. Check the saved versions again before sending.
        let saved = self.store.drafts(&p.snapshot)?;
        for comment in &p.comments {
            if !saved.iter().any(|d| {
                d.id == comment.draft
                    && d.version == comment.version
                    && d.body == comment.body
                    && !d.published
                    && d.is_saved()
            }) {
                return Err(
                    "the saved text is newer or already published; prepare another preview".into(),
                );
            }
        }
        entry.baseline_review_ids = self
            .remote
            .reviews(&p.target)?
            .iter()
            .filter_map(|r| r["id"].as_u64())
            .collect();
        // The baseline lookup took time, so check the remote state again just before sending.
        if self.remote.current(&p.target)? != p.target {
            return Err("the PR changed just before sending; no request was made".into());
        }
        entry.state = OutboxState::InFlight;
        self.store.transition(&entry, &*self.rules)?;
        // A failure after this point creates InFlight; on the next open, the database changes it to UnknownOutcome.
        match self.remote.send(&p) {
            SendOutcome::Rejected(code) => {
                entry.state = OutboxState::Rejected;
                entry.diagnostic = Some(format!(
                    "Provider refused this request (HTTP {code}); drafts remain"
                ));
            }
            SendOutcome::Unknown(reason) => {
                entry.state = OutboxState::UnknownOutcome;
                entry.diagnostic = Some(reason);
            }
            SendOutcome::Accepted(response) => {
                entry.remote_id = response["id"].as_u64();
                let verified = match entry.remote_id {
                    Some(id) => self
                        .remote
                        .comments(&p.target, id)
                        .ok()
                        .is_some_and(|comments| {
                            matches_review(&p, &response, &comments, self.rules.marks_reviews())
                        }),
                    None => false,
                };
                if verified {
                    entry.state = OutboxState::Confirmed;
                    entry.diagnostic = None;
                } else {
                    entry.state = OutboxState::UnknownOutcome;
                    entry.diagnostic = Some("The server accepted the request, but the review or comment could not be fully verified.".into());
                }
            }
        }
        self.store.transition(&entry, &*self.rules)?;
        Ok(entry)
    }
    pub fn reconcile(&self, id: &OperationId) -> Result<OutboxEntry> {
        let _guard = self.gate.lock().map_err(|_| "publication gate poisoned")?;
        let mut e = self.store.operation(id)?;
        if e.state != OutboxState::UnknownOutcome {
            return Ok(e);
        }
        let account = self.remote.current(&e.prepared.target)?.account;
        if account != e.prepared.target.account {
            return Err(
                "reconciliation requires the account that originally sent this review".into(),
            );
        }
        let reviews = self.remote.reviews(&e.prepared.target)?;
        let mut matches = vec![];
        for r in reviews {
            let Some(id) = r["id"].as_u64() else { continue };
            if let Some(known) = e.remote_id {
                if known != id {
                    continue;
                }
            } else if e.baseline_review_ids.contains(&id) {
                continue;
            }
            let marked = self.rules.marks_reviews();
            if !matches_metadata(&e.prepared, &r, marked) {
                continue;
            }
            let comments = self.remote.comments(&e.prepared.target, id)?;
            if matches_review(&e.prepared, &r, &comments, marked) {
                matches.push(id)
            }
        }
        if matches.len() == 1 {
            e.state = OutboxState::Confirmed;
            e.remote_id = Some(matches[0]);
            e.diagnostic = None;
            self.store.transition(&e, &*self.rules)?;
        }
        // With zero or several matches, keep UnknownOutcome and provide no resend route.
        Ok(e)
    }
}
fn same_pr(a: &RemoteTarget, b: &RemoteTarget) -> bool {
    a.provider == b.provider
        && a.repository == b.repository
        && a.pr == b.pr
        && a.account == b.account
}
/// `marked`: the provider tags its reviews with diffz's fingerprint, so a match must carry it.
fn matches_metadata(p: &PreparedReview, r: &Value, marked: bool) -> bool {
    if marked && r["fingerprint"].as_str() != Some(p.fingerprint.as_str()) {
        return false;
    }
    r["commit_id"].as_str() == Some(p.target.head.as_str())
        && r["state"].as_str() == Some(p.verdict.remote_state())
        && r["body"].as_str().unwrap_or_default() == p.summary
        && r["user"]["login"].as_str() == Some(p.target.account.as_str())
}
fn comment_key(c: &Value) -> Option<(String, String, String, u64, u64)> {
    let line = c["line"].as_u64().or_else(|| c["original_line"].as_u64())?;
    let start = c["start_line"]
        .as_u64()
        .or_else(|| c["original_start_line"].as_u64())
        .unwrap_or(line);
    let side = c["side"].as_str()?;
    let start_side = c["start_side"].as_str().unwrap_or(side);
    if start_side != side {
        return None;
    }
    Some((
        c["path"].as_str()?.into(),
        c["body"].as_str()?.into(),
        side.into(),
        start,
        line,
    ))
}
pub fn matches_review(p: &PreparedReview, r: &Value, comments: &[Value], marked: bool) -> bool {
    if !matches_metadata(p, r, marked) || comments.len() != p.comments.len() {
        return false;
    }
    let mut expected: Vec<_> = p
        .comments
        .iter()
        .map(|c| {
            (
                c.path.clone(),
                c.body.clone(),
                c.side.api().to_owned(),
                u64::from(c.start_line),
                u64::from(c.line),
            )
        })
        .collect();
    let Some(mut actual) = comments.iter().map(comment_key).collect::<Option<Vec<_>>>() else {
        return false;
    };
    expected.sort();
    actual.sort();
    expected == actual
}
