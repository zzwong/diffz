#![cfg(feature = "desktop")]
use std::{
    process::Command,
    time::{SystemTime, UNIX_EPOCH},
};

#[test]
#[ignore = "requires a real native desktop and fonts"]
fn unicode_native_geometry_succeeds() {
    let output = std::env::temp_dir().join(format!(
        "review-probe-{}-{}.json",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let status = Command::new(env!("CARGO_BIN_EXE_diffz"))
        .args([
            "--probe",
            concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/../../fixtures/unicode/after.txt"
            ),
            "--probe-output",
        ])
        .arg(&output)
        .status()
        .unwrap();
    let report: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&output).unwrap()).unwrap();
    std::fs::remove_file(output).unwrap();
    assert_eq!(report["status"], "native_geometry_passed");
    assert!(
        report["bidi_checks"]
            .as_array()
            .unwrap()
            .iter()
            .all(|case| case["passed"] == true)
    );
    assert!(status.success());
}
