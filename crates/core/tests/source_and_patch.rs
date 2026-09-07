use diffz_core::domain::{LineEnding, RepoPath, Side, SourceDocument};
use diffz_core::inline::word_diff;
use diffz_core::patch::{ChangeKind, ContentKind, ParseLimits, PatchError, parse_patch};

fn fixture(name: &str) -> Vec<u8> {
    std::fs::read(format!(
        "{}/../../fixtures/{name}/change.patch",
        env!("CARGO_MANIFEST_DIR")
    ))
    .unwrap()
}
#[test]
fn source_bytes_and_endings_are_not_normalized() {
    let d = SourceDocument::from_utf8(b"a\r\nb".to_vec()).unwrap();
    assert_eq!(d.raw_bytes(), b"a\r\nb");
    assert_eq!(d.line_text(1), Some("a"));
    assert_eq!(d.lines()[0].ending, LineEnding::CrLf);
    assert_eq!(d.lines()[1].ending, LineEnding::None);
    assert_eq!(d.line_text(0), None);
}
#[test]
fn no_phantom_eof_line() {
    for (s, n) in [("", 0), ("\n", 1), ("a\n", 1), ("a\n\n", 2), ("a\nb", 2)] {
        assert_eq!(
            SourceDocument::from_utf8(s.as_bytes().to_vec())
                .unwrap()
                .lines()
                .len(),
            n
        );
    }
}
#[test]
fn source_rejects_invalid_utf8() {
    assert!(SourceDocument::from_utf8(vec![255]).is_err());
}
#[test]
fn source_positions_validate_boundaries() {
    let d = SourceDocument::from_utf8("é\t中\r\nz".as_bytes().to_vec()).unwrap();
    assert!(d.offset(1, 1).is_err());
    assert_eq!(d.offset(1, 2).unwrap(), 2);
    assert_eq!(d.offset(2, 0).unwrap(), 8);
}
#[test]
fn ordinary_fixture_patches_parse() {
    for name in [
        "markdown-prose",
        "markdown-url",
        "unicode",
        "crlf-to-lf",
        "eof-newline",
        "markdown-whitespace",
        "split-asymmetric",
        "quoted-path",
        "mode-only",
        "rename-only",
        "large-line",
        "many-lines",
    ] {
        let p = parse_patch(&fixture(name), ParseLimits::default())
            .unwrap_or_else(|e| panic!("{name}: {e}"));
        assert!(!p.files.is_empty(), "{name}");
    }
}
#[test]
fn malformed_counts_are_not_complete() {
    assert!(matches!(
        parse_patch(&fixture("malformed-hunk"), ParseLimits::default()),
        Err(PatchError::Malformed { .. })
    ));
}
#[test]
fn combined_is_explicitly_unsupported() {
    assert!(matches!(
        parse_patch(&fixture("combined-diff"), ParseLimits::default()),
        Err(PatchError::Unsupported(_))
    ));
}
#[test]
fn parser_limits_fail_before_dropping_rows() {
    let limits = ParseLimits {
        max_rows: 1,
        ..ParseLimits::default()
    };
    assert!(matches!(
        parse_patch(&fixture("markdown-prose"), limits),
        Err(PatchError::Limit(_))
    ));
}
#[test]
fn eof_metadata_survives() {
    let p = parse_patch(&fixture("eof-newline"), ParseLimits::default()).unwrap();
    assert!(
        p.files[0]
            .hunks
            .iter()
            .flat_map(|h| &h.rows)
            .any(|r| r.ending == LineEnding::None)
    );
}
#[test]
fn rename_and_mode_only_are_visible() {
    let p = parse_patch(&fixture("rename-only"), ParseLimits::default()).unwrap();
    assert_eq!(p.files[0].kind, ChangeKind::Renamed);
    let p = parse_patch(&fixture("mode-only"), ParseLimits::default()).unwrap();
    assert_ne!(p.files[0].old_mode, p.files[0].new_mode);
}
#[test]
fn binary_is_not_a_text_change() {
    let p = parse_patch(
        b"diff --git a/p.png b/p.png\nindex a..b 100644\nBinary files a/p.png and b/p.png differ\n",
        ParseLimits::default(),
    )
    .unwrap();
    assert_eq!(p.files[0].content, ContentKind::Binary);
}
#[test]
fn counted_hunk_ignores_header_looking_source() {
    let p = parse_patch(
        b"diff --git a/a b/a\n--- a/a\n+++ b/a\n@@ -1 +1 @@\n--- source\n+++ source\n",
        ParseLimits::default(),
    )
    .unwrap();
    assert_eq!(p.files[0].hunks[0].rows[0].text, "-- source");
    assert_eq!(p.files[0].hunks[0].rows[1].text, "++ source");
}
#[test]
fn duplicate_paths_are_rejected() {
    let patch = b"diff --git a/a b/a\nold mode 100644\nnew mode 100755\n";
    assert!(
        parse_patch(
            &[patch.as_slice(), patch.as_slice()].concat(),
            ParseLimits::default()
        )
        .is_err()
    );
}
#[test]
fn line_counts_cannot_overflow() {
    assert!(
        parse_patch(
            b"diff --git a/a b/a\n--- a/a\n+++ b/a\n@@ -4294967295,2 +1 @@\n-a\n-b\n+c\n",
            ParseLimits::default()
        )
        .is_err()
    );
}
#[test]
fn c_quoted_paths_preserve_raw_bytes() {
    assert_eq!(
        RepoPath::from_git_token(br#""a/na\303\257ve\tname""#)
            .unwrap()
            .bytes(),
        "a/naïve\tname".as_bytes()
    );
    assert!(RepoPath::from_git_token(br#""bad\999""#).is_err());
}
#[test]
fn side_coordinate_is_source_not_visual_index() {
    let p = parse_patch(&fixture("markdown-prose"), ParseLimits::default()).unwrap();
    let f = &p.files[0];
    for h in &f.hunks {
        for r in &h.rows {
            if let Some(n) = r.new_line {
                assert_eq!(f.line(Side::Right, n).unwrap().text, r.text);
            }
        }
    }
}
#[test]
fn word_highlighting_is_bounded_and_utf8_safe() {
    let (a, b) = word_diff("let naïve = 1;", "let brave = 1;");
    assert!(!a.is_empty() && !b.is_empty());
    for r in a {
        assert!("let naïve = 1;".get(r).is_some());
    }
    assert_eq!(word_diff(&"x".repeat(4097), "x"), (vec![], vec![]));
}

#[test]
fn identical_local_patches_in_different_repositories_do_not_share_drafts() {
    let p = diffz_core::patch::parse_patch(b"", diffz_core::patch::ParseLimits::default()).unwrap();
    let a = diffz_core::domain::Snapshot::with_origin(
        "A".into(),
        p.clone(),
        None,
        vec![],
        "local:/a:staged".into(),
    );
    let b = diffz_core::domain::Snapshot::with_origin(
        "B".into(),
        p,
        None,
        vec![],
        "local:/b:staged".into(),
    );
    assert_ne!(a.id, b.id);
    assert!(a.verify_identity());
}

#[test]
fn marker_separator_is_not_part_of_quoted_path() {
    let patch = parse_patch(&fixture("quoted-path"), ParseLimits::default()).unwrap();
    assert_eq!(
        patch.files[0].old_path.as_ref().unwrap().bytes(),
        "docs/quote \"name\" ü.md".as_bytes()
    );
    assert_eq!(
        patch.files[0].new_path.as_ref().unwrap().bytes(),
        "docs/quote \"name\" ü.md".as_bytes()
    );
}
