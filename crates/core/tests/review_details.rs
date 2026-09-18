use diffz_core::{
    domain::Snapshot,
    patch::{ParseLimits, parse_patch},
    review_details::*,
};
#[test]
fn removes_hidden_metadata_and_keeps_code_examples() {
    assert_eq!(
        visible_markdown("<!-- secret -->Hello **there**"),
        "Hello **there**"
    );
    assert_eq!(
        visible_markdown("```html\n<!-- example -->\n```"),
        "```html\n<!-- example -->\n```"
    );
    assert!(!visible_markdown("![pixel](https://example.com/x)").contains("!["));
}
#[test]
fn old_cached_snapshot_deserializes_without_overview() {
    let s = Snapshot::new(
        "test".into(),
        parse_patch(
            include_bytes!("../../../fixtures/split-asymmetric/change.patch"),
            ParseLimits::default(),
        )
        .unwrap(),
        None,
        vec![],
    );
    let mut value = serde_json::to_value(&s).unwrap();
    value.as_object_mut().unwrap().remove("overview");
    let old: Snapshot = serde_json::from_value(value).unwrap();
    assert!(old.overview.description.is_none());
    assert!(old.verify_identity());
}
#[test]
fn momentum_does_not_skip_files() {
    let mut gate = BoundaryScroll::default();
    assert_eq!(gate.update(1, 1., false), None);
    for _ in 0..20 {
        assert_eq!(gate.update(1, 12., false), None);
    }
    // A fresh gesture pulls against the edge and turns once it has come far enough.
    assert_eq!(gate.update(1, 12., true), None);
    let mut turned = None;
    for _ in 0..20 {
        turned = gate.update(1, 12., false);
        if turned.is_some() {
            break;
        }
    }
    assert_eq!(turned, Some(1));
    assert_eq!(gate.update(1, 12., false), None);
    // Reversing at the other edge first arms it, then one fresh pull turns.
    assert_eq!(gate.update(-1, -12., true), None);
    assert_eq!(gate.update(-1, -12., true), None);
    let mut turned = None;
    for _ in 0..20 {
        turned = gate.update(-1, -12., false);
        if turned.is_some() {
            break;
        }
    }
    assert_eq!(turned, Some(-1));
}

#[test]
fn unified_context_finds_threads_from_the_other_side() {
    use diffz_core::domain::*;
    let mut snapshot = Snapshot::new(
        "test".into(),
        parse_patch(
            include_bytes!("../../../fixtures/review-flow/change.patch"),
            ParseLimits::default(),
        )
        .unwrap(),
        None,
        vec![],
    );
    let file = snapshot.patch.files[0].id.clone();
    snapshot.comments.push(ThreadComment {
        id: 1,
        root_id: 1,
        path: "src/review.rs".into(),
        side: Some(Side::Left),
        line: Some(1),
        start_line: None,
        body: "text".into(),
        author: "reviewer".into(),
        commit_id: String::new(),
        created_at: None,
    });
    let point = SourcePoint {
        snapshot: snapshot.id.clone(),
        file,
        side: Side::Right,
        line: 1,
        byte_column: 0,
    };
    assert_eq!(thread_roots_at(&snapshot, &point), vec![1]);
}

#[test]
fn fresh_gesture_after_reaching_boundary_pulls_to_the_threshold() {
    use diffz_core::scroll::PULL_THRESHOLD;
    let mut gate = BoundaryScroll::default();
    assert_eq!(gate.update(0, 100., false), None);
    assert_eq!(gate.update(1, 20., true), None);
    assert!(gate.progress() > 0.);
    assert_eq!(gate.update(1, PULL_THRESHOLD, false), Some(1));
}

#[test]
fn short_timestamp_trims_rfc3339_to_minutes() {
    use diffz_core::review_details::short_timestamp;
    assert_eq!(short_timestamp("2026-09-04T14:22:31Z"), "2026-09-04 14:22");
    assert_eq!(
        short_timestamp("2026-09-04T14:22:31.123+02:00"),
        "2026-09-04 14:22"
    );
    assert_eq!(short_timestamp("yesterday"), "yesterday");
    assert_eq!(short_timestamp(""), "");
}

fn comment(id: u64, root_id: u64, path: &str) -> diffz_core::domain::ThreadComment {
    use diffz_core::domain::ThreadComment;
    ThreadComment {
        id,
        root_id,
        path: path.into(),
        side: None,
        line: None,
        start_line: None,
        body: String::new(),
        author: "reviewer".into(),
        commit_id: String::new(),
        created_at: None,
    }
}

#[test]
fn thread_counts_sums_root_threads_per_file_ignoring_replies() {
    let comments = vec![
        comment(1, 1, "a.rs"),
        comment(2, 2, "a.rs"),
        comment(3, 1, "a.rs"), // reply on thread 1
        comment(4, 4, "b.rs"),
    ];
    let counts = thread_counts(&comments);
    assert_eq!(
        counts,
        std::collections::BTreeMap::from([("a.rs".into(), 2), ("b.rs".into(), 1)])
    );
}

#[test]
fn reviewed_progress_counts_viewed_files_and_total() {
    use std::collections::BTreeMap;
    let viewed = BTreeMap::from([("a.rs".into(), true), ("b.rs".into(), true)]);
    let files = vec!["a.rs".into(), "b.rs".into(), "d.rs".into()];
    assert_eq!(reviewed_progress(&viewed, &files), (2, 3));
    assert_eq!(reviewed_progress(&BTreeMap::new(), &[]), (0, 0));
}
#[test]
fn review_decision_follows_each_reviewer_s_latest_ruling() {
    use diffz_core::review_details::{ReviewDecision, review_decision};
    assert_eq!(
        review_decision([("a", "COMMENTED"), ("b", "PENDING")]),
        None
    );
    assert_eq!(
        review_decision([("a", "APPROVED"), ("b", "CHANGES_REQUESTED")]),
        Some(ReviewDecision::ChangesRequested)
    );
    assert_eq!(
        review_decision([("a", "CHANGES_REQUESTED"), ("a", "APPROVED")]),
        Some(ReviewDecision::Approved)
    );
    assert_eq!(
        review_decision([("a", "APPROVED"), ("a", "DISMISSED")]),
        None
    );
    assert_eq!(
        ReviewDecision::ChangesRequested.label(),
        "changes requested"
    );
}
