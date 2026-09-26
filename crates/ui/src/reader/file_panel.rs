use super::*;

impl Workbench {
    pub(super) fn resize_panel(&mut self, left: bool, desired: f32, window: &Window) {
        let width = f32::from(window.viewport_size().width);
        let other = if left && self.inspector_visible && width >= 1100. {
            self.overview_width + 6.
        } else if !left && self.files_visible && width >= 1100. {
            self.files_width + 6.
        } else {
            0.
        };
        let minimum = if left { 180. } else { 280. };
        let limit = (width - other - 326.).min(width * 0.45).max(minimum);
        if left {
            self.files_width = desired.clamp(minimum, limit);
        } else {
            self.overview_width = desired.clamp(minimum, limit);
        }
    }
    /// The pointer entered or left the left-edge zone.
    pub(super) fn files_peek_edge(
        &mut self,
        hovered: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let (dwell, close) = self.files_peek.hover_edge(hovered);
        self.files_peek_dwell = dwell.then(|| {
            cx.spawn_in(window, async move |this, cx| {
                smol::Timer::after(std::time::Duration::from_millis(FILES_PEEK_DWELL_MS)).await;
                let _ = this.update_in(cx, |this, window, cx| this.files_peek_dwelled(window, cx));
            })
        });
        self.files_peek_hovered(close, window, cx);
    }
    pub(super) fn files_peek_left(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.files_peek_dwell = None;
        let close = self.files_peek.pointer_left();
        self.files_peek_hovered(close, window, cx);
    }
    /// A pointer that left the window and came back gets no fresh hover from gpui: the
    /// exit leaves its last position behind, so the region still counts as hovered
    /// there. A move inside the region stands in for the entry that never arrives.
    pub(super) fn files_peek_edge_moved(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.files_peek.edge_hovered {
            self.files_peek_edge(true, window, cx);
        }
    }
    pub(super) fn files_peek_panel_moved(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.files_peek.panel_hovered {
            let close = self.files_peek.hover_panel(true);
            self.files_peek_hovered(close, window, cx);
        }
    }
    fn files_peek_dwelled(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.files_peek_dwell = None;
        // A pointer holding a selection, a scrollbar, or a panel edge is on its way
        // somewhere else and only passes the zone.
        let gesturing = self.drag_start.is_some()
            || self.resizing_panel.is_some()
            || self.scrollbar_drag
            || self.horizontal_drag;
        if gesturing || !self.files_peek.dwell_elapsed(self.files_visible) {
            return;
        }
        self.animate_files_peek(window, cx);
        cx.notify();
    }
    /// Called from the peek hover handlers with the "a close is due" answer.
    pub(crate) fn files_peek_hovered(
        &mut self,
        close: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.files_peek_close = close.then(|| {
            cx.spawn_in(window, async move |this, cx| {
                smol::Timer::after(std::time::Duration::from_millis(FILES_PEEK_GRACE_MS)).await;
                let _ = this.update_in(cx, |this, window, cx| {
                    if !this.files_peek.expire() {
                        return;
                    }
                    let filter = this.filter_input.read(cx).focus_handle(cx);
                    if this.tree_focus.contains_focused(window, cx)
                        || filter.contains_focused(window, cx)
                    {
                        this.diff_focus.focus(window, cx);
                    }
                    this.animate_files_peek(window, cx);
                    cx.notify();
                });
            })
        });
        self.animate_files_peek(window, cx);
        cx.notify();
    }
    /// Advance the peek once per frame until it settles on its target.
    fn animate_files_peek(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.files_peek_frame.is_some() || !self.files_peek.moving() {
            return;
        }
        self.files_peek_frame = Some(std::time::Instant::now());
        cx.on_next_frame(window, |this, window, cx| this.files_peek_tick(window, cx));
    }
    fn files_peek_tick(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let now = std::time::Instant::now();
        let Some(previous) = self.files_peek_frame.replace(now) else {
            return;
        };
        let dt = (now.saturating_duration_since(previous).as_secs_f32() * 1_000.)
            .min(FILES_PEEK_STEP_MS);
        if self.files_peek.advance(dt) {
            cx.on_next_frame(window, |this, window, cx| this.files_peek_tick(window, cx));
        } else {
            self.files_peek_frame = None;
        }
        cx.notify();
    }
    pub(super) fn reset_files_peek(&mut self) {
        self.files_peek.reset();
        self.files_peek_close = None;
        self.files_peek_dwell = None;
        self.files_peek_frame = None;
    }
    pub(super) fn panel_divider(&self, left: bool, cx: &mut Context<Self>) -> AnyElement {
        let skin = self.skin();
        let active = self
            .resizing_panel
            .as_ref()
            .is_some_and(|state| state.left == left);
        div()
            .id(if left {
                "resize-files"
            } else {
                "resize-overview"
            })
            .absolute()
            .top_0()
            .bottom_0()
            .when(left, |s| s.left(px(self.files_width - 3.)))
            .when(!left, |s| s.right(px(self.overview_width - 3.)))
            .w(px(6.))
            .h_full()
            .flex_shrink_0()
            .cursor_col_resize()
            .track_focus(if left {
                &self.files_resize_focus
            } else {
                &self.overview_resize_focus
            })
            .tab_stop(true)
            .aria_label(if left {
                "Change the file panel width using horizontal arrow keys."
            } else {
                "Change the overview panel width using horizontal arrow keys."
            })
            .on_key_down(cx.listener(move |a, e: &KeyDownEvent, w, cx| {
                let delta = match e.keystroke.key.as_str() {
                    "left" => -16.,
                    "right" => 16.,
                    _ => return,
                };
                let width = if left {
                    a.files_width + delta
                } else {
                    a.overview_width - delta
                };
                a.resize_panel(left, width, w);
                cx.stop_propagation();
                cx.notify();
            }))
            .when(active, |s| s.bg(skin.accent))
            .hover(|s| s.bg(skin.accent))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |a, e: &MouseDownEvent, w, cx| {
                    if left {
                        a.files_resize_focus.focus(w, cx);
                    } else {
                        a.overview_resize_focus.focus(w, cx);
                    }
                    let x = f32::from(e.position.x);
                    a.resizing_panel = Some(PanelResizeState {
                        left,
                        start_x: x,
                        start_width: if left {
                            a.files_width
                        } else {
                            a.overview_width
                        },
                        last_x: x,
                        last_sample: std::time::Instant::now(),
                        velocity_x: 0.,
                    });
                    a.drag_start = None;
                    cx.stop_propagation();
                    cx.notify();
                }),
            )
            .into_any_element()
    }
}
