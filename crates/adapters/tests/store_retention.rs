use diffz_adapters::{github::GithubRules, store::Store};
use diffz_core::{
    domain::*,
    patch::{ParseLimits, parse_patch},
    provider::SavedView,
    review::*,
    review_details::{BlameRead, Blamed, releases_key},
};

fn snapshot(i: usize) -> Snapshot {
    let target = RemoteTarget {
        provider: ProviderId::GITHUB,
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
        merged: false,
        pending_review: false,
        compare: None,
    };
    Snapshot::with_origin(
        format!("s{i}"),
        parse_patch(
            b"diff --git a/a b/a\n--- a/a\n+++ b/a\n@@ -1 +1 @@\n-x\n+y\n",
            ParseLimits::default(),
        )
        .unwrap(),
        Some(target),
        vec![],
        format!("origin-{i}"),
    )
}

fn prepare(store: &Store, s: &Snapshot) -> OutboxEntry {
    let p = PreparedReview::prepare(
        &GithubRules,
        OperationId(format!("op-{}", s.title)),
        s,
        vec![],
        Verdict::Approve,
        String::new(),
    )
    .unwrap();
    store.insert_prepared(&p, &GithubRules).unwrap()
}

#[test]
fn pruning_keeps_recent_and_pending_work_and_removes_the_rest() {
    let temp = tempfile::tempdir().unwrap();
    let store = Store::open(temp.path()).unwrap();
    let s: Vec<_> = (0..6).map(snapshot).collect();
    for snap in &s {
        store.put_snapshot(snap).unwrap();
    }
    store
        .save_draft(Draft {
            id: DraftId("pending".into()),
            snapshot: s[0].id.clone(),
            file: s[0].patch.files[0].id.clone(),
            side: Side::Right,
            start_line: 1,
            line: 1,
            file_level: false,
            body: "keep me".into(),
            version: 1,
            saved_version: 0,
            published: false,
        })
        .unwrap();
    prepare(&store, &s[1]);
    let mut rejected = prepare(&store, &s[3]);
    rejected.state = OutboxState::Rejected;
    store.transition(&rejected, &GithubRules).unwrap();
    store.save_view(&s[2].id, &SavedView::default()).unwrap();
    store.hide_recent(Some(&s[2].id)).unwrap();
    let mut blame = BlameRead::new();
    blame.insert(
        "a".into(),
        Some(vec![Blamed {
            start: 1,
            end: 1,
            release: 0,
            commit: "abc".into(),
        }]),
    );
    store
        .save_blame(&s[2].id, &releases_key(&s[2].overview.releases), &blame)
        .unwrap();

    assert_eq!(store.prune(2).unwrap(), 2);
    for kept in [0, 1, 4, 5] {
        assert!(store.snapshot(&s[kept].id).is_ok(), "s{kept}");
    }
    for gone in [2, 3] {
        assert!(store.snapshot(&s[gone].id).is_err(), "s{gone}");
    }
    let db = rusqlite::Connection::open(temp.path().join("review.sqlite3")).unwrap();
    let remaining: i64 = db
        .query_row(
            "SELECT count(*) FROM snapshot_blame WHERE snapshot_id=?1",
            [&s[2].id.0],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(remaining, 0);
    assert_eq!(store.drafts(&s[0].id).unwrap().len(), 1);
    assert_eq!(store.outbox().unwrap().len(), 2);
    assert_eq!(store.prune(2).unwrap(), 0);
}

#[test]
fn opening_the_store_prunes_beyond_the_recent_window() {
    let temp = tempfile::tempdir().unwrap();
    {
        let store = Store::open(temp.path()).unwrap();
        for i in 0..60 {
            store.put_snapshot(&snapshot(i)).unwrap();
        }
        assert_eq!(store.recent().unwrap().len(), 30);
    }
    let store = Store::open(temp.path()).unwrap();
    assert!(store.snapshot(&snapshot(9).id).is_err());
    assert!(store.snapshot(&snapshot(10).id).is_ok());
    assert!(store.snapshot(&snapshot(59).id).is_ok());
}

#[test]
fn snapshots_are_stored_compressed_and_legacy_text_rows_still_load() {
    let temp = tempfile::tempdir().unwrap();
    let s = snapshot(0);
    Store::open(temp.path()).unwrap().put_snapshot(&s).unwrap();
    let db = rusqlite::Connection::open(temp.path().join("review.sqlite3")).unwrap();
    let kind: String = db
        .query_row("SELECT typeof(data) FROM snapshots", [], |r| r.get(0))
        .unwrap();
    assert_eq!(kind, "blob");
    db.execute(
        "UPDATE snapshots SET data=?1",
        [serde_json::to_string(&s).unwrap()],
    )
    .unwrap();
    drop(db);
    let loaded = Store::open(temp.path()).unwrap().snapshot(&s.id).unwrap();
    assert_eq!(loaded.id, s.id);
}
