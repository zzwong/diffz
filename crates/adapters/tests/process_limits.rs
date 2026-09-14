use diffz_adapters::process::{ProcessRequest, Runner};
use diffz_core::provider::Cancellation;
use std::{path::PathBuf, time::Duration};
fn req(code: &str) -> ProcessRequest {
    let mut r = ProcessRequest::new(PathBuf::from("/bin/sh"));
    r.args = vec!["-c".into(), code.into()];
    r.deadline = Duration::from_secs(2);
    r
}
#[test]
fn drains_both_streams() {
    let r = Runner::run(
        req("printf 'abc';printf 'def' >&2"),
        Cancellation::default(),
    )
    .unwrap();
    assert_eq!(r.stdout, b"abc");
    assert_eq!(r.stderr, b"def");
}
#[test]
fn timeout_reaps() {
    let mut request = req("sleep 9");
    request.deadline = Duration::from_millis(300);
    let r = Runner::run(request, Cancellation::default());
    assert!(r.unwrap_err().to_string().contains("deadline"));
}
#[test]
fn cancel_before_spawn() {
    let c = Cancellation::default();
    c.cancel();
    assert!(
        Runner::run(req("exit 0"), c)
            .unwrap_err()
            .to_string()
            .contains("cancel")
    );
}
#[test]
fn output_cap_is_not_truncation() {
    let mut r = req("printf '12345678'");
    r.stdout_limit = 4;
    assert!(
        Runner::run(r, Cancellation::default())
            .unwrap_err()
            .to_string()
            .contains("limit")
    );
}
#[test]
fn stdin_roundtrips() {
    let mut r = req("cat");
    r.stdin = b"line1\nprivate\0data".to_vec();
    assert_eq!(
        Runner::run(r.clone(), Cancellation::default())
            .unwrap()
            .stdout,
        r.stdin
    );
}
#[test]
fn argv_is_not_shell_interpolated() {
    let mut r = ProcessRequest::new(PathBuf::from("/usr/bin/printf"));
    r.args = vec!["%s".into(), "$(touch NEVER_CREATED); *.rs".into()];
    assert_eq!(
        Runner::run(r, Cancellation::default()).unwrap().stdout,
        b"$(touch NEVER_CREATED); *.rs"
    );
}
