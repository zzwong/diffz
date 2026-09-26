use diffz_adapters::{
    gitlab::{GitlabPosition, GitlabReader, GitlabRules, GitlabWriter, MrAddress},
    outbox::Outbox,
    store::Store,
};
use diffz_core::{domain::*, provider::Cancellation, review::*};
use std::sync::Arc;
fn position(p: &PreparedReview) -> GitlabPosition {
    serde_json::from_str(p.comments[0].position.as_ref().unwrap().get()).unwrap()
}
#[test]
fn nested_projects_and_strict_addresses() {
    let a = MrAddress::parse("https://git.example.com/group/sub/project/-/merge_requests/12/diffs")
        .unwrap();
    assert_eq!(a.project, "group/sub/project");
    assert_eq!(a.number, 12);
    assert!(MrAddress::parse("group/sub/project!2").is_ok());
    for s in [
        "2",
        "group/project!0",
        "http://gitlab.com/a/b/-/merge_requests/1",
        "https://me:pw@gitlab.com/a/b/-/merge_requests/1",
        "https://gitlab.com/a/../b/-/merge_requests/1",
        "a/b!1?x",
        "https://gitlab.com/a/b/-/merge_requests/1?x",
    ] {
        assert!(MrAddress::parse(s).is_err(), "{s}");
    }
}
#[cfg(unix)]
fn reader(dir: &std::path::Path) -> Arc<GitlabReader> {
    use std::os::unix::fs::PermissionsExt;
    let path = dir.join("fake-glab");
    std::fs::write(&path, include_str!("support/fake_glab.py")).unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o700)).unwrap();
    Arc::new(GitlabReader::new(path))
}
#[test]
#[cfg(unix)]
fn failed_glab_run_reports_its_stderr() {
    use std::os::unix::fs::PermissionsExt;
    let temp = tempfile::tempdir().unwrap();
    let glab = temp.path().join("glab");
    std::fs::write(
        &glab,
        "#!/bin/sh\necho 'glab: 401 Unauthorized' >&2\nexit 1\n",
    )
    .unwrap();
    std::fs::set_permissions(&glab, std::fs::Permissions::from_mode(0o700)).unwrap();
    let err = GitlabReader::new(glab)
        .snapshot(
            &MrAddress::parse("team/repo!7").unwrap(),
            Cancellation::default(),
        )
        .err()
        .unwrap()
        .to_string();
    assert!(
        err.contains("glab exited before a full HTTP response arrived (exit Some(1))"),
        "{err}"
    );
    assert!(
        err.contains("glab reported: glab: 401 Unauthorized;"),
        "{err}"
    );
}
#[test]
#[cfg(unix)]
fn gitlab_read_publish_and_history_are_local_mocked() {
    let temp = tempfile::tempdir().unwrap();
    let r = reader(temp.path());
    let a = MrAddress::parse("team/sub/repo!7").unwrap();
    let s = r.snapshot(&a, Cancellation::default()).unwrap();
    assert_eq!(s.remote.as_ref().unwrap().provider, ProviderId::GITLAB);
    assert_eq!(s.comments.len(), 2);
    assert_eq!(s.overview.checks.len(), 2);
    assert_eq!(s.overview.conversation.len(), 1);
    assert_eq!(s.overview.author.as_deref(), Some("mr-author"));
    assert_eq!(s.overview.decision, None);
    let store = Arc::new(Store::open(&temp.path().join("db")).unwrap());
    store.put_snapshot(&s).unwrap();
    let mut d = Draft {
        id: DraftId("d".into()),
        snapshot: s.id.clone(),
        file: s.patch.files[0].id.clone(),
        side: Side::Left,
        start_line: 1,
        line: 1,
        file_level: false,
        body: "Please keep this context.".into(),
        version: 1,
        saved_version: 0,
        published: false,
    };
    d.saved_version = store.save_draft(d.clone()).unwrap();
    let p = PreparedReview::prepare(
        &GitlabRules,
        OperationId("op".into()),
        &s,
        vec![d],
        Verdict::Comment,
        "Looks good with this note.".into(),
    )
    .unwrap();
    assert_eq!(position(&p).old_line, Some(1));
    assert_eq!(position(&p).new_line, Some(1));
    store.insert_prepared(&p, &GitlabRules).unwrap();
    let o = Outbox::new(
        store.clone(),
        Arc::new(GitlabRules),
        Arc::new(GitlabWriter::new(r)),
    );
    assert_eq!(o.publish(p.clone()).unwrap().state, OutboxState::Confirmed);
    assert!(o.publish(p).is_err());
    store.hide_recent(Some(&s.id)).unwrap();
    assert!(store.recent().unwrap().is_empty());
    assert_eq!(store.drafts(&s.id).unwrap().len(), 1);
    store.opened(&s.id).unwrap();
    assert_eq!(store.recent().unwrap().len(), 1);
    store.hide_recent(None).unwrap();
    drop(o);
    drop(store);
    let store = Store::open(&temp.path().join("db")).unwrap();
    assert!(store.recent().unwrap().is_empty());
    assert!(store.snapshot(&s.id).unwrap().verify_identity());
}
#[test]
#[cfg(unix)]
fn gitlab_file_level_draft_publishes_as_plain_note_not_discussion() {
    let temp = tempfile::tempdir().unwrap();
    let r = reader(temp.path());
    let s = r
        .snapshot(
            &MrAddress::parse("team/sub/repo!7").unwrap(),
            Cancellation::default(),
        )
        .unwrap();
    let store = Arc::new(Store::open(&temp.path().join("db")).unwrap());
    store.put_snapshot(&s).unwrap();
    let mut d = Draft {
        id: DraftId("fl".into()),
        snapshot: s.id.clone(),
        file: s.patch.files[0].id.clone(),
        side: Side::Right,
        start_line: 0,
        line: 0,
        file_level: true,
        body: "Please review the whole file.".into(),
        version: 1,
        saved_version: 0,
        published: false,
    };
    d.saved_version = store.save_draft(d.clone()).unwrap();
    let p = PreparedReview::prepare(
        &GitlabRules,
        OperationId("fl-op".into()),
        &s,
        vec![d],
        Verdict::Comment,
        "Summary.".into(),
    )
    .unwrap();
    assert!(p.comments[0].file_level);
    store.insert_prepared(&p, &GitlabRules).unwrap();
    let outbox = Outbox::new(store, Arc::new(GitlabRules), Arc::new(GitlabWriter::new(r)));
    assert_eq!(
        outbox.publish(p.clone()).unwrap().state,
        OutboxState::Confirmed
    );
    let state: serde_json::Value =
        serde_json::from_slice(&std::fs::read(temp.path().join("fake-state.json")).unwrap())
            .unwrap();
    // This file-level draft went out as a single plain note on the MR, ahead of the summary.
    let body = state["notes"][0]["body"].as_str().unwrap();
    assert!(body.starts_with("**a.rs**\n\nPlease review the whole file."));
    assert!(body.contains("<!-- diffz:"));
    assert!(state["notes"][0]["type"].is_null());
    // It never created a positioned discussion.
    assert_eq!(state["discussions"].as_array().unwrap().len(), 0);
}
#[test]
#[cfg(unix)]
fn partial_gitlab_send_remains_unknown_and_cannot_retry() {
    let temp = tempfile::tempdir().unwrap();
    let r = reader(temp.path());
    let s = r
        .snapshot(
            &MrAddress::parse("team/sub/repo!7").unwrap(),
            Cancellation::default(),
        )
        .unwrap();
    let store = Arc::new(Store::open(&temp.path().join("db")).unwrap());
    store.put_snapshot(&s).unwrap();
    let mut d = Draft {
        id: DraftId("d".into()),
        snapshot: s.id.clone(),
        file: s.patch.files[0].id.clone(),
        side: Side::Right,
        start_line: 2,
        line: 2,
        file_level: false,
        body: "A note.".into(),
        version: 1,
        saved_version: 0,
        published: false,
    };
    d.saved_version = store.save_draft(d.clone()).unwrap();
    let p = PreparedReview::prepare(
        &GitlabRules,
        OperationId("op".into()),
        &s,
        vec![d],
        Verdict::Comment,
        "Summary".into(),
    )
    .unwrap();
    store.insert_prepared(&p, &GitlabRules).unwrap();
    std::fs::write(temp.path().join("fail-summary"), "").unwrap();
    let o = Outbox::new(
        store.clone(),
        Arc::new(GitlabRules),
        Arc::new(GitlabWriter::new(r)),
    );
    assert_eq!(
        o.publish(p.clone()).unwrap().state,
        OutboxState::UnknownOutcome
    );
    assert!(o.publish(p.clone()).is_err());
    assert_eq!(
        o.reconcile(&p.id).unwrap().state,
        OutboxState::UnknownOutcome
    );
    assert!(!store.drafts(&s.id).unwrap()[0].published);
}

