use crate::{
    app::{Panel, PanelResizeState, Workbench},
    commands::{self, Command},
    viewport::ContextRequest,
};
use diffz_core::{domain::*, provider::OpenRequest};
use gpui_kit::component::{StyledExt, button::*, input::Input};
use gpui_kit::{prelude::*, *};

mod command_handlers;
mod file_panel;
mod files_peek;
mod render;

pub use files_peek::FilesPeek;

const FILES_EDGE_X: f32 = 24.;
const FILES_COLLAPSE_WIDTH: f32 = 150.;
const FILES_COLLAPSE_VELOCITY: f32 = -900.;
/// The pointer crosses a few titlebar pixels that belong to neither region on its way
/// from the toggle to the panel, so a leave has to wait before it closes the peek.
const FILES_PEEK_GRACE_MS: u64 = 200;
const FILES_PEEK_IN_MS: f32 = 180.;
const FILES_PEEK_OUT_MS: f32 = 120.;
const FILES_PEEK_SLIDE_PX: f32 = 16.;
/// A stalled frame must not carry the panel across the whole path in one step.
const FILES_PEEK_STEP_MS: f32 = 32.;
const FILES_PEEK_EDGE_PX: f32 = 8.;
/// Tiled windows put another window immediately left of this edge, so the pointer
/// crosses the zone on its way out. Only a pointer that rests there means the panel.
const FILES_PEEK_DWELL_MS: u64 = 100;

fn ease_out_cubic(t: f32) -> f32 {
    let remaining = 1. - t;
    1. - remaining * remaining * remaining
}

fn should_collapse_files(pointer_x: f32, raw_desired_width: f32, velocity_x: f32) -> bool {
    pointer_x <= FILES_EDGE_X
        || (raw_desired_width <= FILES_COLLAPSE_WIDTH && velocity_x <= FILES_COLLAPSE_VELOCITY)
}

fn effective_resize_velocity(velocity_x: f32, sample_age: std::time::Duration) -> f32 {
    if sample_age <= std::time::Duration::from_millis(100) {
        velocity_x
    } else {
        0.
    }
}

fn cancel_resize_on_unpressed_mouse(
    resizing_panel: &mut Option<PanelResizeState>,
    pressed_button: Option<MouseButton>,
) -> bool {
    if resizing_panel.is_some() && pressed_button != Some(MouseButton::Left) {
        *resizing_panel = None;
        true
    } else {
        false
    }
}

fn adjacent_file_index(index: usize, len: usize, forward: bool) -> Option<usize> {
    if index >= len {
        return None;
    }
    if forward {
        index.checked_add(1).filter(|next| *next < len)
    } else {
        index.checked_sub(1)
    }
}

impl Workbench {
    /// Move between changed files after scrolling reaches an edge; both rich and source
    /// views use this helper. Positive `direction` advances.
    pub(crate) fn turn_file(&mut self, direction: i8, window: &mut Window, cx: &mut Context<Self>) {
        let current = self.viewport.as_ref().map(|v| v.borrow().file.clone());
        let Some(at) = self
            .browser
            .visible_files
            .iter()
            .position(|id| Some(id) == current.as_ref())
        else {
            return;
        };
        let next = adjacent_file_index(at, self.browser.visible_files.len(), direction > 0);
        let Some(next) = next else {
            self.status = "End of the changed-file list".into();
            return;
        };
        self.select_file(self.browser.visible_files[next].clone(), cx);
        if let Some(v) = &self.viewport {
            v.borrow_mut().jump_edge(direction < 0);
        }
        self.diff_focus.focus(window, cx);
        self.status = format!("File {} of {}", next + 1, self.browser.visible_files.len());
    }
    /// Supply the wheel movement and whether the view is at a content boundary
    /// before deciding whether edge scrolling should begin.
    pub(crate) fn edge_of(delta_y: f32, at_start: bool, at_end: bool) -> i8 {
        if delta_y < 0. && at_start {
            -1
        } else if delta_y > 0. && at_end {
            1
        } else {
            0
        }
    }

