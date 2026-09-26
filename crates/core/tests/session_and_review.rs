use diffz_core::domain::*;
use diffz_core::patch::*;
use diffz_core::review::*;
use diffz_core::session::*;
fn snapshot() -> Snapshot {
    let patch = parse_patch(
        b"diff --git a/a.md b/a.md\n--- a/a.md\n+++ b/a.md\n@@ -42 +42 @@\n-old\n+new\n",
        ParseLimits::default(),
    )
    .unwrap();
    Snapshot::new(
        "test".into(),
        patch,
        Some(RemoteTarget {
            provider: diffz_core::domain::ProviderId::GITHUB,
            repository: RepositoryKey {
                host: "github.com".into(),
                id: 1,
                owner: "o".into(),
                name: "r".into(),
            },
            account: "alice".into(),
            pr: 3,
            target_tip: "a".repeat(40),
            comparison_base: "a".repeat(40),
            head: "b".repeat(40),
            open: true,
            draft: false,
            pending_review: false,
        }),
        vec![],
    )
}
fn draft(s: &Snapshot) -> Draft {
    Draft {
        id: DraftId("d".into()),
        snapshot: s.id.clone(),
        file: s.patch.files[0].id.clone(),
        side: Side::Right,
        start_line: 42,
        line: 42,
        file_level: false,
        body: "Please clarify".into(),
        version: 1,
        saved_version: 1,
        published: false,
    }
}
#[test]
fn late_save_ack_does_not_ack_newer_edit() {
    let s = snapshot();
    let d = draft(&s);
    let mut session = Session::new(s);
    session.drafts.insert(d.id.clone(), d.clone());
    session.reduce(Event::EditDraft {
        id: d.id.clone(),
        body: "new text".into(),
    });
    session.reduce(Event::DraftSaved {
        id: d.id.clone(),
        version: 1,
    });
    assert!(!session.drafts[&d.id].is_saved());
}
#[test]
fn revision_offer_never_replaces_snapshot() {
    let s = snapshot();
    let old = s.id.clone();
    let mut session = Session::new(s);
    session.reduce(Event::RevisionOffered {
        current: old.clone(),
        head: "c".repeat(40),
    });
    assert_eq!(session.snapshot.id, old);
    assert!(session.revision_offer.is_some());
}
#[test]
fn review_uses_original_source_line_and_commit() {
    let s = snapshot();
    let d = draft(&s);
    let p = PreparedReview::prepare(
        OperationId("op".into()),
        &s,
        vec![d],
        Verdict::Comment,
        "summary".into(),
    )
    .unwrap();
    let json = p.payload();
    assert_eq!(json["comments"][0]["line"], 42);
    assert_eq!(json["commit_id"], "b".repeat(40));
}
#[test]
fn file_level_draft_github_payload_is_a_subject_file_comment() {
    let s = snapshot();
    let mut d = draft(&s);
    d.side = Side::Right;
    d.start_line = 0;
    d.line = 0;
    d.file_level = true;
    let p = PreparedReview::prepare(
        OperationId("fl".into()),
        &s,
        vec![d],
        Verdict::Comment,
        "summary".into(),
    )
    .unwrap();
    let json = p.payload();
    let c = &json["comments"][0];
    assert_eq!(c["path"], "a.md");
    assert_eq!(c["body"], "Please clarify");
    assert_eq!(c["subject_type"], "file");
    assert!(c.get("line").is_none());
    assert!(c.get("side").is_none());
    assert!(c.get("start_line").is_none());
}
#[test]
fn imported_patch_cannot_publish() {
    let mut s = snapshot();
    s.remote = None;
    assert!(
        PreparedReview::prepare(
            OperationId("x".into()),
            &s,
            vec![],
            Verdict::Approve,
            String::new()
        )
        .is_err()
    );
}
#[test]
fn existing_pending_review_blocks_new_submission() {
    let mut s = snapshot();
    s.remote.as_mut().unwrap().pending_review = true;
    assert!(
        PreparedReview::prepare(
            OperationId("x".into()),
            &s,
            vec![],
            Verdict::Approve,
            String::new()
        )
        .is_err()
    );
}
#[test]
fn comments_must_belong_to_frozen_snapshot() {
    let s = snapshot();
    let mut d = draft(&s);
    d.snapshot = SnapshotId("other".into());
    assert!(
        PreparedReview::prepare(
            OperationId("x".into()),
            &s,
            vec![d],
            Verdict::Comment,
            "summary".into()
        )
        .is_err()
    );
}
#[test]
fn unknown_outcome_never_becomes_prepared_automatically() {
    assert!(!OutboxState::UnknownOutcome.can_transition(OutboxState::InFlight));
    assert!(!OutboxState::InFlight.can_transition(OutboxState::Prepared));
    assert!(OutboxState::UnknownOutcome.can_transition(OutboxState::Confirmed));
}
#[test]
fn same_visible_text_different_endings_has_different_snapshot() {
    let s = snapshot();
    let mut p = s.patch.clone();
    p.files[0].hunks[0].rows[1].ending = LineEnding::None;
    let changed = Snapshot::new("test".into(), p, s.remote.clone(), vec![]);
    assert_ne!(s.id, changed.id);
}
#[test]
fn file_level_draft_round_trips_and_validates() {
    let s = snapshot();
    let mut d = draft(&s);
    // This file-level draft keeps side Right with start_line and line both at zero.
    d.side = Side::Right;
    d.start_line = 0;
    d.line = 0;
    d.file_level = true;

    // Push it through serde_json and back, keeping file_level.
    let json = serde_json::to_string(&d).unwrap();
    let back: Draft = serde_json::from_str(&json).unwrap();
    assert!(back.is_file_level());
    assert_eq!(back.start_line, 0);
    assert_eq!(back.line, 0);
    assert_eq!(back.side, Side::Right);

    // A file-level draft passes despite line 0 not being an actual source line.
    let p = PreparedReview::prepare(
        OperationId("fl".into()),
        &s,
        vec![back],
        Verdict::Comment,
        "summary".into(),
    )
    .unwrap();
    assert_eq!(p.comments.len(), 1);
}
#[test]
fn legacy_draft_without_file_level_defaults_to_false() {
    let s = snapshot();
    let mut d = draft(&s);
    d.file_level = true;
    let json = serde_json::to_string(&d).unwrap();
    let legacy: serde_json::Value = serde_json::from_str(&json).unwrap();
    // Remove the new key so the payload looks legacy.
    let mut legacy = legacy.as_object().unwrap().clone();
    legacy.remove("file_level");
    let legacy = serde_json::Value::Object(legacy);

    let back: Draft = serde_json::from_value(legacy).unwrap();
    assert!(!back.is_file_level());
}
