use diffz_adapters::export::write_private_json;
#[test]
fn no_overwrite() {
    let d = tempfile::tempdir().unwrap();
    let p = d.path().join("context.json");
    write_private_json(&p, &serde_json::json!({"text":"中文\nhello"})).unwrap();
    assert!(write_private_json(&p, &serde_json::json!({})).is_err());
}
#[cfg(unix)]
#[test]
fn reject_symlink_target() {
    let d = tempfile::tempdir().unwrap();
    let p = d.path().join("target");
    std::fs::write(&p, b"original").unwrap();
    let link = d.path().join("link");
    std::os::unix::fs::symlink(&p, &link).unwrap();
    assert!(write_private_json(&link, &serde_json::json!({})).is_err());
    assert_eq!(std::fs::read(&p).unwrap(), b"original");
}
