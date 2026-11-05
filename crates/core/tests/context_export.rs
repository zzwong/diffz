use diffz_core::{
    domain::{RepoPath, Side, Snapshot, SourcePoint, SourceSelection},
    export::ContextExport,
    patch::{ParseLimits, parse_patch},
};

fn snapshot(patch: &[u8]) -> Snapshot {
    Snapshot::new(
        "selected context".into(),
        parse_patch(patch, ParseLimits::default()).unwrap(),
        None,
        vec![],
    )
}

fn selection(snapshot: &Snapshot, side: Side) -> SourceSelection {
    let file = &snapshot.patch.files[0];
    let row = file.line(side, 1).unwrap();
    let point = |byte_column| SourcePoint {
        snapshot: snapshot.id.clone(),
        file: file.id.clone(),
        side,
        line: 1,
        byte_column,
    };
    SourceSelection {
        start: point(0),
        end: point(row.text.len()),
    }
}

fn assert_export(snapshot: &Snapshot, side: Side, path: &[u8], text: &str) {
    let export = ContextExport::selected(
        snapshot,
        selection(snapshot, side),
        vec!["local note".into()],
    )
    .unwrap();
    assert_eq!(export.source_path_bytes, path);
    assert_eq!(
        export.source_path,
        RepoPath::new(path.to_vec()).unwrap().display()
    );
    assert_eq!(export.text, text);
    assert_eq!(export.selection.start.side, side);
    assert_eq!(export.snapshot, snapshot.id);
    assert_eq!(export.schema_version, 1);
    assert_eq!(export.notes, vec!["local note"]);
    let json = serde_json::to_string(&export).unwrap();
    let restored: ContextExport = serde_json::from_str(&json).unwrap();
    assert_eq!(restored.source_path_bytes, path);
    assert_eq!(restored.source_path, export.source_path);
    assert_eq!(restored.text, text);
}

#[test]
fn renamed_file_exports_the_path_on_the_selected_side() {
    let snapshot = snapshot(
        b"diff --git a/old.rs b/new.rs\nsimilarity index 50%\nrename from old.rs\nrename to new.rs\n--- a/old.rs\n+++ b/new.rs\n@@ -1 +1 @@\n-old\n+new\n",
    );
    assert_export(&snapshot, Side::Left, b"old.rs", "old");
    assert_export(&snapshot, Side::Right, b"new.rs", "new");
}

#[test]
fn copied_file_exports_the_path_on_the_selected_side() {
    let snapshot = snapshot(
        b"diff --git a/original.rs b/copy.rs\nsimilarity index 50%\ncopy from original.rs\ncopy to copy.rs\n--- a/original.rs\n+++ b/copy.rs\n@@ -1 +1 @@\n-old\n+new\n",
    );
    assert_export(&snapshot, Side::Left, b"original.rs", "old");
    assert_export(&snapshot, Side::Right, b"copy.rs", "new");
}

#[test]
fn renamed_non_utf8_paths_keep_exact_selected_bytes() {
    let snapshot = snapshot(
        b"diff --git \"a/old\\377.rs\" \"b/new\\376.rs\"\nsimilarity index 50%\nrename from \"old\\377.rs\"\nrename to \"new\\376.rs\"\n--- \"a/old\\377.rs\"\n+++ \"b/new\\376.rs\"\n@@ -1 +1 @@\n-old\n+new\n",
    );
    assert_export(&snapshot, Side::Left, b"old\xff.rs", "old");
    assert_export(&snapshot, Side::Right, b"new\xfe.rs", "new");
}

#[test]
fn unchanged_added_and_deleted_paths_still_export() {
    let modified = snapshot(
        b"diff --git a/same.rs b/same.rs\n--- a/same.rs\n+++ b/same.rs\n@@ -1 +1 @@\n-old\n+new\n",
    );
    assert_export(&modified, Side::Left, b"same.rs", "old");
    assert_export(&modified, Side::Right, b"same.rs", "new");

    let added = snapshot(
        b"diff --git a/added.rs b/added.rs\nnew file mode 100644\n--- /dev/null\n+++ b/added.rs\n@@ -0,0 +1 @@\n+new\n",
    );
    assert_export(&added, Side::Right, b"added.rs", "new");

    let deleted = snapshot(
        b"diff --git a/deleted.rs b/deleted.rs\ndeleted file mode 100644\n--- a/deleted.rs\n+++ /dev/null\n@@ -1 +0,0 @@\n-old\n",
    );
    assert_export(&deleted, Side::Left, b"deleted.rs", "old");
}

#[test]
fn reversed_left_selection_keeps_the_old_path() {
    let snapshot = snapshot(
        b"diff --git a/old.rs b/new.rs\nrename from old.rs\nrename to new.rs\n--- a/old.rs\n+++ b/new.rs\n@@ -1 +1 @@\n-old\n+new\n",
    );
    let mut selection = selection(&snapshot, Side::Left);
    std::mem::swap(&mut selection.start, &mut selection.end);
    let export = ContextExport::selected(&snapshot, selection, vec![]).unwrap();
    assert_eq!(export.source_path_bytes, b"old.rs");
    assert_eq!(export.text, "old");
}

#[test]
fn mixed_side_selection_remains_rejected() {
    let snapshot = snapshot(
        b"diff --git a/old.rs b/new.rs\nrename from old.rs\nrename to new.rs\n--- a/old.rs\n+++ b/new.rs\n@@ -1 +1 @@\n-old\n+new\n",
    );
    let mut selection = selection(&snapshot, Side::Left);
    selection.end.side = Side::Right;
    assert!(ContextExport::selected(&snapshot, selection, vec![]).is_err());
}

#[test]
fn selected_side_without_a_path_is_rejected() {
    let original = snapshot(
        b"diff --git a/old.rs b/new.rs\nrename from old.rs\nrename to new.rs\n--- a/old.rs\n+++ b/new.rs\n@@ -1 +1 @@\n-old\n+new\n",
    );
    for side in [Side::Left, Side::Right] {
        let mut patch = original.patch.clone();
        match side {
            Side::Left => patch.files[0].old_path = None,
            Side::Right => patch.files[0].new_path = None,
        }
        let snapshot = Snapshot::new("missing side path".into(), patch, None, vec![]);
        let error =
            ContextExport::selected(&snapshot, selection(&snapshot, side), vec![]).unwrap_err();
        assert_eq!(error, "selected source side has no file path");
    }
}