    pub(crate) fn expand_context(&mut self, request: ContextRequest, cx: &mut Context<Self>) {
        if self.context_busy {
            return;
        }
        let Some(viewport) = self.viewport.clone() else {
            return;
        };
        let v = viewport.borrow();
        let Some((hunk, old, new, count)) = v.context_plan(request) else {
            self.status = "There is no additional hidden context in that direction".into();
            cx.notify();
            return;
        };
        let Some(target) = v.snapshot.remote.clone() else {
            self.status = "Context expansion needs a saved hosted review; standalone patches have no omitted source".into();
            cx.notify();
            return;
        };
        let Some(file) = v.snapshot.file(&v.file) else {
            return;
        };
        let Some((old_path, new_path)) = file
            .old_path
            .as_ref()
            .zip(file.new_path.as_ref())
            .and_then(|(a, b)| Some((a.utf8().ok()?.to_owned(), b.utf8().ok()?.to_owned())))
        else {
            return;
        };
        drop(v);
        self.context_busy = true;
        self.status = format!("Fetching {count} lines of context pinned to the revision…");
        let services = self.services.clone();
        cx.spawn(async move |this, cx| {
            let result = cx
                .background_spawn(async move {
                    let left = services.source_lines(
                        &target,
                        &old_path,
                        &target.comparison_base,
                        old,
                        count,
                    )?;
                    let right =
                        services.source_lines(&target, &new_path, &target.head, new, count)?;
                    if left != right {
                        return Err(diffz_core::provider::ServiceError::from(
                            "The skipped source text differs between revisions, so no context lines were inserted.",
                        ));
                    }
                    Ok(right)
                })
                .await;
            let _ = this.update(cx, |this, cx| {
                this.context_busy = false;
                if !this
                    .viewport
                    .as_ref()
                    .is_some_and(|v| std::rc::Rc::ptr_eq(v, &viewport))
                {
                    cx.notify();
                    return;
                }
                match result {
                    Ok(lines) => {
                        let count = lines.len();
                        viewport.borrow_mut().insert_context(
                            hunk,
                            request.above(),
                            old,
                            new,
                            lines,
                        );
                        this.status = format!(
                            "Added {count} unchanged lines · the extra context does not accept comments or draft selections"
                        );
                    }
                    Err(e) => this.status = e.to_string(),
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    fn reader(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let Some(v) = self.viewport.clone() else {
            return div()
                .v_flex()
                .size_full()
                .p_8()
                .gap_3()
                .child("diffz")
                .child("Open a source, or begin with the long Markdown fixture.")
                .child(
                    Button::new("demo")
                        .cursor_pointer()
                        .primary()
                        .label("Open Markdown fixture")
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.open(OpenRequest::Fixture("F01".into()), false, cx)
                        })),
                )
                .into_any_element();
        };
        if let Some(active) = &self.active {
            let mut viewport = v.borrow_mut();
            viewport.draft_lines = active
                .drafts
                .iter()
                .filter(|d| d.file == viewport.file)
                .map(|d| (d.side, d.line))
                .collect();
        }
        v.borrow_mut().pull = self.gesture.pull();
        let measure = v.clone();
        let paint = v.clone();
        let skin = self.skin();
        div()
            .id("source-viewport")
            .cursor(v.borrow().hover_cursor)
            .track_focus(&self.diff_focus)
            .key_context("WorkbenchDiff")
            .tab_stop(true)
            .aria_label("Diff viewer. Use arrows to navigate, Shift to extend, and C to comment on a selected line.")
            .relative()
            .size_full()
            .min_w_0()
            .min_h_0()
            .overflow_hidden()
            .on_hover(cx.listener(|this, hovered: &bool, _, cx| {
                if !hovered {
                    if let Some(v) = &this.viewport {
                        let mut v = v.borrow_mut();
                        v.hovered = None;
                        v.hover_cursor = CursorStyle::Arrow;
                    }
                    cx.notify();
                }
            }))
            .on_scroll_wheel(cx.listener(|this, event: &ScrollWheelEvent, window, cx| {
                this.wheel(event, crate::scrolling::ScrollTarget::Source, window, cx)
            }))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, event: &MouseDownEvent, window, cx| {
                    this.diff_focus.focus(window, cx);
                    let expansion = this.viewport.as_ref().and_then(|v| v.borrow_mut().context_hit(event.position));
                    if let Some(request) = expansion { this.expand_context(request, cx); return; }
                    let mut comment = false;
                    if let Some(v) = &this.viewport {
                        let mut v = v.borrow_mut();
                        if let Some(point) = v.comment_hit(event.position) {
                            v.selection=Some(SourceSelection{start:point.clone(),end:point});comment=true;
                        } else if let Some(point) = v.line_number_hit(event.position, window) {
                            v.select_source_line(point);
                            this.drag_start = None;
                        } else if let Some(f) = v.hit_horizontal_scrollbar(event.position) {
                            this.horizontal_drag = true;
                            v.horizontal = f * v.max_horizontal;
                        } else if let Some(f) = v.hit_scrollbar(event.position) {
                            this.scrollbar_drag = true;
                            v.jump_fraction(f);
                        } else {
                            this.drag_start = v.select_at(event.position);
                        }
                    }
                    if comment { this.new_draft(window,cx); }
                    cx.notify();
                }),
            )
            .on_mouse_down(MouseButton::Right, cx.listener(|this, event: &MouseDownEvent, window, cx| {
                let Some(v) = &this.viewport else { return; };
                let mut v = v.borrow_mut();
                let Some(point) = v.line_number_hit(event.position, window) else { return; };
                let rules = v.snapshot.remote.as_ref().and_then(|t| this.services.provider(&t.provider));
                match diffz_core::source_link::source_link(&v.snapshot, &point, rules.as_deref()) {
                    Ok(link) => { cx.write_to_clipboard(ClipboardItem::new_string(link)); this.status = format!("Copied source link · {} {}", point.side.api(), point.line); },
                    Err(e) => this.status = e,
                }
                v.select_source_line(point);
                cx.stop_propagation(); cx.notify();
            }))
            .on_mouse_move(cx.listener(|this, event: &MouseMoveEvent, window, cx| {
                if let Some(v) = &this.viewport {
                    let mut v = v.borrow_mut();
                    let hovered = v.hit(event.position);
                    let cursor = if v.comment_hit(event.position).is_some() || v.context_hover(event.position) || v.line_number_hit(event.position, window).is_some() {
                        CursorStyle::PointingHand
                    } else if v.hit_scrollbar(event.position).is_some() || v.hit_horizontal_scrollbar(event.position).is_some() {
                        if event.pressed_button == Some(MouseButton::Left) { CursorStyle::ClosedHand } else { CursorStyle::OpenHand }
                    } else if hovered.is_some() { CursorStyle::IBeam } else { CursorStyle::Arrow };
                    if hovered != v.hovered || cursor != v.hover_cursor {
                        v.hovered = hovered;
                        v.hover_cursor = cursor;
                        cx.notify();
                    }
                }
                if event.pressed_button != Some(MouseButton::Left) {
                    this.drag_start = None;
                    this.scrollbar_drag = false;
                    this.horizontal_drag = false;
                    return;
                }

                if let Some(v) = &this.viewport {
                    let mut v = v.borrow_mut();
                    if this.horizontal_drag {
                        if let Some(f) = v.hit_horizontal_scrollbar(event.position) {
                            v.horizontal = f * v.max_horizontal;
                        }
                    } else if this.scrollbar_drag {
                        if let Some(f) = v.hit_scrollbar(event.position) {
                            v.jump_fraction(f);
                        }
                    } else if let Some(start) = &this.drag_start {
                        if let Some(end) = v.hit(event.position) {
                            if end.side == start.side {
                                v.selection = Some(SourceSelection {
                                    start: start.clone(),
                                    end,
                                });
                            }
                        } else if let Some(frame) = &v.last {
                            let delta = if event.position.y < frame.bounds.top() {
                                -25.0
                            } else if event.position.y > frame.bounds.bottom() {
                                25.0
                            } else {
                                0.0
                            };
                            v.scroll(0.0, delta);
                        }
                    }
                }
                if this.drag_start.is_some() || this.scrollbar_drag || this.horizontal_drag {
                    cx.notify();
                }
            }))
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|this, _, _, cx| {
                    this.drag_start = None;
                    this.scrollbar_drag = false;
                    this.horizontal_drag = false;
                    this.schedule_view_save(cx);
                    cx.notify();
                }),
            )
            .child(
                canvas(
                    move |bounds, window, _| measure.borrow_mut().frame(bounds, window),
                    move |_, frame, window, cx| {
                        window.with_content_mask(
                            Some(ContentMask {
                                bounds: frame.bounds,
                            }),
                            |window| paint.borrow().paint(&frame, skin, window, cx),
                        );
                    },
                )
                .size_full(),
            )
            .into_any_element()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[::core::prelude::v1::test]
    fn files_collapse_at_the_pointer_edge() {
        assert!(should_collapse_files(24., 180., 0.));
        assert!(!should_collapse_files(24.1, 180., 0.));
    }

