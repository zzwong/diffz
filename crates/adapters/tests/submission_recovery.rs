use diffz_adapters::{
    Result,
    github::GithubRules,
    outbox::Outbox,
    provider::{ReviewRemote, SendOutcome},
    store::Store,
};
use diffz_core::{
    domain::*,
    patch::{ParseLimits, parse_patch},
    review::*,
};
use serde_json::{Value, json};
use std::sync::{Arc, Mutex};
struct Fake {
    target: RemoteTarget,
    rows: Mutex<Vec<Value>>,
    sends: Mutex<u32>,
}
impl ReviewRemote for Fake {
    fn current(&self, _: &RemoteTarget) -> Result<RemoteTarget> {
        Ok(self.target.clone())
    }
    fn reviews(&self, _: &RemoteTarget) -> Result<Vec<Value>> {
        Ok(self.rows.lock().unwrap().clone())
    }
    fn comments(&self, _: &RemoteTarget, _: u64) -> Result<Vec<Value>> {
        Ok(vec![])
    }
    fn send(&self, p: &PreparedReview) -> SendOutcome {
        *self.sends.lock().unwrap() += 1;
        self.rows.lock().unwrap().push(json!({"id":7,"commit_id":p.target.head,"state":"APPROVED","body":p.summary,"user":{"login":p.target.account}}));
        SendOutcome::Unknown("connection closed after acceptance".into())
    }
}
fn setup() -> (tempfile::TempDir, Arc<Store>, Arc<Fake>, PreparedReview) {
    let temp = tempfile::tempdir().unwrap();
    let store = Arc::new(Store::open(temp.path()).unwrap());
    let t = RemoteTarget {
        provider: diffz_core::domain::ProviderId::GITHUB,
        repository: RepositoryKey {
            host: "github.com".into(),
            id: 1,
            owner: "o".into(),
            name: "r".into(),
        },
        account: "me".into(),
        pr: 1,
        target_tip: "a".repeat(40),
        comparison_base: "a".repeat(40),
        head: "b".repeat(40),
        open: true,
        draft: false,
        pending_review: false,
    };
    let s = Snapshot::new(
        "p".into(),
        parse_patch(
            b"diff --git a/a b/a\n--- a/a\n+++ b/a\n@@ -1 +1 @@\n-x\n+y\n",
            ParseLimits::default(),
        )
        .unwrap(),
        Some(t.clone()),
        vec![],
    );
    store.put_snapshot(&s).unwrap();
    let p = PreparedReview::prepare(
        &GithubRules,
        OperationId("op".into()),
        &s,
        vec![],
        Verdict::Approve,
        "".into(),
    )
    .unwrap();
    store.insert_prepared(&p, &GithubRules).unwrap();
    let f = Arc::new(Fake {
        target: t,
        rows: Mutex::new(vec![]),
        sends: Mutex::new(0),
    });
    (temp, store, f, p)
}
#[test]
fn disconnect_is_unknown_and_not_retried() {
    let (_t, s, f, p) = setup();
    let o = Outbox::new(s, Arc::new(GithubRules), f.clone());
    let e = o.publish(p.clone()).unwrap();
    assert_eq!(e.state, OutboxState::UnknownOutcome);
    assert!(o.publish(p.clone()).is_err());
    assert_eq!(*f.sends.lock().unwrap(), 1);
    let e = o.reconcile(&p.id).unwrap();
    assert_eq!(e.state, OutboxState::Confirmed);
    assert_eq!(*f.sends.lock().unwrap(), 1);
}
#[test]
fn zero_matching_reviews_remain_unknown() {
    let (_t, s, f, p) = setup();
    let o = Outbox::new(s, Arc::new(GithubRules), f.clone());
    o.publish(p.clone()).unwrap();
    f.rows.lock().unwrap().clear();
    assert_eq!(
        o.reconcile(&p.id).unwrap().state,
        OutboxState::UnknownOutcome
    );
}
#[test]
fn duplicate_matches_do_not_confirm() {
    let (_t, s, f, p) = setup();
    let o = Outbox::new(s, Arc::new(GithubRules), f.clone());
    o.publish(p.clone()).unwrap();
    let mut rows = f.rows.lock().unwrap();
    let mut other = rows[0].clone();
    other["id"] = json!(8);
    rows.push(other);
    drop(rows);
    assert_eq!(
        o.reconcile(&p.id).unwrap().state,
        OutboxState::UnknownOutcome
    );
}
#[test]
fn restart_inflight_becomes_unknown() {
    let (t, s, _f, p) = setup();
    let mut e = s.operation(&p.id).unwrap();
    e.state = OutboxState::InFlight;
    s.transition(&e, &GithubRules).unwrap();
    drop(s);
    let s = Store::open(t.path()).unwrap();
    assert_eq!(
        s.operation(&p.id).unwrap().state,
        OutboxState::UnknownOutcome
    );
}
