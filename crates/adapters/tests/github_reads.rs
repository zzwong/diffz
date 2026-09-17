use diffz_adapters::github::{PrAddress, decode_http, encode_path};
#[test]
fn strict_url() {
    let a = PrAddress::parse("https://github.com/o/r/pull/123/files").unwrap();
    assert_eq!(a.number, 123);
    assert_eq!(a.host, "github.com");
    for bad in [
        "https://github.com/o/r/pull/12evil",
        "http://github.com/o/r/pull/1",
        "https://user:pass@github.com/o/r/pull/1",
        "https://github.com/o/r/pull/0",
        "https://github.com/o/r/pull/1/../../other",
        "https://github.com/o/r/pull/1?host=evil",
    ] {
        assert!(PrAddress::parse(bad).is_err(), "{bad}");
    }
}
#[test]
fn explicit_slug_only() {
    assert!(PrAddress::parse("o/r#42").is_ok());
    assert!(PrAddress::parse("42").is_err());
}
#[test]
fn http_is_parsed_not_grepped() {
    let x = decode_http(b"HTTP/2.0 200 OK\r\nContent-Type: text/plain\r\n\r\nbody contains 404")
        .unwrap();
    assert_eq!(x.status, 200);
    assert_eq!(x.body, b"body contains 404");
}
#[test]
fn reject_non_http() {
    assert!(decode_http(b"arbitrary error").is_err());
}
#[test]
fn path_encoder() {
    assert_eq!(
        encode_path("dir with space/naïve+file#1.rs"),
        "dir%20with%20space/na%C3%AFve%2Bfile%231.rs"
    );
}

#[test]
fn check_metadata_distinguishes_running_and_completed() {
    let running = diffz_adapters::github::check_row(
        &serde_json::json!({"name":"Build","status":"in_progress","conclusion":null}),
        "Workflow",
    );
    assert_eq!(running.status, "in_progress");
    assert!(running.conclusion.is_none());
    let failed = diffz_adapters::github::check_row(
        &serde_json::json!({"context":"Tests","state":"failure"}),
        "Status",
    );
    assert_eq!(failed.conclusion.as_deref(), Some("failure"));
}

#[test]
#[cfg(unix)]
fn failed_gh_run_reports_its_redacted_stderr() {
    use std::os::unix::fs::PermissionsExt;
    let temp = tempfile::tempdir().unwrap();
    let gh = temp.path().join("gh");
    std::fs::write(
        &gh,
        "#!/bin/sh\nprintf 'mise ERROR no tasks defined in ~\\ntoken ghp_abcdefghijklmnopqrstuvwxyz0123\\n' >&2\nexit 1\n",
    )
    .unwrap();
    std::fs::set_permissions(&gh, std::fs::Permissions::from_mode(0o700)).unwrap();
    let err = diffz_adapters::github::GithubReader::new(gh)
        .account("github.com", diffz_core::provider::Cancellation::default())
        .unwrap_err()
        .to_string();
    assert_eq!(
        err,
        "gh exited before a full HTTP response arrived (exit Some(1)); gh reported: mise ERROR no tasks defined in ~ token [redacted]; verify gh auth status covers this host"
    );
}
