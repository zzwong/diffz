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
fn release(tag: &str, shas: &[&str], files: &[(&str, Option<&str>)]) -> Release {
    Release {
        tag: Some(tag.into()),
        commit: shas.last().map_or_else(String::new, |s| s.to_string()),
        date: None,
        commits: shas.len() as u64,
        files: files
            .iter()
            .map(|(path, previous)| ReleaseFile {
                path: path.to_string(),
                additions: 1,
                deletions: 1,
                previous: previous.map(str::to_owned),
            })
            .collect(),
        notes: None,
        url: None,
        shas: shas.iter().map(|s| s.to_string()).collect(),
    }
}
#[test]
fn releases_follow_a_file_renamed_in_the_range() {
    // v2 renames old.rs to new.rs; v3 renames it again to last.rs.
    let releases = [
        release("v1", &["a"], &[("old.rs", None), ("other.rs", None)]),
        release("v2", &["b"], &[("new.rs", Some("old.rs"))]),
        release("v3", &["c"], &[("last.rs", Some("new.rs"))]),
        release("v4", &["d"], &[("last.rs", None)]),
    ];
    let by_path = releases_by_path(&releases);
    assert_eq!(by_path["last.rs"], [0, 1, 2, 3]);
    assert_eq!(by_path["other.rs"], [0]);
    assert!(!by_path.contains_key("old.rs") && !by_path.contains_key("new.rs"));
}
#[test]
fn blame_ranges_map_to_the_release_that_added_their_commit() {
    let releases = [
        release("v1", &["a1", "a2"], &[("f.rs", None)]),
        release("v2", &["b1"], &[("f.rs", None)]),
    ];
    let ranges = [
        (8, 9, "b1".to_string()),
        (1, 3, "before".to_string()),
        (4, 4, "a2".to_string()),
    ];
    let found = attribute(&releases, &ranges);
    assert_eq!(
        found
            .iter()
            .map(|b| (b.start, b.end, b.release))
            .collect::<Vec<_>>(),
        [(4, 4, 0), (8, 9, 1)]
    );
    let mut overview = Overview {
        releases: releases.to_vec(),
        ..Default::default()
    };
    overview.blame.insert("f.rs".into(), Some(found));
    assert_eq!(overview.blamed("f.rs", 9).map(|b| b.release), Some(1));
    for line in [2, 5, 10] {
        assert!(overview.blamed("f.rs", line).is_none(), "{line}");
    }
    assert!(overview.blamed("g.rs", 4).is_none());
}
#[test]
fn only_added_rows_take_a_release() {
    use diffz_core::patch::{PatchRow, RowKind};
    let releases = [
        release("v1", &["a"], &[("f.rs", None)]),
        release("v2", &["b"], &[("f.rs", None)]),
    ];
    let mut overview = Overview {
        releases: releases.to_vec(),
        ..Default::default()
    };
    let blamed = attribute(&releases, &[(1, 10, "b".into())]);
    overview.blame.insert("f.rs".into(), Some(blamed));
    let row = |kind, old_line, new_line| PatchRow {
        old_line,
        new_line,
        text: "x".into(),
        ending: diffz_core::domain::LineEnding::Lf,
        kind,
    };
    let added = row(RowKind::Added, None, Some(3));
    assert_eq!(
        overview.row_release("f.rs", &added).map(|b| b.release),
        Some(1)
    );
    // Blame at the head covers context too, and cannot see removals.
    assert!(
        overview
            .row_release("f.rs", &row(RowKind::Context, Some(3), Some(4)))
            .is_none()
    );
    assert!(
        overview
            .row_release("f.rs", &row(RowKind::Removed, Some(3), None))
            .is_none()
    );
    assert_eq!(removed_in(&[]), "Removed in this range");
    assert_eq!(removed_in(&["v2"]), "Removed in v2");
    assert_eq!(removed_in(&["v1", "v2"]), "Removed in one of: v1, v2");
    assert_eq!(
        removed_in(&["v1", "v2", "v3", "v4", "v5"]),
        "Removed in one of 5 releases, v1 to v5"
    );
}
#[test]
fn blame_is_wanted_once_per_file_and_only_across_releases() {
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
    let file = &s.patch.files[0];
    let path = file.display_path();
    let mut overview = Overview {
        releases: vec![release("v1", &["a"], &[(&path, None)])],
        ..Default::default()
    };
    assert_eq!(blame_span(&overview, file), None);
    overview
        .releases
        .push(release("v2", &["b"], &[(&path, None)]));
    let added: Vec<u32> = file
        .hunks
        .iter()
        .flat_map(|h| &h.rows)
        .filter(|r| r.kind == diffz_core::patch::RowKind::Added)
        .filter_map(|r| r.new_line)
        .collect();
    assert_eq!(
        blame_span(&overview, file),
        Some((*added.iter().min().unwrap(), *added.iter().max().unwrap()))
    );
    // A file read once, even one that failed, is not asked for again; failures share one warning.
    overview.blame.insert(path.clone(), None);
    assert_eq!(blame_span(&overview, file), None);
    assert!(overview.unblamed_warning().unwrap().contains(&path));
    overview.blame.insert("b.rs".into(), None);
    overview.blame.insert("c.rs".into(), Some(vec![]));
    let warning = overview.unblamed_warning().unwrap();
    assert!(
        warning.starts_with(UNBLAMED) && warning.contains("2 files"),
        "{warning}"
    );
}
