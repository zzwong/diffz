use diffz_adapters::{
    local_git::{LocalGit, LocalMode},
    process::resolve_program,
};
use diffz_core::provider::Cancellation;
use std::process::Command;
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
#[test]
#[cfg(unix)]
fn symlinked_git_that_points_into_the_repository_is_rejected() {
    use std::os::unix::fs::PermissionsExt;
    let repo = tempfile::tempdir().unwrap();
    git(repo.path(), &["init", "-b", "main"]);
    let inner = repo.path().join("git");
    std::fs::write(&inner, "#!/bin/sh\nexec git \"$@\"\n").unwrap();
    std::fs::set_permissions(&inner, std::fs::Permissions::from_mode(0o700)).unwrap();
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
