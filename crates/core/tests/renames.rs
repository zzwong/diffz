use diffz_core::domain::{FileId, RepoPath};
use diffz_core::file_tree::FileTree;
use diffz_core::patch::{ChangeKind, MoveLabel, ParseLimits, parse_patch};
use std::collections::{HashMap, HashSet};

fn parse(text: &str) -> diffz_core::patch::PatchReport {
    parse_patch(text.as_bytes(), ParseLimits::default()).unwrap()
}

#[test]
fn similarity_and_dissimilarity_indexes_are_read() {
    let r = parse(
        "diff --git a/a b/b\nsimilarity index 87%\nrename from a\nrename to b\n\
         @@ -1 +1 @@\n-x\n+y\n\
         diff --git a/c b/c\ndissimilarity index 60%\n@@ -1 +1 @@\n-x\n+y\n",
    );
    assert_eq!(r.files[0].similarity(), Some(87));
    assert_eq!(r.files[0].kind, ChangeKind::Renamed);
    assert_eq!(r.files[1].dissimilarity(), Some(60));
    assert_eq!(r.files[1].similarity(), None);
}

#[test]
fn reading_similarity_leaves_stored_bytes_unchanged() {
    // The header stays in `metadata`, so snapshot identities from before stay valid.
    let r = parse("diff --git a/a b/b\nsimilarity index 87%\nrename from a\nrename to b\n");
    assert_eq!(
        r.files[0].metadata,
        vec!["similarity index 87%".to_string()]
    );
    let json = serde_json::to_string(&r.files[0]).unwrap();
    assert!(!json.contains("dissimilarity"));
}

#[test]
fn pure_rename_explains_the_empty_reader() {
    let r = parse(
        &String::from_utf8(std::fs::read("../../fixtures/rename-only/change.patch").unwrap())
            .unwrap(),
    );
    let f = &r.files[0];
    assert!(f.hunks.is_empty());
    assert_eq!(f.similarity(), Some(100));
    assert_eq!(
        f.empty_note().unwrap(),
        "Renamed from old.md, contents unchanged (100% similar)"
    );
}

#[test]
fn copy_mode_and_binary_notes() {
    let r = parse(
        "diff --git a/a b/b\nsimilarity index 100%\ncopy from a\ncopy to b\n\
         diff --git a/run.sh b/run.sh\nold mode 100644\nnew mode 100755\n\
         diff --git a/i.png b/i.png\nindex 1..2 100644\nBinary files a/i.png and b/i.png differ\n\
         diff --git a/e b/e\nindex 1..2 100644\n",
    );
    assert_eq!(
        r.files[0].empty_note().unwrap(),
        "Copied from a, contents unchanged (100% similar)"
    );
    assert_eq!(
        r.files[1].empty_note().unwrap(),
        "Mode changed 100644 → 100755"
    );
    assert_eq!(r.files[2].empty_note().unwrap(), "Binary file changed");
    assert_eq!(r.files[3].empty_note(), None);
}

#[test]
fn files_with_hunks_have_no_note() {
    let r = parse("diff --git a/a b/b\nrename from a\nrename to b\n@@ -1 +1 @@\n-x\n+y\n");
    assert_eq!(r.files[0].empty_note(), None);
    assert_eq!(
        r.files[0].moved_from(),
        Some(&RepoPath::new(b"a".to_vec()).unwrap())
    );
}

fn label(old: &str, new: &str) -> String {
    MoveLabel::new(old, new).plain()
}

#[test]
fn move_labels_fold_what_the_paths_share() {
    // Same folder, different name.
    assert_eq!(label("src/old.rs", "src/new.rs"), "src/{old.rs → new.rs}");
    // Different folder, same name.
    assert_eq!(label("a/x/foo.rs", "a/y/foo.rs"), "a/{x → y}/foo.rs");
    // Nothing in common.
    assert_eq!(label("a/one.rs", "b/two.rs"), "a/one.rs → b/two.rs");
    // Root level.
    assert_eq!(label("a.rs", "b.rs"), "a.rs → b.rs");
    assert_eq!(label("foo.rs", "sub/foo.rs"), "foo.rs → sub/foo.rs");
    // A file becomes a module directory.
    assert_eq!(
        label("crates/ui/src/panels.rs", "crates/ui/src/panels/mod.rs"),
        "crates/ui/src/{panels.rs → panels/mod.rs}"
    );
    // Nested common suffix.
    assert_eq!(
        label("src/a/deep/x/mod.rs", "src/b/deep/x/mod.rs"),
        "src/{a → b}/deep/x/mod.rs"
    );
    // Names sharing only letters are not split mid-word.
    assert_eq!(
        label("src/foobar.rs", "src/foobaz.rs"),
        "src/{foobar.rs → foobaz.rs}"
    );
    // Multibyte names never split a character.
    assert_eq!(label("d/é.rs", "d/è.rs"), "d/{é.rs → è.rs}");
}

#[test]
fn move_label_parts_are_exposed_for_styling() {
    let l = MoveLabel::new("a/x/foo.rs", "a/y/foo.rs");
    assert_eq!(
        (
            l.prefix.as_str(),
            l.old.as_str(),
            l.new.as_str(),
            l.suffix.as_str()
        ),
        ("a/", "x", "y", "/foo.rs")
    );
    assert!(l.folded());
    assert!(!MoveLabel::new("a.rs", "b.rs").folded());
}

#[test]
fn old_stored_remote_targets_load_as_not_merged_and_keep_their_bytes() {
    use diffz_core::domain::{ProviderId, RemoteTarget, RepositoryKey};
    let mut t = RemoteTarget {
        provider: ProviderId::GITHUB,
        repository: RepositoryKey {
            host: "github.com".into(),
            id: 1,
            owner: "o".into(),
            name: "r".into(),
        },
        account: "alice".into(),
        pr: 3,
        target_tip: "a".repeat(40),
        comparison_base: "a".repeat(40),
        head: "b".repeat(40),
        open: false,
        draft: false,
        merged: false,
        pending_review: false,
        compare: None,
    };
    let old = serde_json::to_string(&t).unwrap();
    assert!(!old.contains("merged"), "an unset flag is not stored");
    assert!(!serde_json::from_str::<RemoteTarget>(&old).unwrap().merged);
    t.merged = true;
    let new = serde_json::to_string(&t).unwrap();
    assert!(serde_json::from_str::<RemoteTarget>(&new).unwrap().merged);
}

#[test]
fn filter_matches_the_old_path_of_a_moved_file() {
    let entries = vec![
        (FileId("m".into()), "src/panels/mod.rs".into()),
        (FileId("o".into()), "src/other.rs".into()),
    ];
    let tree = FileTree::new(entries).with_old_paths(HashMap::from([(
        FileId("m".into()),
        "src/panels.rs".to_string(),
    )]));
    let rows = tree.rows(&HashSet::new(), "panels.rs");
    let files: Vec<_> = rows.iter().filter_map(|r| r.file.clone()).collect();
    assert_eq!(files, vec![FileId("m".into())]);
    assert_eq!(
        rows.last().unwrap().old_path.as_deref(),
        Some("src/panels.rs")
    );
    assert!(tree.rows(&HashSet::new(), "nomatch").is_empty());
}
