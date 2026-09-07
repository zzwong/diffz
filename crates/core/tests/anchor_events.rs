use diffz_core::anchor::*;
use diffz_core::domain::*;
use diffz_core::layout::*;
fn point() -> SourcePoint {
    SourcePoint {
        snapshot: SnapshotId("s".into()),
        file: FileId("f".into()),
        side: Side::Right,
        line: 42,
        byte_column: 15,
    }
}
#[test]
fn inserted_height_compensates_without_navigation() {
    let a = ViewportAnchor {
        point: point(),
        viewport_y: 20.0,
        horizontal: 0.0,
    };
    assert_eq!(restore_scroll(&a, 160.0, 1000.0, 200.0).unwrap(), 140.0);
}
#[test]
fn identical_geometry_does_not_move() {
    let a = ViewportAnchor {
        point: point(),
        viewport_y: 20.0,
        horizontal: 0.0,
    };
    assert_eq!(restore_scroll(&a, 120.0, 1000.0, 200.0).unwrap(), 100.0);
}
#[test]
fn stale_navigation_cannot_override_newer_input() {
    let mut n = NavigationClock::default();
    let old = n.generation();
    n.user_input();
    assert!(!n.accepts(old));
    assert!(n.accepts(n.generation()));
}
#[test]
fn soft_wrap_does_not_insert_newlines() {
    assert_eq!(
        copy_source_range(b"one long paragraph", 4..18).unwrap(),
        "long paragraph"
    );
}
#[test]
fn copy_preserves_crlf_and_hard_break_spaces() {
    assert_eq!(copy_source_range(b"a  \r\nb", 0..6).unwrap(), "a  \r\nb");
}
#[test]
fn copy_rejects_invalid_boundaries_and_reversed_ranges() {
    assert!(copy_source_range("é".as_bytes(), 1..2).is_err());
    assert!(copy_source_range(b"a", std::ops::Range { start: 1, end: 0 }).is_err());
}
#[test]
fn tab_expansion_maps_back_to_source() {
    let d = DisplayText::new("a\tb", 4);
    assert_eq!(d.text, "a   b");
    assert_eq!(d.source_byte(2), 1);
    assert_eq!(d.source_byte(4), 2);
    assert_eq!(d.display_byte(2), 4);
}
#[test]
fn split_filler_has_no_source_target() {
    assert_eq!(split_row_height(20.0, 60.0), 60.0);
    assert!(!in_source_height(45.0, 20.0));
}
#[test]
fn fragments_cover_source_once() {
    let m = MeasuredLine {
        fragments: vec![
            VisualFragment {
                bytes: 0..5,
                y: 0.0,
                height: 20.0,
            },
            VisualFragment {
                bytes: 5..10,
                y: 20.0,
                height: 20.0,
            },
        ],
        height: 40.0,
    };
    assert!(m.validate("hello rust").is_ok());
    assert_eq!(m.fragment_for_byte(7), Some(1));
}
#[test]
fn grapheme_snapping_keeps_combining_sequence() {
    let s = "a\u{301}b";
    assert_eq!(snap_grapheme(s, 1), 0);
    assert_eq!(snap_grapheme(s, 3), 3);
}

#[test]
fn plain_text_mapping_does_not_allocate_per_character() {
    let source = "x".repeat(100_000);
    let display = diffz_core::layout::DisplayText::new(&source, 4);
    assert_eq!(display.source_byte(50_000), 50_000);
    assert_eq!(display.display_byte(99_999), 99_999);
    assert!(display.estimated_bytes() <= source.len() * 2);
}
