use diffz_adapters::{
    local_git::{LocalGit, LocalMode},
    process::resolve_program,
    service::Services,
    store::Store,
};
use diffz_core::{
    domain::{Draft, DraftId, Side, Snapshot},
    patch::ContentKind,
    provider::{Cancellation, OpenRequest, SavedView, WorkbenchServices},
};
use std::{collections::BTreeMap, path::Path, process::Command};

#[cfg(unix)]
#[path = "support/fake_cli.rs"]
mod fake_cli;

fn repository() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    let output = Command::new("git")
        .arg("-C")
        .arg(dir.path())
        .args(["init", "-b", "main"])
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .output()
        .unwrap();
    assert!(output.status.success(), "{:?}", output.stderr);
    dir
}

fn repository_files(root: &Path) -> BTreeMap<std::path::PathBuf, Vec<u8>> {
    fn collect(root: &Path, dir: &Path, files: &mut BTreeMap<std::path::PathBuf, Vec<u8>>) {
        for entry in std::fs::read_dir(dir).unwrap() {
            let entry = entry.unwrap();
            let path = entry.path();
            if entry.file_type().unwrap().is_dir() {
                collect(root, &path, files);
            } else {
                files.insert(
                    path.strip_prefix(root).unwrap().to_path_buf(),
                    std::fs::read(path).unwrap(),
                );
            }
        }
    }
    let mut files = BTreeMap::new();
    collect(root, root, &mut files);
    files
}

fn capture(git: &LocalGit, root: &Path) -> Snapshot {
    let before = repository_files(root);
    let snapshot = git
        .snapshot(root, LocalMode::WorkingTree, Cancellation::default())
        .unwrap();
    assert_eq!(repository_files(root), before);
    assert!(snapshot.verify_identity());
    snapshot
}

#[test]
fn untracked_binary_content_controls_snapshot_but_not_file_identity() {
    for (before, after) in [
        (b"\0old".as_slice(), b"\0new".as_slice()),
        (b"\xffold".as_slice(), b"\xffnew".as_slice()),
        (b"\0short".as_slice(), b"\0longer contents".as_slice()),
    ] {
        let repo = repository();
        let path = repo.path().join("untracked.bin");
        let git = LocalGit::new(resolve_program("git").unwrap());
        std::fs::write(&path, before).unwrap();
        let first = capture(&git, repo.path());
        assert_eq!(first.id, capture(&git, repo.path()).id);
        assert_eq!(first.patch.files.len(), 1);
        let file = &first.patch.files[0];
        assert_eq!(file.content, ContentKind::Binary);
        assert!(file.hunks.is_empty());
        assert!(file.new_oid.is_none());
        assert_eq!(file.metadata.len(), 2);
        assert!(file.metadata[1].starts_with("untracked binary content digest: "));

        std::fs::write(&path, after).unwrap();
        let second = capture(&git, repo.path());
        assert_eq!(second.id, capture(&git, repo.path()).id);
        assert_eq!(file.id, second.patch.files[0].id);
        assert_eq!(second.patch.files[0].content, ContentKind::Binary);
        assert_ne!(first.id, second.id);
        assert_ne!(file.metadata, second.patch.files[0].metadata);

        std::fs::write(&path, before).unwrap();
        assert_eq!(first.id, capture(&git, repo.path()).id);
    }
}

#[test]
fn untracked_text_keeps_its_existing_representation() {
    let repo = repository();
    let path = repo.path().join("untracked.txt");
    let git = LocalGit::new(resolve_program("git").unwrap());
    for bytes in [b"".as_slice(), b"old\n".as_slice(), b"new\r\n".as_slice()] {
        std::fs::write(&path, bytes).unwrap();
        let snapshot = capture(&git, repo.path());
        let file = &snapshot.patch.files[0];
        assert_eq!(file.content, ContentKind::Text);
        assert!(file.new_oid.is_none());
        assert_eq!(
            file.metadata,
            ["untracked file; the worktree was left unchanged"]
        );
        assert_eq!(snapshot.id, capture(&git, repo.path()).id);
    }
}