#[test]
#[cfg(unix)]
fn gitlab_approval_is_verified_and_changed_head_rejected() {
    let temp = tempfile::tempdir().unwrap();
    let r = reader(temp.path());
    let s = r
        .snapshot(
            &MrAddress::parse("team/sub/repo!7").unwrap(),
            Cancellation::default(),
        )
        .unwrap();
    let store = Arc::new(Store::open(&temp.path().join("db")).unwrap());
    store.put_snapshot(&s).unwrap();
    let p = PreparedReview::prepare(
        &GitlabRules,
        OperationId("approval".into()),
        &s,
        vec![],
        Verdict::Approve,
        "Approved.".into(),
    )
    .unwrap();
    store.insert_prepared(&p, &GitlabRules).unwrap();
    let outbox = Outbox::new(
        store.clone(),
        Arc::new(GitlabRules),
        Arc::new(GitlabWriter::new(r)),
    );
    assert_eq!(outbox.publish(p).unwrap().state, OutboxState::Confirmed);
    let p = PreparedReview::prepare(
        &GitlabRules,
        OperationId("changed".into()),
        &s,
        vec![],
        Verdict::Comment,
        "Later review".into(),
    )
    .unwrap();
    store.insert_prepared(&p, &GitlabRules).unwrap();
    std::fs::write(temp.path().join("changed-head"), "").unwrap();
    assert_eq!(outbox.publish(p).unwrap().state, OutboxState::Rejected);
}
#[test]
#[cfg(unix)]
fn accepted_gitlab_summary_with_lost_response_reconciles_without_resending() {
    let temp = tempfile::tempdir().unwrap();
    let r = reader(temp.path());
    let s = r
        .snapshot(
            &MrAddress::parse("team/sub/repo!7").unwrap(),
            Cancellation::default(),
        )
        .unwrap();
    let store = Arc::new(Store::open(&temp.path().join("db")).unwrap());
    store.put_snapshot(&s).unwrap();
    let p = PreparedReview::prepare(
        &GitlabRules,
        OperationId("lost".into()),
        &s,
        vec![],
        Verdict::Comment,
        "Summary.".into(),
    )
    .unwrap();
    store.insert_prepared(&p, &GitlabRules).unwrap();
    std::fs::write(temp.path().join("lost-response"), "").unwrap();
    let outbox = Outbox::new(store, Arc::new(GitlabRules), Arc::new(GitlabWriter::new(r)));
    assert_eq!(
        outbox.publish(p.clone()).unwrap().state,
        OutboxState::UnknownOutcome
    );
    assert_eq!(
        outbox.reconcile(&p.id).unwrap().state,
        OutboxState::Confirmed
    );
    let state: serde_json::Value =
        serde_json::from_slice(&std::fs::read(temp.path().join("fake-state.json")).unwrap())
            .unwrap();
    assert_eq!(state["notes"].as_array().unwrap().len(), 1);
}
