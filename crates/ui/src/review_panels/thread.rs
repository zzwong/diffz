use crate::{
    app::{Panel, Workbench},
    commands::Command,
};
use diffz_core::{
    annotation::Anchor,
    domain::*,
    review_details::{
        ThreadLocation, short_timestamp, thread_location, thread_roots_at, visible_markdown,
    },
};
use gpui_kit::component::{Sizable, StyledExt, button::*, text::TextView};
use gpui_kit::{prelude::*, *};

impl Workbench {
    fn thread_target(&self, root: u64) -> Option<(ThreadComment, ThreadLocation)> {
        let a = self.active.as_ref()?;
        let comment = a
            .snapshot
            .comments
            .iter()
            .find(|c| c.id == root)
            .or_else(|| a.snapshot.comments.iter().find(|c| c.root_id == root))
            .cloned()?;
        let location = thread_location(&a.snapshot, &comment);
        Some((comment, location))
    }
    /// Select and highlight a thread line from the overview without opening the reader.
    pub(crate) fn jump_to_thread(
        &mut self,
        root: u64,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some((comment, location)) = self.thread_target(root) else {
            return;
        };
        match location {
            ThreadLocation::Line(point) => {
                self.focus_point(point, window, cx);
                self.thread_root = None;
                self.status = format!(
                    "{}:{} · {}'s thread · press Enter or click the line to open it",
                    comment.path,
                    comment.line.unwrap_or(0),
                    comment.author
                );
            }
            ThreadLocation::File(point) => {
                self.select_file(point.file, cx);
                self.diff_focus.focus(window, cx);
                self.thread_root = None;
                self.status = format!(
                    "{} · {}'s file thread · open it from the overview",
                    comment.path, comment.author
                );
            }
            ThreadLocation::Outdated => {
                self.status = format!(
                    "{} · {}'s thread is outdated; this revision has no matching location",
                    comment.path, comment.author
                );
            }
        }
        cx.notify();
    }
    fn focus_point(&mut self, point: SourcePoint, window: &mut Window, cx: &mut Context<Self>) {
        self.select_file(point.file.clone(), cx);
        if let Some(v) = &self.viewport {
            let mut v = v.borrow_mut();
            let mut point = point;
            // Unified mode paints one cell and places it on the RIGHT side.
            if !v.split
                && point.side == Side::Left
                && let Some(row) = v
                    .snapshot
                    .file(&point.file)
                    .and_then(|f| f.line(point.side, point.line))
                && row.kind == diffz_core::patch::RowKind::Context
                && let Some(new) = row.new_line
            {
                point.side = Side::Right;
                point.line = new;
            }
            v.reveal(point.clone());
            v.selection = Some(SourceSelection {
                start: point.clone(),
                end: point,
            });
            v.active_search = None;
        }
        self.diff_focus.focus(window, cx);
        self.schedule_view_save(cx);
    }
    pub(crate) fn jump_to_annotation(
        &mut self,
        index: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(n) = self.annotations.get(index).cloned() else {
            return;
        };
        let Some(a) = &self.active else { return };
        let Some(file) = a
            .snapshot
            .patch
            .files
            .iter()
            .find(|f| f.display_path() == n.anchor.path())
            .map(|f| f.id.clone())
        else {
            return;
        };
        match n.anchor {
            Anchor::Lines { side, start, .. } => {
                let point = SourcePoint {
                    snapshot: a.snapshot.id.clone(),
                    file,
                    side,
                    line: start,
                    byte_column: 0,
                };
                self.focus_point(point, window, cx);
            }
            Anchor::File { .. } => self.select_file(file, cx),
        }
        self.status = format!("{} · {}", n.source, n.title);
        cx.notify();
    }
    pub(crate) fn show_thread(&mut self, root: u64, window: &mut Window, cx: &mut Context<Self>) {
        let Some((_, location)) = self.thread_target(root) else {
            return;
        };
        self.line_context = None;
        if let ThreadLocation::Line(point) | ThreadLocation::File(point) = location {
            self.select_file(point.file.clone(), cx);
            if point.line > 0
                && let Some(v) = &self.viewport
            {
                v.borrow_mut().reveal(point.clone());
            }
            self.line_context = Some(SourceSelection {
                start: point.clone(),
                end: point,
            });
        }
        self.selected_draft = None;
        self.thread_root = Some(root);
        self.thread_scroll
            .set_offset(gpui_kit::point(px(0.), px(0.)));
        self.return_focus = window.focused(cx);
        self.panel = Panel::Line;
        self.reset_comment_editor();
        self.composer
            .draft_input
            .update(cx, |input, cx| input.set_value("", window, cx));
        self.panel_focus.focus(window, cx);
        cx.notify();
    }
    pub(crate) fn line_panel(&self, window: &Window, cx: &mut Context<Self>) -> AnyElement {
        let skin = self.skin();
        let Some(active) = &self.active else {
            return div().into_any_element();
        };
        let source = self.line_context.as_ref();
        let file_comment = source
            .is_some_and(|s| s.start.side == Side::Right && s.start.line == 0 && s.end.line == 0);
        let path = source
            .and_then(|s| active.snapshot.file(&s.start.file))
            .map(|f| f.display_path());
        let roots = if let Some(root) = self.thread_root {
            vec![root]
        } else {
            source.map_or_else(Vec::new, |s| thread_roots_at(&active.snapshot, &s.end))
        };
        let comments: Vec<_> = active
            .snapshot
            .comments
            .iter()
            .filter(|c| roots.contains(&c.root_id))
            .collect();
        let notes: Vec<_> = source
            .zip(path.as_deref())
            .map(|(s, path)| {
                self.annotations
                    .iter()
                    .filter(|n| n.anchor.path() == path)
                    .filter(|n| match n.anchor {
                        Anchor::File { .. } => file_comment,
                        Anchor::Lines {
                            side, start, end, ..
                        } => side == s.start.side && start <= s.end.line && s.start.line <= end,
                    })
                    .collect()
            })
            .unwrap_or_default();
        // The release a single changed line came from, as its gutter marks it.
        let release = source
            .filter(|s| s.start.line == s.end.line && !file_comment)
            .and_then(|s| {
                let v = self.viewport.as_ref()?.borrow();
                (v.file == s.start.file)
                    .then(|| v.releases.get(&(s.start.side, s.start.line)).cloned())
                    .flatten()
            });
        let title = source
            .map(|s| {
                if file_comment {
                    format!("{} · file comment", path.as_deref().unwrap_or("Source"))
                } else {
                    format!(
                        "{} · {} {}{}",
                        path.as_deref().unwrap_or("Source"),
                        s.start.side.api(),
                        s.start.line,
                        if s.start.line == s.end.line {
                            String::new()
                        } else {
                            format!("–{}", s.end.line)
                        }
                    )
                }
            })
            .or_else(|| {
                comments
                    .first()
                    .map(|c| format!("{} · comment from an earlier revision", c.path))
            })
            .unwrap_or_else(|| "Line comment".into());
        let selected = self
            .selected_draft
            .as_ref()
            .and_then(|id| active.drafts.iter().find(|d| &d.id == id));
        let reading = !comments.is_empty() && selected.is_none();
        let reader_height = (f32::from(window.viewport_size().height) - 96.).max(240.);
        let mut card = div()
            .id("line-comment-card")
            .occlude()
            .on_mouse_down(MouseButton::Left, |_, _, c| c.stop_propagation())
            .track_focus(&self.panel_focus)
            .tab_group()
            .v_flex()
            .when(reading, |d| d.h(px(reader_height)))
            .on_key_down(cx.listener(move |a, e: &KeyDownEvent, _, c| {
                if !reading {
                    return;
                }
                let mut offset = a.thread_scroll.offset();
                match e.keystroke.key.as_str() {
                    "down" => offset.y -= px(40.),
                    "up" => offset.y += px(40.),
                    "pagedown" | "space" => offset.y -= px(reader_height * 0.8),
                    "pageup" => offset.y += px(reader_height * 0.8),
                    "home" => offset.y = px(0.),
                    "end" => offset.y = px(-1_000_000.),
                    _ => return,
                }
                a.thread_scroll.set_offset(offset);
                c.stop_propagation();
                c.notify();
            }))
            .gap_3()
            .p_4()
            .rounded_lg()
            .bg(skin.surface)
            .border_1()
            .border_color(skin.border)
            .shadow_lg()
            .child(
                div()
                    .h_flex()
                    .gap_2()
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .text_size(px(12.))
                            .text_color(skin.muted)
                            .whitespace_normal()
                            .font_family(crate::theme::code_font())
                            .child(title),
                    )
                    .child(
                        Button::new("close-line")
                            .ghost()
                            .small()
                            .cursor_pointer()
                            .label("Esc")
                            .on_click(cx.listener(|a, _, w, c| a.command(Command::Cancel, w, c))),
                    ),
            );
        for n in notes {
            card = card.child(
                div()
                    .v_flex()
                    .gap_1()
                    .p_2()
                    .rounded_md()
                    .border_l_2()
                    .border_color(skin.severity(n.severity))
                    .bg(skin.base.opacity(0.4))
                    .child(
                        div()
                            .h_flex()
                            .gap_2()
                            .text_size(px(12.))
                            .child(div().text_color(skin.text).child(n.title.clone()))
                            .child(div().text_color(skin.muted).child(n.source.clone())),
                    )
                    .when_some(n.body.clone(), |d, body| {
                        d.child(div().text_size(px(12.)).whitespace_normal().child(body))
                    }),
            );
        }
        if let Some(r) = release {
            card = card.child(
                div()
                    .p_2()
                    .rounded_md()
                    .border_l_2()
                    .border_color(r.release.map_or(skin.muted, |i| skin.release(i)))
                    .bg(skin.base.opacity(0.4))
                    .text_size(px(12.))
                    .child(r.label),
            );
        }
        if !comments.is_empty() {
            let mut thread = div()
                .id("line-thread-scroll")
                .v_flex()
                .gap_3()
                .when(reading, |d| d.flex_1().min_h_0())
                .when(!reading, |d| d.max_h(px(240.)))
                .track_scroll(&self.thread_scroll)
                .overflow_y_scroll();
            for c in comments {
                thread = thread.child(
                    div()
                        .v_flex()
                        .gap_2()
                        .flex_shrink_0()
                        .p_3()
                        .rounded_md()
                        .bg(skin.base.opacity(0.4))
                        .pb_3()
                        .border_b_1()
                        .border_color(skin.border)
                        .child(
                            div()
                                .h_flex()
                                .gap_2()
                                .text_size(px(12.))
                                .child(div().text_color(skin.accent).child(c.author.clone()))
                                .when_some(c.created_at.as_deref(), |d, at| {
                                    d.child(div().text_color(skin.muted).child(short_timestamp(at)))
                                }),
                        )
                        .child(
                            TextView::markdown(
                                format!("thread-body-{}", c.id),
                                visible_markdown(&c.body),
                            )
                            .selectable(true)
                            .text_size(px(14.))
                            .line_height(px(22.)),
                        ),
                );
            }
            card = card.child(thread);
        }
        if source.is_some() && !reading {
            card = card.child(
                self.comment_composer(self.busy || selected.is_some_and(|d| d.published), cx),
            );
            let status = if selected.is_some_and(|d| d.published) {
                "Published comment"
            } else if selected.is_some_and(|d| !d.is_saved()) {
                "Saving draft…"
            } else if selected.is_some() {
                "Draft saved locally"
            } else {
                "Drafts remain on this device until you submit a review"
            };
            card = card.child(
                div()
                    .h_flex()
                    .gap_2()
                    .child(
                        div()
                            .flex_1()
                            .text_size(px(11.))
                            .text_color(skin.muted)
                            .child(status),
                    )
                    .when(selected.is_some_and(|d| !d.published), |row| {
                        row.child(
                            Button::new("discard-line")
                                .ghost()
                                .small()
                                .cursor_pointer()
                                .label(
                                    if self.discard_candidate.as_ref()
                                        == self.selected_draft.as_ref()
                                    {
                                        "Confirm discard"
                                    } else {
                                        "Discard"
                                    },
                                )
                                .on_click(cx.listener(|a, _, w, c| {
                                    a.discard_selected(c);
                                    if a.busy {
                                        a.command(Command::Cancel, w, c);
                                    }
                                })),
                        )
                    })
                    .child(
                        Button::new("done-line")
                            .primary()
                            .small()
                            .cursor_pointer()
                            .label("Done")
                            .tooltip(format!(
                                "Close · {}",
                                crate::commands::display_key("primary-enter")
                            ))
                            .on_click(cx.listener(|a, _, w, c| a.command(Command::Cancel, w, c))),
                    ),
            );
        } else if source.is_none() {
            card = card.child(
                div()
                    .text_size(px(12.))
                    .text_color(skin.muted)
                    .child("This thread refers to source outside the loaded revision."),
            );
        }
        if selected.is_some_and(|d| !d.is_saved()) {
            card = card.child(
                Button::new("retry-line-save")
                    .ghost()
                    .small()
                    .label("Retry saving")
                    .on_click(cx.listener(|a, _, _, c| a.retry_saves(c))),
            );
            if self.status.contains("NOT saved") {
                card = card.child(
                    div()
                        .text_color(skin.warning)
                        .whitespace_normal()
                        .child(self.status.clone()),
                );
            }
        }
        let size = window.viewport_size();
        let width = (f32::from(size.width) - 48.).clamp(280., if reading { 860. } else { 600. });
        let point = if reading {
            None
        } else {
            source.and_then(|s| self.viewport.as_ref()?.borrow().source_screen_point(&s.end))
        };
        let x = point
            .map_or((f32::from(size.width) - width) / 2., |p| f32::from(p.x))
            .clamp(16., (f32::from(size.width) - width - 16.).max(16.));
        let y = point
            .map_or(90., |p| f32::from(p.y) + 8.)
            .clamp(48., (f32::from(size.height) - 430.).max(48.));
        let y = if reading { 48. } else { y };
        let mut backdrop = self.modal_backdrop(cx).child(
            card.absolute()
                .left(px(x))
                .top(px(y))
                .w(px(width))
                .max_h(px((f32::from(size.height) - y - 16.).max(200.)))
                .when(!reading, |d| d.overflow_y_scroll()),
        );
        if let Some(menu) = self.composer.comment_slash.filter(|_| {
            source.is_some()
                && !reading
                && !self.busy
                && !selected.is_some_and(|draft| draft.published)
                && !self.composer.comment_preview
        }) {
            let viewport_width = f32::from(size.width);
            let preferred_width: f32 = match menu {
                crate::comment_editor::SlashMenu::Commands => 340.,
                crate::comment_editor::SlashMenu::Table => 296.,
                crate::comment_editor::SlashMenu::Language => 264.,
                crate::comment_editor::SlashMenu::Replies => 320.,
            };
            let menu_width = preferred_width.min((viewport_width - 32.).max(180.));
            let menu_x = (x + width - menu_width - 16.)
                .clamp(16., (viewport_width - menu_width - 16.).max(16.));
            let menu_y = (y + 80.).min((f32::from(size.height) - 184.).max(16.));
            let menu_height = (f32::from(size.height) - menu_y - 16.).max(120.);
            backdrop = backdrop.child(
                div()
                    .absolute()
                    .left(px(menu_x))
                    .top(px(menu_y))
                    .child(self.comment_slash_menu(menu, menu_width, menu_height, window, cx)),
            );
        }
        backdrop.into_any_element()
    }
}