#[test]
fn binary_revisions_keep_drafts_and_view_state_separate() {
    let repo = repository();
    let state = tempfile::tempdir().unwrap();
    let services = Services::new(state.path(), false).unwrap();
    let request = OpenRequest::LocalWorktree(repo.path().to_path_buf());
    let path = repo.path().join("untracked.bin");
    std::fs::write(&path, b"\0old").unwrap();
    let first = services
        .open(request.clone(), Cancellation::default())
        .unwrap();
    let file = first.snapshot.patch.files[0].id.clone();
    services
        .save_draft(Draft {
            id: DraftId("binary-draft".into()),
            snapshot: first.snapshot.id.clone(),
            file: file.clone(),
            side: Side::Right,
            start_line: 0,
            line: 0,
            file_level: true,
            body: "Review of the old binary".into(),
            version: 1,
            saved_version: 0,
            published: false,
        })
        .unwrap();
    let mut view = SavedView {
        revision: 1,
        selected_file: Some(file.clone()),
        ..Default::default()
    };
    view.preferences.viewed.insert(file.0.clone(), true);
    services.save_view(&first.snapshot.id, view).unwrap();

    std::fs::write(&path, b"\0new").unwrap();
    let second = services.open(request, Cancellation::default()).unwrap();
    assert_ne!(first.snapshot.id, second.snapshot.id);
    assert!(second.drafts.is_empty());
    assert!(second.view.preferences.viewed.is_empty());
    let resumed = services
        .open(OpenRequest::Resume(first.snapshot.id), Cancellation::default())
        .unwrap();
    assert_eq!(resumed.drafts.len(), 1);
    assert_eq!(resumed.drafts[0].body, "Review of the old binary");
    assert_eq!(resumed.view.preferences.viewed.get(&file.0), Some(&true));
}

#[test]
fn legacy_untracked_binary_snapshot_still_verifies_and_resumes() {
    // Pre-digest metadata and a fixed legacy identity, with no binary payload to recover.
    let legacy: Snapshot =
        serde_json::from_str(include_str!("fixtures/untracked_binary_legacy.json")).unwrap();
    assert!(legacy.verify_identity());
    let expected = serde_json::to_vec(&legacy).unwrap();
    let state = tempfile::tempdir().unwrap();
    {
        let store = Store::open(state.path()).unwrap();
        store.put_snapshot(&legacy).unwrap();
    }
    let store = Store::open(state.path()).unwrap();
    let restored = store.opened(&legacy.id).unwrap().snapshot;
    assert!(restored.verify_identity());
    assert_eq!(serde_json::to_vec(&restored).unwrap(), expected);
    assert_eq!(restored.patch.files[0].metadata.len(), 1);
}

#[test]
#[cfg(unix)]
fn binary_changes_between_captures_are_rejected() {
    let repo = repository();
    let bin = tempfile::tempdir().unwrap();
    let path = repo.path().join("untracked.bin");
    let count = bin.path().join("reads");
    let program = bin.path().join("git");
    std::fs::write(&path, b"\0old").unwrap();
    let quote = |path: &Path| format!("'{}'", path.to_str().unwrap().replace('\'', "'\\''"));
    // Only this test's wrapper alternates the file before each untracked read.
    fake_cli::write_executable(
        &program,
        &format!(
            "#!/bin/sh\nset -eu\nfor arg in \"$@\"; do\n\
             if [ \"$arg\" = ls-files ]; then\n\
             n=0\nif [ -f {count} ]; then n=$(cat {count}); fi\n\
             n=$((n + 1))\nprintf '%s' \"$n\" > {count}\n\
             if [ $((n % 2)) -eq 1 ]; then\n\
             printf '\\000old' > {path}\nelse\n\
             printf '\\000new' > {path}\nfi\nfi\ndone\n\
             exec {git} \"$@\"\n",
            count = quote(&count),
            path = quote(&path),
            git = quote(&resolve_program("git").unwrap()),
        ),
    );
    let error = LocalGit::new(program)
        .snapshot(repo.path(), LocalMode::WorkingTree, Cancellation::default())
        .unwrap_err();
    assert!(error.to_string().contains("local source changed during capture"));
    assert_eq!(std::fs::read_to_string(count).unwrap(), "6");
}