    #[::core::prelude::v1::test]
    fn files_collapse_at_the_fast_raw_width_boundary() {
        assert!(should_collapse_files(100., 150., -900.));
        assert!(!should_collapse_files(100., 150., -899.9));
        assert!(!should_collapse_files(100., 150.1, -1_000.));
    }

    #[::core::prelude::v1::test]
    fn slow_drag_below_minimum_does_not_collapse_files() {
        assert!(!should_collapse_files(100., 170., -100.));
    }

    #[::core::prelude::v1::test]
    fn resize_velocity_expires_after_100_milliseconds() {
        assert_eq!(
            effective_resize_velocity(-1_000., std::time::Duration::from_millis(100)),
            -1_000.
        );
        assert_eq!(
            effective_resize_velocity(-1_000., std::time::Duration::from_millis(101)),
            0.
        );
    }

    #[::core::prelude::v1::test]
    fn unpressed_mouse_move_cancels_active_resize() {
        let now = std::time::Instant::now();
        let mut resizing = Some(PanelResizeState {
            left: true,
            start_x: 300.,
            start_width: 282.,
            last_x: 300.,
            last_sample: now,
            velocity_x: 0.,
        });

        assert!(cancel_resize_on_unpressed_mouse(&mut resizing, None));
        assert!(resizing.is_none());
    }

