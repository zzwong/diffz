use crate::{
    app::{Panel, PanelResizeState, Workbench},
    commands::{self, Command},
    viewport::ContextRequest,
};
use diffz_core::{domain::*, provider::OpenRequest};
use gpui_kit::component::{StyledExt, button::*, input::Input};
use gpui_kit::{prelude::*, *};

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

fn ease_out_cubic(t: f32) -> f32 {
    let remaining = 1. - t;
    1. - remaining * remaining * remaining
}

/// The floating file panel shown while the collapsed panel's toggle, or the panel
/// itself, is hovered. `open` is the target; `visibility` is where the motion is.
#[derive(Default)]
pub struct FilesPeek {
    pub open: bool,
    visibility: f32,
    button_hovered: bool,
    panel_hovered: bool,
}

impl FilesPeek {
    /// Arms on the hover-enter event alone: a click that collapses the pinned panel
    /// leaves the pointer on the button, and reading "is hovered" would reopen it.
    pub(crate) fn hover_button(&mut self, hovered: bool, pinned: bool) -> bool {
        let entered = hovered && !self.button_hovered;
        self.button_hovered = hovered;
        if entered && !pinned {
            self.open = true;
        }
        self.closing()
    }
    pub(crate) fn hover_panel(&mut self, hovered: bool) -> bool {
        self.panel_hovered = hovered;
        // A pointer that catches the fading panel brings it back from where it is.
        if hovered && self.visibility > 0. {
            self.open = true;
        }
        self.closing()
    }
    fn closing(&self) -> bool {
        self.open && !self.button_hovered && !self.panel_hovered
    }
    fn expire(&mut self) -> bool {
        let close = self.closing();
        if close {
            self.open = false;
        }
        close
    }
    fn reset(&mut self) {
        *self = Self::default();
    }
    fn shown(&self) -> bool {
        self.open || self.visibility > 0.
    }
    fn target(&self) -> f32 {
        if self.open { 1. } else { 0. }
    }
    fn moving(&self) -> bool {
        self.visibility != self.target()
    }
    /// Steps the progress towards the target; true while it has further to go.
    fn advance(&mut self, dt_ms: f32) -> bool {
        let target = self.target();
        let step = dt_ms
            / if self.open {
                FILES_PEEK_IN_MS
            } else {
                FILES_PEEK_OUT_MS
            };
        self.visibility = if self.visibility < target {
            (self.visibility + step).min(target)
        } else {
            (self.visibility - step).max(target)
        };
        self.moving()
    }
    /// Left offset in pixels and opacity. Both directions read the same progress
    /// through the same curve, so a reversal mid-flight stays continuous.
    fn frame(&self) -> (f32, f32) {
        let eased = ease_out_cubic(self.visibility);
        (-FILES_PEEK_SLIDE_PX * (1. - eased), eased)
    }
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
            .visible_files
            .iter()
            .position(|id| Some(id) == current.as_ref())
        else {
            return;
        };
        let next = if direction > 0 {
            at.checked_add(1).filter(|i| *i < self.visible_files.len())
        } else {
            at.checked_sub(1)
        };
        let Some(next) = next else {
            self.status = "End of the changed-file list".into();
            return;
        };
        self.select_file(self.visible_files[next].clone(), cx);
        if let Some(v) = &self.viewport {
            v.borrow_mut().jump_edge(direction < 0);
        }
        self.diff_focus.focus(window, cx);
        self.status = format!("File {} of {}", next + 1, self.visible_files.len());
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
            self.status = "Context expansion needs a saved GitHub or GitLab review; standalone patches have no omitted source".into();
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

