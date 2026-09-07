use diffz_core::{
    anchor::ViewportAnchor,
    domain::{Side, Snapshot, SourcePoint, SourceSelection},
    patch::{ParseLimits, parse_patch},
};
use diffz_ui::viewport::Viewport;
use gpui_kit::{point, px};
use std::sync::Arc;

fn viewport() -> (Viewport, SourcePoint) {
    let patch = parse_patch(
        include_bytes!("../../../fixtures/split-asymmetric/change.patch"),
        ParseLimits::default(),
    )
    .unwrap();
    let snapshot = Arc::new(Snapshot::new("fixture".into(), patch, None, vec![]));
    let file = snapshot.patch.files[0].id.clone();
    let point = SourcePoint {
        snapshot: snapshot.id.clone(),
        file: file.clone(),
        side: Side::Left,
        line: 1,
        byte_column: 0,
    };
    let anchor = ViewportAnchor {
        point: point.clone(),
        viewport_y: 0.0,
        horizontal: 0.0,
    };
    (
        Viewport::new(
            snapshot,
            file,
            true,
            true,
            14.0,
            "Menlo".into(),
            Some(anchor),
        ),
        point,
    )
}
#[test]
fn scrolling_anchor_is_not_a_comment_selection() {
    let (view, _) = viewport();
    assert!(view.draft_selection().is_none());
}
#[test]
fn clicking_without_a_source_hit_clears_previous_selection() {
    let (mut view, source) = viewport();
    view.selection = Some(SourceSelection {
        start: source.clone(),
        end: source,
    });
    assert!(view.draft_selection().is_some());
    assert!(view.select_at(point(px(0.0), px(0.0))).is_none());
    assert!(view.draft_selection().is_none());
}

#[test]
fn keyboard_range_targets_original_lines_and_stays_on_one_side() {
    let (mut view, source) = viewport();
    view.selection = Some(SourceSelection {
        start: source.clone(),
        end: source,
    });
    view.move_selection(diffz_ui::viewport::Motion::Down, true);
    let selection = view.draft_selection().unwrap();
    assert_eq!(selection.start.line, 1);
    assert_eq!(selection.end.line, 2);
    assert_eq!(selection.start.side, Side::Left);
    assert_eq!(selection.end.side, Side::Left);
    view.move_selection(diffz_ui::viewport::Motion::Home, false);
    let selection = view.draft_selection().unwrap();
    assert_eq!(selection.start, selection.end);
    assert_eq!(selection.end.byte_column, 0);
}
