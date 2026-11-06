use diffz_adapters::{
    local_git::{LocalGit, LocalMode},
    process::resolve_program,
};
use diffz_core::{
    patch::{ContentKind, RowKind},
    provider::Cancellation,
};
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

#[test]
fn entire_remotes_review_all_local_modes_without_touching_repository_state() {
    for forge in ["et", "gh"] {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path();
        git(root, &["init", "-b", "main"]);
        git(root, &["config", "user.email", "test@example.invalid"]);
        git(root, &["config", "user.name", "Test"]);
        git(root, &["config", "commit.gpgsign", "false"]);
        git(root, &["config", "protocol.entire.allow", "never"]);
        git(
            root,
            &[
                "remote",
                "add",
                "origin",
                &format!("entire://aws-eu-central-1.entire.io/{forge}/example/repo"),
            ],
        );
        std::fs::write(root.join("doc.md"), "base\n").unwrap();
        git(root, &["add", "doc.md"]);
        git(root, &["commit", "-m", "base"]);
        git(root, &["checkout", "-b", "entire/native/change"]);
        std::fs::write(root.join("doc.md"), "committed\n").unwrap();
        git(root, &["commit", "-am", "change"]);
        std::fs::write(root.join("doc.md"), "staged\n").unwrap();
        git(root, &["add", "doc.md"]);
        std::fs::write(root.join("doc.md"), "unstaged\n").unwrap();
        std::fs::write(root.join("new.txt"), "untracked\n").unwrap();

        // The custom transport is explicitly disabled: local review must work even without
        // Entire installed or signed in, and must not fetch an Entire-only branch.
        let before = repository_files(root);
        let reader = LocalGit::new(resolve_program("git").unwrap());
        for (mode, expected) in [
            (
                LocalMode::Compare {
                    base: "main".into(),
                    head: "HEAD".into(),
                },
                "committed",
            ),
            (LocalMode::Staged, "staged"),
            (LocalMode::WorkingTree, "unstaged"),
        ] {
            let snapshot = reader
                .snapshot(root, mode, Cancellation::default())
                .unwrap();
            assert!(snapshot.remote.is_none());
            let file = snapshot
                .patch
                .files
                .iter()
                .find(|file| file.display_path() == "doc.md")
                .unwrap();
            assert!(
                file.hunks
                    .iter()
                    .flat_map(|h| &h.rows)
                    .any(|row| { row.kind == RowKind::Added && row.text.as_ref() == expected })
            );
            assert_eq!(repository_files(root), before, "{forge}: {expected}");
        }
    }
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

fn head_oid(repo: &Path) -> String {
    let output = Command::new("git")
        .arg("-C")
        .arg(repo)
        .args(["rev-parse", "HEAD"])
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .output()
        .unwrap();
    assert!(output.status.success(), "{:?}", output.stderr);
    String::from_utf8(output.stdout).unwrap().trim().into()
}

fn assert_submodule_format_survives_config(
    mode: LocalMode,
    program: PathBuf,
    enable_format: impl FnOnce(&Path),
) {
    let d = tempfile::tempdir().unwrap();
    let submodule = d.path().join("a-submodule");
    std::fs::create_dir(&submodule).unwrap();
    for root in [d.path(), submodule.as_path()] {
        git(root, &["init", "-b", "main"]);
        git(root, &["config", "user.email", "test@example.invalid"]);
        git(root, &["config", "user.name", "Test"]);
        git(root, &["config", "commit.gpgsign", "false"]);
    }
    std::fs::write(submodule.join("data.txt"), "old\n").unwrap();
    git(&submodule, &["add", "data.txt"]);
    git(&submodule, &["commit", "-m", "submodule base"]);
    let old_oid = head_oid(&submodule);
    std::fs::write(d.path().join("z.txt"), "old\n").unwrap();
    git(d.path(), &["add", "z.txt"]);
    git(
        d.path(),
        &[
            "update-index",
            "--add",
            "--cacheinfo",
            &format!("160000,{old_oid},a-submodule"),
        ],
    );
    git(d.path(), &["commit", "-m", "base"]);

    std::fs::write(submodule.join("data.txt"), "new\n").unwrap();
    git(&submodule, &["commit", "-am", "submodule change"]);
    let new_oid = head_oid(&submodule);
    std::fs::write(d.path().join("z.txt"), "new\n").unwrap();
    match &mode {
        LocalMode::Compare { .. } => {
            git(d.path(), &["add", "a-submodule", "z.txt"]);
            git(d.path(), &["commit", "-m", "change"]);
        }
        LocalMode::Staged => git(d.path(), &["add", "a-submodule", "z.txt"]),
        LocalMode::WorkingTree => {}
    }

    let g = LocalGit::new(program);
    git(d.path(), &["config", "diff.submodule", "short"]);
    let control = g
        .snapshot(d.path(), mode.clone(), Cancellation::default())
        .unwrap();
    assert_eq!(control.patch.files.len(), 2);
    let gitlink = control
        .patch
        .files
        .iter()
        .find(|file| file.path().bytes() == b"a-submodule")
        .unwrap();
    assert_eq!(gitlink.content, ContentKind::Submodule);
    assert_eq!(gitlink.old_mode.as_deref(), Some("160000"));
    assert_eq!(gitlink.new_mode.as_deref(), Some("160000"));
    assert_eq!(gitlink.old_oid.as_deref(), Some(old_oid.as_str()));
    assert_eq!(gitlink.new_oid.as_deref(), Some(new_oid.as_str()));
    assert!(
        control
            .patch
            .files
            .iter()
            .any(|file| file.path().bytes() == b"z.txt")
    );

    enable_format(d.path());
    // Include the submodule and superproject config, index, refs, objects, and worktree.
    let before = repository_files(d.path());
    let snapshot = g.snapshot(d.path(), mode, Cancellation::default());
    assert_eq!(repository_files(d.path()), before);
    assert_eq!(
        serde_json::to_vec(&snapshot.unwrap().patch).unwrap(),
        serde_json::to_vec(&control.patch).unwrap(),
    );
}

#[test]
fn configured_submodule_formats_preserve_gitlink_changes() {
    for format in ["log", "diff"] {
        for mode in [
            LocalMode::Compare {
                base: "HEAD~1".into(),
                head: "HEAD".into(),
            },
            LocalMode::Staged,
            LocalMode::WorkingTree,
        ] {
            assert_submodule_format_survives_config(
                mode,
                resolve_program("git").unwrap(),
                |repo| git(repo, &["config", "diff.submodule", format]),
            );
        }
    }
}

#[test]
#[cfg(unix)]
fn global_submodule_formats_preserve_gitlink_changes() {
    for format in ["log", "diff"] {
        let bin = tempfile::tempdir().unwrap();
        let config = bin.path().join("gitconfig");
        let settings = format!("[diff]\n\tsubmodule = {format}\n");
        std::fs::write(&config, &settings).unwrap();
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
        for mode in [
            LocalMode::Compare {
                base: "HEAD~1".into(),
                head: "HEAD".into(),
            },
            LocalMode::Staged,
            LocalMode::WorkingTree,
        ] {
            assert_submodule_format_survives_config(mode, program.clone(), |repo| {
                git(repo, &["config", "--unset", "diff.submodule"]);
            });
        }
        assert_eq!(std::fs::read_to_string(config).unwrap(), settings);
    }
}
