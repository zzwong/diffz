use diffz_adapters::store::Store;
use diffz_core::{
    domain::*,
    patch::{ParseLimits, parse_patch},
};
fn snapshot() -> Snapshot {
    Snapshot::new(
        "test".into(),
        parse_patch(
            b"diff --git a/a.md b/a.md\n--- a/a.md\n+++ b/a.md\n@@ -1 +1 @@\n-old\n+new\n",
            ParseLimits::default(),
        )
        .unwrap(),
        None,
        vec![],
    )
}
fn draft(s: &Snapshot) -> Draft {
    Draft {
        id: DraftId("d".into()),
        snapshot: s.id.clone(),
        file: s.patch.files[0].id.clone(),
        side: Side::Right,
        start_line: 1,
        line: 1,
        file_level: false,
        body: "keep me".into(),
        version: 2,
        saved_version: 0,
        published: false,
    }
}
#[test]
fn restart_preserves_draft() {
    let d = tempfile::tempdir().unwrap();
    let s = snapshot();
    {
        let db = Store::open(d.path()).unwrap();
        db.put_snapshot(&s).unwrap();
        assert_eq!(db.save_draft(draft(&s)).unwrap(), 2);
    }
    let db = Store::open(d.path()).unwrap();
    let ds = db.drafts(&s.id).unwrap();
    assert_eq!(ds[0].body, "keep me");
    assert!(ds[0].is_saved());
}
#[test]
fn stale_save_cannot_overwrite_new() {
    let d = tempfile::tempdir().unwrap();
    let db = Store::open(d.path()).unwrap();
    let s = snapshot();
    db.put_snapshot(&s).unwrap();
    db.save_draft(draft(&s)).unwrap();
    let mut old = draft(&s);
    old.version = 1;
    old.body = "stale".into();
    assert!(db.save_draft(old).is_err());
    assert_eq!(db.drafts(&s.id).unwrap()[0].body, "keep me");
}
#[test]
fn same_version_different_body_is_rejected() {
    let d = tempfile::tempdir().unwrap();
    let db = Store::open(d.path()).unwrap();
    let s = snapshot();
    db.put_snapshot(&s).unwrap();
    db.save_draft(draft(&s)).unwrap();
    let mut bad = draft(&s);
    bad.body = "different".into();
    assert!(db.save_draft(bad).is_err());
}
#[test]
fn one_writer_only() {
    let d = tempfile::tempdir().unwrap();
    let _a = Store::open(d.path()).unwrap();
    assert!(Store::open(d.path()).is_err());
}
#[test]
fn forged_snapshot_identity_is_rejected() {
    let d = tempfile::tempdir().unwrap();
    let db = Store::open(d.path()).unwrap();
    let mut s = snapshot();
    s.id = SnapshotId("spoof".into());
    assert!(db.put_snapshot(&s).is_err());
}
#[test]
fn settings_are_global_and_survive_restart() {
    let d = tempfile::tempdir().unwrap();
    {
        let db = Store::open(d.path()).unwrap();
        assert_eq!(db.settings().unwrap(), Settings::default());
        db.save_settings(&Settings {
            split: true,
            wrap: Some(false),
            font_size: 15.0,
            dark: false,
            theme: Some("tokyo-night".into()),
            rich: false,
            rich_inline: false,
        })
        .unwrap();
    }
    let db = Store::open(d.path()).unwrap();
    let s = db.settings().unwrap();
    assert!(s.split && !s.dark);
    assert_eq!(s.wrap, Some(false));
    assert_eq!(s.font_size, 15.0);
    assert_eq!(s.theme, Some("tokyo-night".into()));
}
