use diffz_core::{domain::FileId, file_tree::FileTree};
use std::collections::HashSet;
#[test]
fn nested_paths_are_compacted_sorted_and_collapsible() {
    let entries = vec![
        (FileId("b".into()), "src/deep/b.rs".into()),
        (FileId("a".into()), "src/deep/a.rs".into()),
        (FileId("r".into()), "README.md".into()),
    ];
    let tree = FileTree::new(entries);
    let rows = tree.rows(&HashSet::new(), "");
    assert_eq!(
        rows.iter().map(|r| r.label.as_str()).collect::<Vec<_>>(),
        vec!["src/deep", "a.rs", "b.rs", "README.md"]
    );
    let rows = tree.rows(&HashSet::from(["src/deep".into()]), "");
    assert_eq!(rows.len(), 2);
    let rows = tree.rows(&HashSet::from(["src/deep".into()]), "a.rs");
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[1].file, Some(FileId("a".into())));
}
#[test]
fn filtering_keeps_ancestors_and_no_unrelated_files() {
    let tree = FileTree::new(vec![
        (FileId("a".into()), "src/a.rs".into()),
        (FileId("b".into()), "tests/b.rs".into()),
    ]);
    let rows = tree.rows(&HashSet::new(), "tests/b");
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[0].label, "tests");
    assert_eq!(rows[1].depth, 1);
}