    #[::core::prelude::v1::test]
    fn peek_opens_on_button_entry_only_while_the_panel_is_collapsed() {
        let mut peek = FilesPeek::default();
        assert!(!peek.hover_button(true, true));
        assert!(!peek.open);

        peek.reset();
        assert!(!peek.hover_button(true, false));
        assert!(peek.open);
    }

    #[::core::prelude::v1::test]
    fn peek_survives_the_move_from_the_button_to_the_panel() {
        let mut peek = FilesPeek::default();
        peek.hover_button(true, false);
        assert!(!peek.hover_panel(true));
        assert!(!peek.hover_button(false, false));
        assert!(!peek.expire());
        assert!(peek.open);
    }

    #[::core::prelude::v1::test]
    fn leaving_both_regions_closes_the_peek_when_the_delay_expires() {
        let mut peek = FilesPeek::default();
        peek.hover_button(true, false);
        assert!(peek.hover_button(false, false));
        assert!(peek.expire());
        assert!(!peek.open);
        assert!(!peek.expire());
    }

    #[::core::prelude::v1::test]
    fn hovering_again_before_the_delay_expires_keeps_the_peek_open() {
        let mut peek = FilesPeek::default();
        peek.hover_button(true, false);
        assert!(peek.hover_button(false, false));
        assert!(!peek.hover_panel(true));
        assert!(!peek.expire());
        assert!(peek.open);
    }

