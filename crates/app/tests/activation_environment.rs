//! Exercise real process-entry removal without mutating the parallel test runner.
#![cfg(target_os = "linux")]
use std::{
    ffi::OsString,
    os::unix::{ffi::OsStringExt, fs::PermissionsExt},
    process::Command,
};

#[test]
fn provider_children_never_inherit_startup_activation_metadata() {
    for token in [
        OsString::from("synthetic-entry-token"),
        OsString::new(),
        OsString::from("x".repeat(4097)),
        OsString::from_vec(vec![0xff]),
    ] {
        let dir = tempfile::tempdir().unwrap();
        let bin = dir.path().join("bin");
        let repo = dir.path().join("repo");
        std::fs::create_dir(&bin).unwrap();
        std::fs::create_dir(&repo).unwrap();
        let marker = dir.path().join("inherited");
        let git = bin.join("git");
        std::fs::write(&git, "#!/bin/sh\nprintf '%s' \"${XDG_ACTIVATION_TOKEN+present}\" > \"$DIFFZ_TEST_ENV_MARKER\"\nexit 1\n").unwrap();
        std::fs::set_permissions(&git, std::fs::Permissions::from_mode(0o700)).unwrap();
        let result = Command::new(env!("CARGO_BIN_EXE_diffz"))
            .args(["--inspect", "--git"])
            .arg(&repo)
            .arg("--state-dir")
            .arg(dir.path().join("state"))
            .env("PATH", &bin)
            .env("DIFFZ_TEST_ENV_MARKER", &marker)
            .env("XDG_ACTIVATION_TOKEN", token)
            .output()
            .unwrap();
        assert!(!result.status.success());
        assert!(std::fs::read(&marker).unwrap().is_empty());
        assert!(!String::from_utf8_lossy(&result.stderr).contains("synthetic-entry-token"));
        assert!(!String::from_utf8_lossy(&result.stdout).contains("synthetic-entry-token"));
    }
}
