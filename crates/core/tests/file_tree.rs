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
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].file, Some(FileId("a".into())));
    assert_eq!(rows[0].prefix, "src/deep/");
}
#[test]
fn filtering_keeps_ancestors_and_no_unrelated_files() {
    let tree = FileTree::new(vec![
        (FileId("a".into()), "src/a.rs".into()),
        (FileId("b".into()), "tests/b.rs".into()),
    ]);
    let rows = tree.rows(&HashSet::new(), "tests/b");
    assert_eq!(rows.len(), 1);
    assert_eq!(
        (rows[0].prefix.as_str(), rows[0].label.as_str()),
        ("tests/", "b.rs")
    );
    assert_eq!(rows[0].depth, 0);

    let tree = FileTree::new(vec![
        (FileId("a".into()), "src/a.rs".into()),
        (FileId("b".into()), "src/b.rs".into()),
        (FileId("c".into()), "tests/c.rs".into()),
    ]);
    let rows = tree.rows(&HashSet::new(), ".rs");
    assert_eq!(rows.len(), 4);
    assert_eq!(
        (rows[0].label.as_str(), rows[3].prefix.as_str()),
        ("src", "tests/")
    );
}

fn shape(rows: &[diffz_core::file_tree::TreeRow]) -> Vec<(usize, String, String, bool)> {
    rows.iter()
        .map(|r| (r.depth, r.prefix.clone(), r.label.clone(), r.file.is_some()))
        .collect()
}

#[test]
fn single_file_folders_fold_into_the_file_row() {
    let entries = [
        "crates/ui/src/app/profile.rs",
        "crates/ui/src/reader.rs",
        "crates/ui/src/reader/command_handlers.rs",
        "crates/ui/src/keyboard.rs",
        "docs/performance.md",
        "CHANGELOG.md",
    ]
    .iter()
    .map(|p| (FileId((*p).into()), (*p).to_string()))
    .collect();
    let rows = FileTree::new(entries).rows(&HashSet::new(), "");
    let want = |d: usize, p: &str, l: &str, f: bool| (d, p.to_string(), l.to_string(), f);
    assert_eq!(
        shape(&rows),
        vec![
            want(0, "", "crates/ui/src", false),
            want(1, "app/", "profile.rs", true),
            want(1, "reader/", "command_handlers.rs", true),
            want(1, "", "keyboard.rs", true),
            want(1, "", "reader.rs", true),
            want(0, "docs/", "performance.md", true),
            want(0, "", "CHANGELOG.md", true),
        ]
    );
    assert_eq!(rows[1].path, "crates/ui/src/app/profile.rs");
}

#[test]
fn lone_file_in_a_deep_chain_is_one_row() {
    let tree = FileTree::new(vec![(FileId("f".into()), "a/b/c/f.rs".into())]);
    let rows = tree.rows(&HashSet::new(), "");
    assert_eq!(rows.len(), 1);
    assert_eq!(
        (rows[0].prefix.as_str(), rows[0].label.as_str()),
        ("a/b/c/", "f.rs")
    );
}

#[test]
fn folders_with_more_than_one_entry_keep_their_row() {
    let tree = FileTree::new(vec![
        (FileId("1".into()), "two/a.rs".into()),
        (FileId("2".into()), "two/b.rs".into()),
        (FileId("3".into()), "mixed/x.rs".into()),
        (FileId("4".into()), "mixed/sub/y.rs".into()),
    ]);
    let rows = tree.rows(&HashSet::new(), "");
    assert_eq!(
        shape(&rows),
        vec![
            (0, "".into(), "mixed".into(), false),
            (1, "sub/".into(), "y.rs".into(), true),
            (1, "".into(), "x.rs".into(), true),
            (0, "".into(), "two".into(), false),
            (1, "".into(), "a.rs".into(), true),
            (1, "".into(), "b.rs".into(), true),
        ]
    );
}