    #[::core::prelude::v1::test]
    fn the_peek_enters_over_the_in_duration_and_leaves_over_the_out_duration() {
        let mut peek = FilesPeek::default();
        peek.hover_button(true, false);
        assert!(peek.advance(FILES_PEEK_IN_MS / 2.));
        assert!(!peek.advance(FILES_PEEK_IN_MS / 2.));
        assert_eq!(peek.visibility, 1.);

        peek.hover_button(false, false);
        peek.expire();
        assert!(peek.advance(FILES_PEEK_OUT_MS / 2.));
        assert!(!peek.advance(FILES_PEEK_OUT_MS / 2.));
        assert_eq!(peek.visibility, 0.);
    }

    #[::core::prelude::v1::test]
    fn the_peek_frame_spans_the_slide_and_the_fade() {
        let mut peek = FilesPeek::default();
        assert_eq!(peek.frame(), (-FILES_PEEK_SLIDE_PX, 0.));

        peek.hover_button(true, false);
        peek.advance(FILES_PEEK_IN_MS);
        assert_eq!(peek.frame(), (0., 1.));
    }

    #[::core::prelude::v1::test]
    fn reversing_mid_flight_continues_from_where_the_peek_stands() {
        let mut peek = FilesPeek::default();
        peek.hover_button(true, false);
        peek.advance(FILES_PEEK_IN_MS / 2.);
        let caught = peek.visibility;

        peek.hover_button(false, false);
        peek.expire();
        assert_eq!(peek.visibility, caught);
        peek.advance(FILES_PEEK_OUT_MS / 4.);
        assert_eq!(peek.visibility, caught - 0.25);
    }

    #[::core::prelude::v1::test]
    fn catching_the_closing_peek_reopens_it_from_its_current_progress() {
        let mut peek = FilesPeek::default();
        peek.hover_button(true, false);
        peek.advance(FILES_PEEK_IN_MS);
        peek.hover_button(false, false);
        peek.expire();
        peek.advance(FILES_PEEK_OUT_MS / 2.);
        let caught = peek.visibility;

        assert!(!peek.hover_panel(true));
        assert!(peek.open);
        assert_eq!(peek.visibility, caught);
    }

    #[::core::prelude::v1::test]
    fn the_peek_stays_on_screen_until_the_exit_finishes() {
        let mut peek = FilesPeek::default();
        peek.hover_button(true, false);
        peek.advance(FILES_PEEK_IN_MS);
        peek.hover_button(false, false);
        peek.expire();
        peek.advance(FILES_PEEK_OUT_MS / 2.);
        assert!(peek.shown());

        peek.advance(FILES_PEEK_OUT_MS / 2.);
        assert!(!peek.shown());
    }

    #[::core::prelude::v1::test]
    fn pinning_or_collapsing_the_panel_snaps_the_peek_away() {
        let mut peek = FilesPeek::default();
        peek.hover_button(true, false);
        peek.advance(FILES_PEEK_IN_MS / 2.);
        peek.reset();
        assert!(!peek.shown());
        assert!(!peek.moving());
        assert_eq!(peek.visibility, 0.);
    }

    #[::core::prelude::v1::test]
    fn a_reset_under_the_pointer_waits_for_a_fresh_button_entry() {
        let mut peek = FilesPeek::default();
        peek.hover_button(true, false);
        peek.reset();
        assert!(!peek.open);
        assert!(!peek.hover_panel(false));
        assert!(!peek.open);

        assert!(!peek.hover_button(true, false));
        assert!(peek.open);
    }

    #[::core::prelude::v1::test]
    fn resting_on_the_left_edge_opens_the_peek_unless_the_panel_is_pinned() {
        let mut peek = FilesPeek::default();
        let (dwell, close) = peek.hover_edge(true);
        assert!(dwell);
        assert!(!close);
        assert!(!peek.open);
        assert!(peek.dwell_elapsed(false));
        assert!(peek.open);

        let mut pinned = FilesPeek::default();
        pinned.hover_edge(true);
        assert!(!pinned.dwell_elapsed(true));
        assert!(!pinned.open);
    }

