use diffz_core::{domain::*, patch::*, presentation};

#[test]
fn display_rows_share_immutable_source_allocations() {
    let patch = parse_patch(
        "diff --git a/a.txt b/a.txt\n--- a/a.txt\n+++ b/a.txt\n@@ -1,2 +1,2 @@\n context café שלום\n-old\n+new\n".as_bytes(),
        ParseLimits::default(),
    ).unwrap();
    let file = &patch.files[0];
    for split in [false, true] {
        let rows = presentation::rows(file, split);
        for row in &rows {
            for side in [Side::Left, Side::Right] {
                if let Some(cell) = row.cell(side) {
                    let source = file.hunks[0]
                        .rows
                        .iter()
                        .find(|r| r.number(side) == Some(cell.number))
                        .unwrap();
                    assert_eq!(&*cell.text, &*source.text);
                    assert_eq!(
                        cell.text.as_ptr(),
                        source.text.as_ptr(),
                        "display must share source bytes"
                    );
                }
            }
        }
    }
}

#[test]
fn source_serialization_and_identity_survive_round_trip() {
    let patch = parse_patch(
        include_bytes!("../../../fixtures/unicode/change.patch"),
        ParseLimits::default(),
    )
    .unwrap();
    let snapshot = Snapshot::new("Unicode".into(), patch, None, vec![]);
    let json = serde_json::to_string(&snapshot).unwrap();
    let restored: Snapshot = serde_json::from_str(&json).unwrap();
    assert_eq!(restored.id, snapshot.id);
    assert!(restored.verify_identity());
    assert_eq!(serde_json::to_string(&restored).unwrap(), json);
}
