use diffz_adapters::{
    local_git::{LocalGit, LocalMode},
    process::resolve_program,
};
use diffz_core::{patch::RowKind, provider::Cancellation};
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
    process::Command,
};
#[cfg(unix)]
#[path = "support/fake_cli.rs"]
mod fake_cli;
fn git(p: &std::path::Path, args: &[&str]) {
    let o = Command::new("git")
        .arg("-C")
        .arg(p)
        .args(args)
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .output()
        .unwrap();
    assert!(o.status.success(), "{:?}", o.stderr);
}
#[test]
fn local_diff_does_not_check_out_or_modify_files() {
    let d = tempfile::tempdir().unwrap();
    git(d.path(), &["init", "-b", "main"]);
    git(d.path(), &["config", "user.email", "test@example.invalid"]);
    git(d.path(), &["config", "user.name", "Test"]);
    git(d.path(), &["config", "commit.gpgsign", "false"]);
    std::fs::write(d.path().join("doc.md"), "old\n").unwrap();
    git(d.path(), &["add", "."]);
    git(d.path(), &["commit", "-m", "base"]);
    git(d.path(), &["checkout", "-b", "change"]);
    std::fs::write(d.path().join("doc.md"), "new paragraph\n").unwrap();
    git(d.path(), &["commit", "-am", "change"]);
    let g = LocalGit::new(resolve_program("git").unwrap());
    let s = g
        .snapshot(
            d.path(),
            LocalMode::Compare {
                base: "main".into(),
                head: "HEAD".into(),
            },
            Cancellation::default(),
        )
        .unwrap();
    assert_eq!(s.patch.files.len(), 1);
    assert_eq!(
        std::fs::read(d.path().join("doc.md")).unwrap(),
        b"new paragraph\n"
    );
}
#[test]
fn unborn_staged_repository() {
    let d = tempfile::tempdir().unwrap();
    git(d.path(), &["init", "-b", "main"]);
    std::fs::write(d.path().join("new.md"), "new\n").unwrap();
    git(d.path(), &["add", "."]);
    let g = LocalGit::new(resolve_program("git").unwrap());
    let s = g
        .snapshot(d.path(), LocalMode::Staged, Cancellation::default())
        .unwrap();
    assert_eq!(s.patch.files.len(), 1);
}

fn repository_files(root: &Path) -> BTreeMap<PathBuf, Vec<u8>> {
    fn collect(root: &Path, dir: &Path, files: &mut BTreeMap<PathBuf, Vec<u8>>) {
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

fn assert_blank_context_survives_config(
    mode: LocalMode,
    program: PathBuf,
    enable_suppression: impl FnOnce(&Path),
) {
    let d = tempfile::tempdir().unwrap();
    git(d.path(), &["init", "-b", "main"]);
    git(d.path(), &["config", "user.email", "test@example.invalid"]);
    git(d.path(), &["config", "user.name", "Test"]);
    git(d.path(), &["config", "commit.gpgsign", "false"]);
    std::fs::write(d.path().join("doc.md"), "before\n\nold\n\nafter\n").unwrap();
    git(d.path(), &["add", "."]);
    git(d.path(), &["commit", "-m", "base"]);
    std::fs::write(d.path().join("doc.md"), "before\n\nnew\n\nafter\n").unwrap();
    match &mode {
        LocalMode::Compare { .. } => git(d.path(), &["commit", "-am", "change"]),
        LocalMode::Staged => git(d.path(), &["add", "."]),
        LocalMode::WorkingTree => {}
    }

    let g = LocalGit::new(program);
    git(d.path(), &["config", "diff.suppressBlankEmpty", "false"]);
    let control = g
        .snapshot(d.path(), mode.clone(), Cancellation::default())
        .unwrap();
    assert_eq!(control.patch.files.len(), 1);
    assert_eq!(control.patch.files[0].hunks.len(), 1);
    let blank_context: Vec<_> = control.patch.files[0].hunks[0]
        .rows
        .iter()
        .filter(|row| row.kind == RowKind::Context && row.text.is_empty())
        .map(|row| (row.old_line, row.new_line))
        .collect();
    assert_eq!(blank_context, [(Some(2), Some(2)), (Some(4), Some(4))]);

    enable_suppression(d.path());
    // Include config, index, HEAD, refs, object files, and worktree contents.
    let before = repository_files(d.path());
    let snapshot = g.snapshot(d.path(), mode, Cancellation::default());
    assert_eq!(repository_files(d.path()), before);
    assert_eq!(
        serde_json::to_vec(&snapshot.unwrap().patch).unwrap(),
        serde_json::to_vec(&control.patch).unwrap(),
    );
}

fn assert_blank_context_with_repo_config(mode: LocalMode) {
    assert_blank_context_survives_config(mode, resolve_program("git").unwrap(), |repo| {
        git(repo, &["config", "diff.suppressBlankEmpty", "true"]);
    });
}

#[test]
fn compare_preserves_blank_context_with_suppression_enabled() {
    assert_blank_context_with_repo_config(LocalMode::Compare {
        base: "HEAD~1".into(),
        head: "HEAD".into(),
    });
}

#[test]
fn staged_preserves_blank_context_with_suppression_enabled() {
    assert_blank_context_with_repo_config(LocalMode::Staged);
}

#[test]
fn worktree_preserves_blank_context_with_suppression_enabled() {
    assert_blank_context_with_repo_config(LocalMode::WorkingTree);
}

#[test]
#[cfg(unix)]
fn global_config_preserves_blank_context_with_suppression_enabled() {
    let bin = tempfile::tempdir().unwrap();
    let config = bin.path().join("gitconfig");
    std::fs::write(&config, "[diff]\n\tsuppressBlankEmpty = true\n").unwrap();
    let program = bin.path().join("git");
    let quote = |path: &Path| format!("'{}'", path.to_str().unwrap().replace('\'', "'\\''"));
    fake_cli::write_executable(
        &program,
        &format!(
            "#!/bin/sh\nexport GIT_CONFIG_NOSYSTEM=1\nexport GIT_CONFIG_GLOBAL={}\nexec {} \"$@\"\n",
            quote(&config),
            quote(&resolve_program("git").unwrap()),
        ),
    );
    // Only the wrapper's children inherit this config; parallel tests are unaffected.
    for mode in [
        LocalMode::Compare {
            base: "HEAD~1".into(),
            head: "HEAD".into(),
        },
        LocalMode::Staged,
        LocalMode::WorkingTree,
    ] {
        assert_blank_context_survives_config(mode, program.clone(), |repo| {
            git(repo, &["config", "--unset", "diff.suppressBlankEmpty"]);
        });
    }
    assert_eq!(
        std::fs::read_to_string(config).unwrap(),
        "[diff]\n\tsuppressBlankEmpty = true\n",
    );
}

#[test]
#[cfg(unix)]
fn symlinked_git_that_points_into_the_repository_is_rejected() {
    let repo = tempfile::tempdir().unwrap();
    git(repo.path(), &["init", "-b", "main"]);
    let inner = repo.path().join("git");
    fake_cli::write_executable(&inner, "#!/bin/sh\nexec git \"$@\"\n");
    let bin = tempfile::tempdir().unwrap();
    std::os::unix::fs::symlink(&inner, bin.path().join("git")).unwrap();
    let err = LocalGit::new(bin.path().canonicalize().unwrap().join("git"))
        .snapshot(repo.path(), LocalMode::Staged, Cancellation::default())
        .err()
        .unwrap();
    assert!(
        err.to_string()
            .contains("cannot live inside the repository")
    );
}