    #[::core::prelude::v1::test]
    fn a_pointer_crossing_the_edge_leaves_before_the_dwell_is_up() {
        let mut peek = FilesPeek::default();
        peek.hover_edge(true);
        let (dwell, close) = peek.hover_edge(false);
        assert!(!dwell);
        assert!(!close);
        assert!(!peek.dwell_elapsed(false));
        assert!(!peek.open);
    }

    #[::core::prelude::v1::test]
    fn the_edge_holds_an_open_peek_the_way_the_panel_does() {
        let mut peek = FilesPeek::default();
        peek.hover_button(true, false);
        peek.hover_edge(true);
        assert!(!peek.hover_button(false, false));
        assert!(!peek.expire());
        assert!(peek.open);

        let (_, close) = peek.hover_edge(false);
        assert!(close);
        assert!(peek.expire());
    }

    #[::core::prelude::v1::test]
    fn the_edge_under_a_just_collapsed_panel_does_not_hand_it_back() {
        let mut peek = FilesPeek::default();
        peek.reset();
        // A pointer resting where the gesture left it keeps the block.
        peek.clear_edge_block(FILES_PEEK_EDGE_PX);
        let (dwell, _) = peek.hover_edge(true);
        assert!(!dwell);
        assert!(!peek.dwell_elapsed(false));
        assert!(!peek.open);
    }

    #[::core::prelude::v1::test]
    fn the_edge_arms_again_once_the_pointer_is_seen_away_from_it() {
        let mut peek = FilesPeek::default();
        peek.reset();
        peek.clear_edge_block(FILES_PEEK_EDGE_PX + 1.);

        let (dwell, _) = peek.hover_edge(true);
        assert!(dwell);
        assert!(peek.dwell_elapsed(false));
        assert!(peek.open);
    }

    #[::core::prelude::v1::test]
    fn leaving_the_edge_clears_the_block_for_the_next_visit() {
        let mut peek = FilesPeek::default();
        peek.reset();
        peek.hover_edge(true);
        let (dwell, _) = peek.hover_edge(false);
        assert!(!dwell);

        let (dwell, _) = peek.hover_edge(true);
        assert!(dwell);
        assert!(peek.dwell_elapsed(false));
        assert!(peek.open);
    }

    #[::core::prelude::v1::test]
    fn a_pointer_that_leaves_the_window_lets_the_open_peek_go() {
        let mut peek = FilesPeek::default();
        peek.hover_button(true, false);
        peek.hover_panel(true);

        assert!(peek.pointer_left());
        assert!(peek.open);
        assert!(peek.expire());
        assert!(!peek.open);
    }

    #[::core::prelude::v1::test]
    fn a_pointer_that_leaves_the_window_never_completes_a_dwell() {
        let mut peek = FilesPeek::default();
        peek.hover_edge(true);

        assert!(!peek.pointer_left());
        assert!(!peek.dwell_elapsed(false));
        assert!(!peek.open);
    }

    #[::core::prelude::v1::test]
    fn leaving_the_window_keeps_the_block_a_collapse_left_behind() {
        let mut peek = FilesPeek::default();
        peek.reset();
        peek.hover_edge(true);
        peek.pointer_left();

        let (dwell, _) = peek.hover_edge(true);
        assert!(!dwell);
        assert!(!peek.dwell_elapsed(false));
    }

    #[::core::prelude::v1::test]
    fn returns_only_existing_forward_and_backward_neighbors() {
        for (index, len, direction, expected) in [
            (0, 3, true, Some(1)),
            (1, 3, true, Some(2)),
            (2, 3, true, None),
            (2, 3, false, Some(1)),
            (1, 3, false, Some(0)),
            (0, 3, false, None),
        ] {
            assert_eq!(adjacent_file_index(index, len, direction), expected);
        }
    }

    #[::core::prelude::v1::test]
    fn rejects_empty_and_out_of_range_file_lists() {
        for (index, len, direction) in [
            (0, 0, true),
            (0, 0, false),
            (3, 3, true),
            (3, 3, false),
            (usize::MAX, 3, true),
            (usize::MAX, 3, false),
        ] {
            assert_eq!(adjacent_file_index(index, len, direction), None);
        }
    }
}