    fn resize_panel(&mut self, left: bool, desired: f32, window: &Window) {
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
    /// Called from both peek hover handlers with the "a close is due" answer.
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
    pub(crate) fn reset_files_peek(&mut self) {
        self.files_peek.reset();
        self.files_peek_close = None;
        self.files_peek_frame = None;
    }
    fn panel_divider(&self, left: bool, cx: &mut Context<Self>) -> AnyElement {
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
    pub fn command(&mut self, command: Command, window: &mut Window, cx: &mut Context<Self>) {
        if matches!(
            command,
            Command::Open | Command::Palette | Command::Preview | Command::Export
        ) {
            self.return_focus = window.focused(cx);
        }
        match command {
            Command::Recent => self.show_recents(window, cx),
            Command::Themes => self.show_themes(window, cx),
            Command::Open => {
                self.panel = Panel::Open;
                self.open_input.read(cx).focus_handle(cx).focus(window, cx);
            }
            Command::Palette => {
                self.panel = Panel::Palette;
                self.palette_index = 0;
                self.palette_scroll.scroll_to_item(0);
                self.palette_input
                    .read(cx)
                    .focus_handle(cx)
                    .focus(window, cx);
            }
            Command::Find => {
                self.find_visible = true;
                self.find_input.read(cx).focus_handle(cx).focus(window, cx);
            }
            Command::Files => {
                self.files_visible = !self.files_visible;
                self.reset_files_peek();
                if !self.files_visible {
                    self.diff_focus.focus(window, cx);
                }
                self.schedule_view_save(cx);
            }
            Command::Inspector => {
                self.inspector_visible = !self.inspector_visible;
                if self.inspector_visible {
                    self.overview_focus.focus(window, cx);
                }
                if !self.inspector_visible {
                    self.diff_focus.focus(window, cx);
                }
                self.schedule_view_save(cx);
            }
            Command::Wrap | Command::Split | Command::ZoomIn | Command::ZoomOut => {
                if let Some(view) = &self.viewport {
                    let mut v = view.borrow_mut();
                    let mut split = v.split;
                    let mut wrap = v.wrap;
                    let mut font = v.font_size;
                    match command {
                        Command::Wrap => wrap = !wrap,
                        Command::Split => split = !split,
                        Command::ZoomIn => font += 1.0,
                        Command::ZoomOut => font -= 1.0,
                        _ => {}
                    }
                    v.configure(split, wrap, font);
                    self.settings.split = split;
                    self.settings.font_size = v.font_size;
                    if matches!(command, Command::Wrap) {
                        self.settings.wrap = Some(wrap);
                    }
                }
                self.save_settings(cx);
            }
            Command::Rich => {
                self.settings.rich = !self.settings.rich;
                self.save_settings(cx);
            }
            Command::RichInline => {
                self.settings.rich_inline = !self.settings.rich_inline;
                self.save_settings(cx);
            }
            Command::Theme => {
                if self.palette.is_some() {
                    // Clearing the selected theme keeps the palette's light or dark choice.
                    self.apply_theme(None, window, cx);
                    self.status = "Theme removed; built-in colours are back".into();
                } else {
                    self.dark = !self.dark;
                    self.settings.dark = self.dark;
                    crate::app::apply_appearance(self.dark, None, Some(window), cx);
                    self.save_settings(cx);
                }
            }
            Command::Copy => {
                if let (Some(a), Some(v)) = (&self.active, &self.viewport)
                    && let Some(s) = &v.borrow().selection
                {
                    match a.snapshot.copy_selection(s) {
                        Ok(text) => {
                            cx.write_to_clipboard(ClipboardItem::new_string(text));
                            self.status =
                                "Copied the original source bytes without wrap breaks or gutter text."
                                    .into();
                        }
                        Err(e) => self.status = e,
                    }
                }
            }
            Command::Comment => self.new_draft(window, cx),
            Command::CommentFile => self.new_draft_file(window, cx),
            Command::Preview => {
                self.panel = Panel::Preview;
                self.summary_input
                    .read(cx)
                    .focus_handle(cx)
                    .focus(window, cx);
                let summary = self
                    .active
                    .as_ref()
                    .map_or_else(String::new, |a| a.view.review_summary.clone());
                self.summary_input
                    .update(cx, |s, cx| s.set_value(summary, window, cx));
            }
            Command::Refresh => {
                // A refresh of a review resumed from Recents must query the provider,
                // rather than reuse the local store.
                let remote = self
                    .active
                    .as_ref()
                    .and_then(|a| a.snapshot.remote.as_ref())
                    .map(|r| r.open_request());
                let request = match (self.last_request.clone(), remote) {
                    (Some(OpenRequest::Resume(_)) | None, Some(remote)) => Some(remote),
                    (Some(request), _) => Some(request),
                    (None, None) => None,
                };
                if let Some(request) = request {
                    self.open(request, true, cx)
                } else {
                    self.status = "Nothing can refresh until a source is open.".into();
                }
            }
            Command::NextHunk | Command::PreviousHunk => {
                let forward = command == Command::NextHunk;
                let target = self
                    .viewport
                    .as_ref()
                    .and_then(|v| v.borrow_mut().next_hunk(forward));
                self.status = if let Some((index, total)) = target {
                    format!("Hunk {index} of {total}")
                } else if forward {
                    "This file has no later hunk. Press ] to move to the next file.".into()
                } else {
                    "This file has no earlier hunk. Press [ to move to the previous file.".into()
                };
                self.diff_focus.focus(window, cx);
            }
            Command::NextFile | Command::PreviousFile => {
                let current = self.viewport.as_ref().map(|v| v.borrow().file.clone());
                if let Some(at) = self
                    .visible_files
                    .iter()
                    .position(|f| Some(f) == current.as_ref())
                    && let Some(next) = adjacent_file_index(
                        at,
                        self.visible_files.len(),
                        command == Command::NextFile,
                    )
                {
                    self.select_file(self.visible_files[next].clone(), cx);
                    if let Some(v) = &self.viewport {
                        v.borrow_mut().jump_first_hunk();
                    }
                }
            }
            Command::Export => self.begin_export(cx),
            Command::Cancel => {
                if self.panel != Panel::None {
                    self.panel = Panel::None;
                    self.line_context = None;
                    self.selected_draft = None;
                    self.thread_root = None;
                } else if self.inspector_visible
                    && (self.overview_hovered || self.overview_focus.contains_focused(window, cx))
                {
                    self.inspector_visible = false;
                    self.overview_hovered = false;
                } else if self.find_visible {
                    self.find_visible = false;
                    if let Some(v) = &self.viewport {
                        v.borrow_mut().active_search = None;
                    }
                } else if let Some(v) = &self.viewport {
                    v.borrow_mut().selection = None;
                }
                self.return_focus
                    .take()
                    .unwrap_or_else(|| self.diff_focus.clone())
                    .focus(window, cx);
            }
        }
        cx.notify();
    }
    pub fn navigate_hit(&mut self, forward: bool, cx: &mut Context<Self>) {
        if self.search_hits.is_empty() {
            self.status = "The loaded patch context contains no matches.".into();
            cx.notify();
            return;
        }
        if self.search_index >= self.search_hits.len() {
            self.search_index = if forward {
                0
            } else {
                self.search_hits.len() - 1
            };
        } else if forward {
            self.search_index = (self.search_index + 1) % self.search_hits.len();
        } else {
            self.search_index =
                (self.search_index + self.search_hits.len() - 1) % self.search_hits.len();
        }
        let h = self.search_hits[self.search_index].clone();
        self.select_file(h.file.clone(), cx);
        if let (Some(a), Some(v)) = (&self.active, &self.viewport) {
            let p = SourcePoint {
                snapshot: a.snapshot.id.clone(),
                file: h.file,
                side: h.side,
                line: h.line,
                byte_column: h.bytes.start,
            };
            let q = SourcePoint {
                byte_column: h.bytes.end,
                ..p.clone()
            };
            let mut v = v.borrow_mut();
            v.reveal_range(p.clone(), h.bytes.end);
            v.selection = Some(SourceSelection { start: p, end: q });
            v.active_search = v.selection.clone();
        }
        self.schedule_view_save(cx);
        cx.notify();
    }
    pub fn accept_offer(&mut self, cx: &mut Context<Self>) {
        if self.unsaved() {
            self.status = "Save your drafts before changing revisions.".into();
            cx.notify();
            return;
        }
        if let Some(opened) = self.offered.take() {
            self.install(opened, cx);
            self.status="The new revision is active. Older drafts stay in their session; comments were not remapped.".into();
        }
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
        v.borrow_mut().pull = (
            self.boundary_scroll.pulling(),
            self.boundary_scroll.progress(),
        );
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
                match diffz_core::source_link::source_link(&v.snapshot, &point) {
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
                                bounds: frame.full_bounds,
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

impl Render for Workbench {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        diffz_core::timing::mark_once("first render");
        let skin = self.skin();
        let titlebar = self.topbar(window, cx);
        let toolbar = self.toolbar(cx);
        let mut source = div().v_flex().flex_1().min_w_0().min_h_0().child(toolbar);
        if self.find_visible {
            source = source.child(
                div()
                    .h_flex()
                    .px_3()
                    .py_2()
                    .gap_2()
                    .child(Input::new(&self.find_input))
                    .child(if self.search_index < self.search_hits.len() {
                        format!("{} of {}", self.search_index + 1, self.search_hits.len())
                    } else {
                        format!("{} matches", self.search_hits.len())
                    })
                    .child(
                        Button::new("prev-match")
                            .cursor_pointer()
                            .label("Previous")
                            .on_click(cx.listener(|a, _, _, c| a.navigate_hit(false, c))),
                    )
                    .child(
                        div()
                            .track_focus(&self.find_next_focus)
                            .rounded_md()
                            .border_1()
                            .border_color(if self.find_next_focus.contains_focused(window, cx) {
                                skin.accent
                            } else {
                                skin.base.opacity(0.)
                            })
                            .child(
                                Button::new("next-match")
                                    .cursor_pointer()
                                    .label("Next")
                                    .on_click(cx.listener(|a, _, _, c| a.navigate_hit(true, c))),
                            ),
                    ),
            );
        }
        if self.offered.is_some() {
            source = source.child(
                div()
                    .h_flex()
                    .gap_3()
                    .p_2()
                    .bg(skin.raised)
                    .child("A newer revision exists. Your reviewed snapshot is still on screen.")
                    .child(
                        Button::new("accept-revision")
                            .cursor_pointer()
                            .label("Open new revision")
                            .on_click(cx.listener(|a, _, _, c| a.accept_offer(c))),
                    ),
            );
        }
        if let Some(a) = &self.active {
            for warning in a.snapshot.warnings.iter().take(4) {
                source = source.child(div().p_2().text_color(skin.warning).child(warning.clone()));
            }
        }
        source = source.child(if self.rich_active() {
            self.rich_view(cx)
        } else {
            self.reader(cx)
        });
        let overlay_inspector =
            self.inspector_visible && f32::from(window.viewport_size().width) < 1100.0;
        let mut body = div()
            .h_flex()
            .items_stretch()
            .flex_1()
            .min_h_0()
            .min_w_0()
            .relative();
        if self.files_visible {
            body = body.child(self.files(cx));
        }
        body = body.child(source);
        if self.inspector_visible && !overlay_inspector {
            body = body.child(self.inspector(cx));
        }
        if self.files_visible {
            body = body.child(self.panel_divider(true, cx));
        }
        if self.inspector_visible && !overlay_inspector {
            body = body.child(self.panel_divider(false, cx));
        }
        if overlay_inspector {
            body = body.child(
                div()
                    .absolute()
                    .top_0()
                    .bottom_0()
                    .right_0()
                    .w(px(self.overview_width))
                    .h_flex()
                    .child(self.inspector(cx))
                    .child(self.panel_divider(false, cx)),
            );
        }
        if !self.files_visible && self.files_peek.shown() {
            // The hover region keeps the settled bounds; only the panel inside moves.
            let (offset, opacity) = self.files_peek.frame();
            body = body.child(
                div()
                    .id("files-peek")
                    .absolute()
                    .top_0()
                    .bottom_0()
                    .left_0()
                    .w(px(self.files_width))
                    .occlude()
                    .on_hover(cx.listener(|a, hovered: &bool, window, cx| {
                        let close = a.files_peek.hover_panel(*hovered);
                        a.files_peek_hovered(close, window, cx);
                    }))
                    .child(
                        div()
                            .absolute()
                            .top_0()
                            .bottom_0()
                            .left(px(offset))
                            .w(px(self.files_width))
                            .opacity(opacity)
                            .shadow_lg()
                            .child(self.files(cx)),
                    ),
            );
        }
        if diffz_core::timing::enabled() && self.active.is_some() {
            body = body.child(
                canvas(
                    |_, _, _| (),
                    |_, _, _, _| {
                        static FIRST_PAINT: std::sync::Once = std::sync::Once::new();
                        FIRST_PAINT.call_once(|| diffz_core::timing::mark("first content paint"));
                    },
                )
                .absolute()
                .size(px(1.)),
            );
        }
        let footer = self.footer(window, cx);
        let mut root = div()
            .id("workbench")
            .on_mouse_move(cx.listener(|a, e: &MouseMoveEvent, window, cx| {
                if cancel_resize_on_unpressed_mouse(&mut a.resizing_panel, e.pressed_button) {
                    cx.notify();
                    return;
                }
                if let Some(state) = a.resizing_panel.as_mut() {
                    let x = f32::from(e.position.x);
                    let now = std::time::Instant::now();
                    let elapsed = now.saturating_duration_since(state.last_sample);
                    if elapsed.is_zero() {
                        state.velocity_x = 0.;
                    } else {
                        state.velocity_x = (x - state.last_x) / elapsed.as_secs_f32();
                    }
                    state.last_x = x;
                    state.last_sample = now;
                    let left = state.left;
                    let desired = state.start_width
                        + if left {
                            x - state.start_x
                        } else {
                            state.start_x - x
                        };
                    a.resize_panel(left, desired, window);
                    cx.notify();
                }
            }))
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|a, e: &MouseUpEvent, window, cx| {
                    let Some(state) = a.resizing_panel.take() else {
                        return;
                    };
                    let x = f32::from(e.position.x);
                    let desired = state.start_width
                        + if state.left {
                            x - state.start_x
                        } else {
                            state.start_x - x
                        };
                    let velocity_x = effective_resize_velocity(
                        state.velocity_x,
                        std::time::Instant::now().saturating_duration_since(state.last_sample),
                    );
                    a.resize_panel(state.left, desired, window);
                    if state.left && should_collapse_files(x, desired, velocity_x) {
                        a.files_width = state.start_width;
                        a.files_visible = false;
                        a.reset_files_peek();
                        a.diff_focus.focus(window, cx);
                        a.schedule_view_save(cx);
                    }
                    cx.notify();
                }),
            )
            .track_focus(&self.root_focus)
            .key_context("Workbench")
            .relative()
            .v_flex()
            .size_full()
            .bg(skin.base)
            .text_color(skin.text)
            .font_family(crate::theme::ui_font())
            .text_size(px(13.))
            .capture_action(
                cx.listener(|a, action: &gpui_kit::component::input::Enter, w, c| {
                    if a.panel == Panel::Line && action.secondary {
                        a.command(Command::Cancel, w, c);
                        c.stop_propagation();
                    } else {
                        c.propagate();
                    }
                }),
            )
            .capture_action(
                cx.listener(|a, _: &gpui_kit::component::input::Escape, w, c| {
                    if a.panel != Panel::None {
                        a.command(Command::Cancel, w, c);
                        c.stop_propagation();
                    } else {
                        c.propagate();
                    }
                }),
            )
            .capture_key_down(
                cx.listener(|a, event: &KeyDownEvent, w, c| a.handle_keys(event, w, c)),
            )
            .on_action(
                cx.listener(|a, _: &commands::Recent, w, c| a.command(Command::Recent, w, c)),
            )
            .on_action(
                cx.listener(|a, _: &commands::Themes, w, c| a.command(Command::Themes, w, c)),
            )
            .on_action(cx.listener(|a, _: &commands::Open, w, c| a.command(Command::Open, w, c)))
            .on_action(cx.listener(|a, _: &commands::Find, w, c| a.command(Command::Find, w, c)))
            .on_action(cx.listener(|a, _: &commands::Files, w, c| a.command(Command::Files, w, c)))
            .on_action(
                cx.listener(|a, _: &commands::Inspector, w, c| a.command(Command::Inspector, w, c)),
            )
            .on_action(
                cx.listener(|a, _: &commands::Palette, w, c| a.command(Command::Palette, w, c)),
            )
            .on_action(cx.listener(|a, _: &commands::Wrap, w, c| a.command(Command::Wrap, w, c)))
            .on_action(cx.listener(|a, _: &commands::Split, w, c| a.command(Command::Split, w, c)))
            .on_action(cx.listener(|a, _: &commands::Rich, w, c| a.command(Command::Rich, w, c)))
            .on_action(
                cx.listener(|a, _: &commands::RichInline, w, c| {
                    a.command(Command::RichInline, w, c)
                }),
            )
            .on_action(
                cx.listener(|a, _: &commands::CopySource, w, c| a.command(Command::Copy, w, c)),
            )
            .on_action(
                cx.listener(|a, _: &commands::Comment, w, c| a.command(Command::Comment, w, c)),
            )
            .on_action(cx.listener(|a, _: &commands::CommentFile, w, c| {
                a.command(Command::CommentFile, w, c)
            }))
            .on_action(
                cx.listener(|a, _: &commands::NextHunk, w, c| a.command(Command::NextHunk, w, c)),
            )
            .on_action(cx.listener(|a, _: &commands::PreviousHunk, w, c| {
                a.command(Command::PreviousHunk, w, c)
            }))
            .on_action(
                cx.listener(|a, _: &commands::NextFile, w, c| a.command(Command::NextFile, w, c)),
            )
            .on_action(cx.listener(|a, _: &commands::PreviousFile, w, c| {
                a.command(Command::PreviousFile, w, c)
            }))
            .on_action(
                cx.listener(|a, _: &commands::Preview, w, c| a.command(Command::Preview, w, c)),
            )
            .on_action(
                cx.listener(|a, _: &commands::Refresh, w, c| a.command(Command::Refresh, w, c)),
            )
            .on_action(
                cx.listener(|a, _: &commands::ZoomIn, w, c| a.command(Command::ZoomIn, w, c)),
            )
            .on_action(
                cx.listener(|a, _: &commands::ZoomOut, w, c| a.command(Command::ZoomOut, w, c)),
            )
            .on_action(
                cx.listener(|a, _: &commands::Cancel, w, c| a.command(Command::Cancel, w, c)),
            )
            .child(titlebar)
            .child(body)
            .child(footer);
        if self.panel == Panel::Line {
            root = root.child(self.line_panel(window, cx));
        } else if self.panel != Panel::None {
            root = root.child(self.panel_view(cx));
        }
        root
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
        // Nothing reaches the peek while the pointer rests on the button after a click.
        assert!(!peek.hover_panel(false));
        assert!(!peek.open);

        assert!(!peek.hover_button(true, false));
        assert!(peek.open);
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
