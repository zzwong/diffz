use crate::{
    app::{Panel, Workbench},
    commands::Command,
};
use diffz_core::{
    domain::*,
    review_details::{short_timestamp, thread_roots_at, visible_markdown},
};
use gpui_kit::component::{
    Disableable, Sizable, StyledExt, button::*, input::Textarea, text::TextView,
};
use gpui_kit::{prelude::*, *};
use std::collections::BTreeMap;

impl Workbench {
    /// Return the root comment and its source line when this revision still has that line.
    fn thread_target(&self, root: u64) -> Option<(ThreadComment, Option<SourcePoint>)> {
        let a = self.active.as_ref()?;
        let comment = a
            .snapshot
            .comments
            .iter()
            .find(|c| c.id == root)
            .or_else(|| a.snapshot.comments.iter().find(|c| c.root_id == root))
            .cloned()?;
        let point = a
            .snapshot
            .patch
            .files
            .iter()
            .find(|f| f.display_path() == comment.path)
            .and_then(|f| {
                Some(SourcePoint {
                    snapshot: a.snapshot.id.clone(),
                    file: f.id.clone(),
                    side: comment.side?,
                    line: comment.line?,
                    byte_column: 0,
                })
            })
            .filter(|p| a.snapshot.validate_point(p).is_ok());
        Some((comment, point))
    }
    /// Select and highlight a thread line from the overview without opening the reader.
    pub(crate) fn jump_to_thread(
        &mut self,
        root: u64,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some((comment, point)) = self.thread_target(root) else {
            return;
        };
        let Some(point) = point else {
            self.status = format!(
                "{} · {}'s thread is outdated; this revision has no matching line",
                comment.path, comment.author
            );
            cx.notify();
            return;
        };
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
        self.thread_root = None;
        self.status = format!(
            "{}:{} · {}'s thread · press Enter or click the line to open it",
            comment.path,
            comment.line.unwrap_or(0),
            comment.author
        );
        self.diff_focus.focus(window, cx);
        self.schedule_view_save(cx);
        cx.notify();
    }
    pub(crate) fn show_thread(&mut self, root: u64, window: &mut Window, cx: &mut Context<Self>) {
        let Some((_, point)) = self.thread_target(root) else {
            return;
        };
        self.line_context = None;
        if let Some(point) = point {
            self.select_file(point.file.clone(), cx);
            if let Some(v) = &self.viewport {
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
        self.draft_input
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
        let title = source
            .map(|s| {
                if s.start.side == Side::Right && s.start.line == 0 && s.end.line == 0 {
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
                Textarea::new(&self.draft_input)
                    .h(px(112.))
                    .readonly(self.busy || selected.is_some_and(|d| d.published))
                    .aria_label("Local line comment"),
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
        self.modal_backdrop(cx)
            .child(
                card.absolute()
                    .left(px(x))
                    .top(px(y))
                    .w(px(width))
                    .max_h(px((f32::from(size.height) - y - 16.).max(200.)))
                    .when(!reading, |d| d.overflow_y_scroll()),
            )
            .into_any_element()
    }
    pub(crate) fn inspector(&self, cx: &mut Context<Self>) -> AnyElement {
        let skin = self.skin();
        let mut pane = div()
            .id("review-overview")
            .track_focus(&self.overview_focus)
            .tab_group()
            .on_hover(cx.listener(|a, hovered: &bool, _, _| a.overview_hovered = *hovered))
            .on_key_down(cx.listener(|a, e: &KeyDownEvent, w, c| {
                match e.keystroke.key.as_str() {
                    "down" => {
                        w.focus_next(c);
                    }
                    "up" => {
                        w.focus_prev(c);
                    }
                    "left" => a.overview_tab = (a.overview_tab + 2) % 3,
                    "right" => a.overview_tab = (a.overview_tab + 1) % 3,
                    _ => return,
                }
                c.stop_propagation();
                c.notify();
            }))
            .v_flex()
            .size_full()
            .bg(skin.surface)
            .border_l_1()
            .border_color(skin.border)
            .child(
                div()
                    .h_flex()
                    .px_3()
                    .h(px(40.))
                    .child(div().flex_1().child("Overview of this review"))
                    .child(
                        Button::new("close-overview")
                            .ghost()
                            .small()
                            .cursor_pointer()
                            .label("Close")
                            .tooltip(crate::commands::tip("Close overview", Command::Inspector))
                            .on_click(
                                cx.listener(|a, _, w, c| a.command(Command::Inspector, w, c)),
                            ),
                    ),
            );
        let mut tabs = div().h_flex().gap_1().px_3().pb_2();
        for (i, label) in ["Description", "Comments", "Checks"]
            .into_iter()
            .enumerate()
        {
            tabs = tabs.child(
                Button::new(("overview-tab", i))
                    .ghost()
                    .small()
                    .cursor_pointer()
                    .label(label)
                    .when(self.overview_tab == i, |b| {
                        b.bg(skin.accent.opacity(0.15)).text_color(skin.accent)
                    })
                    .on_click(cx.listener(move |a, _, _, c| {
                        a.overview_tab = i;
                        c.notify();
                    })),
            );
        }
        pane = pane.child(tabs);
        let mut body = div()
            .id("overview-scroll")
            .v_flex()
            .gap_3()
            .p_3()
            .flex_1()
            .min_h_0()
            .overflow_y_scroll();
        if let Some(active) = &self.active {
            match self.overview_tab {
                0 => {
                    body = body.child(
                        div()
                            .text_size(px(14.))
                            .whitespace_normal()
                            .child(active.snapshot.title.clone()),
                    );
                    if let Some(remote) = &active.snapshot.remote {
                        body = body.child(div().text_size(px(11.)).text_color(skin.muted).child(
                            format!(
                                "{} · head {}",
                                if remote.open {
                                    "Open review"
                                } else {
                                    "Closed review"
                                },
                                &remote.head[..remote.head.len().min(10)]
                            ),
                        ));
                    }
                    body=body.child(if let Some(description)=&active.snapshot.overview.description {
                        if description.trim().is_empty() {div().text_color(skin.muted).child("This review has no description.").into_any_element()}
                        else {TextView::markdown("pr-description",visible_markdown(description)).selectable(true).text_size(px(13.)).into_any_element()}
                    } else {div().text_color(skin.muted).whitespace_normal().child(if active.snapshot.remote.is_some(){"The saved review did not capture the pull request description; reload it from the provider to bring it in."}else{"Local review; no pull request details."}).into_any_element()});
                }
                1 => {
                    let mut roots = BTreeMap::new();
                    for c in &active.snapshot.comments {
                        if c.id == c.root_id || !roots.contains_key(&c.root_id) {
                            roots.insert(c.root_id, c);
                        }
                    }
                    let mut summary = format!(
                        "{} drafts saved locally · {} threads pinned to lines · {} comments from conversation",
                        active.drafts.len(),
                        roots.len(),
                        active.snapshot.overview.conversation.len()
                    );
                    if let Some(secs) = active.snapshot.overview.captured_at {
                        summary.push_str(&format!(
                            " · captured {}",
                            diffz_core::timefmt::short_unix_timestamp(secs)
                        ));
                    }
                    body = body.child(
                        div()
                            .text_size(px(11.))
                            .text_color(skin.muted)
                            .child(summary),
                    );
                    let total = active.drafts.len()
                        + roots.len()
                        + active.snapshot.overview.conversation.len();
                    let page = self.comment_page.min(total.saturating_sub(1) / 20);
                    let start = page * 20;
                    let entries: Vec<_> = (0..active.drafts.len())
                        .map(|i| (0, i))
                        .chain((0..roots.len()).map(|i| (1, i)))
                        .chain((0..active.snapshot.overview.conversation.len()).map(|i| (2, i)))
                        .collect();
                    let roots: Vec<_> = roots.into_values().collect();
                    for (kind, index) in entries.into_iter().skip(start).take(20) {
                        match kind {
                            0 => {
                                let d = &active.drafts[index];
                                let id = d.id.clone();
                                let path = active
                                    .snapshot
                                    .file(&d.file)
                                    .map_or("Source".into(), |f| f.display_path());
                                let line_label = if d.is_file_level() {
                                    "file comment".to_string()
                                } else {
                                    format!("{} {}", d.side.api(), d.line)
                                };
                                body = body.child(
                                    Button::new(("overview-draft", index))
                                        .ghost()
                                        .cursor_pointer()
                                        .h_auto()
                                        .flex_shrink_0()
                                        .border_1()
                                        .border_color(skin.border)
                                        .rounded_md()
                                        .bg(skin.base.opacity(0.45))
                                        .w_full()
                                        .justify_start()
                                        .accessibility_label(format!("Draft {path}:{line_label}"))
                                        .child(
                                            div()
                                                .v_flex()
                                                .gap_2()
                                                .p_2()
                                                .text_left()
                                                .line_height(px(18.))
                                                .w_full()
                                                .child(
                                                    div()
                                                        .text_size(px(11.))
                                                        .text_color(skin.accent)
                                                        .font_family(crate::theme::code_font())
                                                        .child(format!(
                                                            "Local draft · {line_label}"
                                                        )),
                                                )
                                                .child(
                                                    div()
                                                        .font_family(crate::theme::code_font())
                                                        .text_size(px(12.))
                                                        .text_ellipsis()
                                                        .child(path),
                                                )
                                                .child(
                                                    div()
                                                        .text_color(skin.muted)
                                                        .text_ellipsis()
                                                        .child(
                                                            d.body
                                                                .chars()
                                                                .take(100)
                                                                .collect::<String>(),
                                                        ),
                                                ),
                                        )
                                        .on_click(cx.listener(move |a, _, w, c| {
                                            a.select_draft(id.clone(), w, c)
                                        })),
                                );
                            }
                            1 => {
                                let t = roots[index];
                                let root = t.root_id;
                                let count = active
                                    .snapshot
                                    .comments
                                    .iter()
                                    .filter(|c| c.root_id == root)
                                    .count();
                                let plain =
                                    visible_markdown(&t.body).replace("**", "").replace('`', "");
                                let plain = plain.split_whitespace().collect::<Vec<_>>().join(" ");
                                let mut preview = plain.chars().take(180).collect::<String>();
                                if plain.chars().count() > 180 {
                                    preview.push('…');
                                }
                                let filename = t.path.rsplit('/').next().unwrap_or(&t.path);
                                let card = div()
                                    .v_flex()
                                    .flex_shrink_0()
                                    .w_full()
                                    .border_1()
                                    .border_color(skin.border)
                                    .rounded_md()
                                    .bg(skin.base.opacity(0.45))
                                    .overflow_hidden()
                                    .child(
                                        Button::new(("overview-thread", index))
                                            .tooltip(t.path.clone())
                                            .ghost()
                                            .cursor_pointer()
                                            .h_auto()
                                            .w_full()
                                            .justify_start()
                                            .accessibility_label(format!(
                                                "Thread {} {}",
                                                t.author, t.path
                                            ))
                                            .child(
                                                div()
                                                    .v_flex()
                                                    .gap_1p5()
                                                    .p_3()
                                                    .w_full()
                                                    .min_w_0()
                                                    .text_left()
                                                    .child(
                                                        div()
                                                            .h_flex()
                                                            .justify_between()
                                                            .items_center()
                                                            .child(
                                                                div()
                                                                    .text_size(px(12.))
                                                                    .font_weight(FontWeight::MEDIUM)
                                                                    .text_color(skin.accent)
                                                                    .child(t.author.clone()),
                                                            )
                                                            .child(
                                                                div()
                                                                    .h_flex()
                                                                    .gap_2()
                                                                    .text_size(px(11.))
                                                                    .text_color(skin.muted)
                                                                    .child(format!(
                                                                        "{} messages",
                                                                        count
                                                                    ))
                                                                    .when_some(
                                                                        t.created_at.as_deref(),
                                                                        |d, at| {
                                                                            d.child(
                                                                                short_timestamp(at),
                                                                            )
                                                                        },
                                                                    ),
                                                            ),
                                                    )
                                                    .child(
                                                        div()
                                                            .px_1p5()
                                                            .py_0p5()
                                                            .rounded_sm()
                                                            .bg(skin.raised)
                                                            .font_family(crate::theme::code_font())
                                                            .text_size(px(11.))
                                                            .text_ellipsis()
                                                            .when_some(t.line, |d, n| {
                                                                d.child(format!(
                                                                    "{}:{}",
                                                                    filename, n
                                                                ))
                                                            })
                                                            .when(t.line.is_none(), |d| {
                                                                d.text_color(skin.warning).child(
                                                                    format!(
                                                                        "{} · outdated",
                                                                        filename
                                                                    ),
                                                                )
                                                            }),
                                                    )
                                                    .child(
                                                        div()
                                                            .text_size(px(12.))
                                                            .line_height(px(18.))
                                                            .text_color(skin.text)
                                                            .whitespace_normal()
                                                            .child(preview),
                                                    ),
                                            )
                                            .on_click(cx.listener(move |a, _, w, c| {
                                                a.show_thread(root, w, c)
                                            })),
                                    )
                                    .child(
                                        div()
                                            .h_flex()
                                            .justify_end()
                                            .gap_1()
                                            .px_2()
                                            .py_1()
                                            .border_t_1()
                                            .border_color(skin.border)
                                            .bg(skin.surface.opacity(0.6))
                                            .when(t.line.is_some(), |d| {
                                                d.child(
                                                    Button::new(("overview-goto", index))
                                                        .ghost()
                                                        .xsmall()
                                                        .cursor_pointer()
                                                        .text_color(skin.muted)
                                                        .label("Go to line ↗")
                                                        .tooltip("Show this line in the diff")
                                                        .on_click(cx.listener(
                                                            move |a, _, w, c| {
                                                                a.jump_to_thread(root, w, c)
                                                            },
                                                        )),
                                                )
                                            })
                                            .child(
                                                Button::new(("overview-open", index))
                                                    .ghost()
                                                    .xsmall()
                                                    .cursor_pointer()
                                                    .text_color(skin.muted)
                                                    .label("Open thread")
                                                    .tooltip("Read and reply")
                                                    .on_click(cx.listener(move |a, _, w, c| {
                                                        a.show_thread(root, w, c)
                                                    })),
                                            ),
                                    );
                                body = body.child(card);
                            }
                            _ => {
                                let c = &active.snapshot.overview.conversation[index];
                                body = body.child(
                                    div()
                                        .v_flex()
                                        .gap_2()
                                        .p_2()
                                        .bg(skin.raised)
                                        .rounded_md()
                                        .child(
                                            div()
                                                .h_flex()
                                                .gap_2()
                                                .child(
                                                    div()
                                                        .text_color(skin.accent)
                                                        .child(c.author.clone()),
                                                )
                                                .when_some(c.created_at.as_deref(), |d, at| {
                                                    d.child(
                                                        div()
                                                            .text_size(px(11.))
                                                            .text_color(skin.muted)
                                                            .child(short_timestamp(at)),
                                                    )
                                                }),
                                        )
                                        .child(
                                            TextView::markdown(
                                                format!("conversation-{index}"),
                                                visible_markdown(&c.body),
                                            )
                                            .selectable(true)
                                            .text_size(px(12.)),
                                        ),
                                );
                            }
                        }
                    }
                    if total == 0 {
                        body = body.child(
                            div()
                                .text_color(skin.muted)
                                .child("This saved review contains no comments."),
                        );
                    }
                    if total > 20 {
                        body = body.child(
                            div()
                                .h_flex()
                                .gap_2()
                                .child(
                                    Button::new("comments-prev")
                                        .ghost()
                                        .small()
                                        .label("Previous")
                                        .disabled(page == 0)
                                        .on_click(cx.listener(|a, _, _, c| {
                                            a.comment_page = a.comment_page.saturating_sub(1);
                                            c.notify();
                                        })),
                                )
                                .child(format!(
                                    "{}–{} of {total}",
                                    start + 1,
                                    (start + 20).min(total)
                                ))
                                .child(
                                    Button::new("comments-next")
                                        .ghost()
                                        .small()
                                        .label("Next")
                                        .disabled(start + 20 >= total)
                                        .on_click(cx.listener(|a, _, _, c| {
                                            a.comment_page += 1;
                                            c.notify();
                                        })),
                                ),
                        );
                    }
                }
                _ => {
                    let overview = &active.snapshot.overview;
                    body=body.child(div().text_size(px(11.)).text_color(skin.muted).whitespace_normal().child(if overview.captured_at.is_some(){"This revision includes the saved status. Reload directly for current status."}else{"The saved review did not capture the checks; reload it from the provider to bring them in."}));
                    for kind in ["Workflow", "Check", "Status", "Pipeline", "Job"] {
                        let checks: Vec<_> =
                            overview.checks.iter().filter(|c| c.kind == kind).collect();
                        if checks.is_empty() {
                            continue;
                        }
                        let done = checks.iter().filter(|c| c.status == "completed").count();
                        body = body
                            .child(
                                div()
                                    .text_size(px(11.))
                                    .text_color(skin.muted)
                                    .child(format!("{} · {done}/{} completed", kind, checks.len())),
                            )
                            .child(
                                div().h(px(3.)).w_full().bg(skin.raised).child(
                                    div()
                                        .h_full()
                                        .w(relative(done as f32 / checks.len() as f32))
                                        .bg(skin.accent),
                                ),
                            );
                        for (i, check) in checks.iter().enumerate() {
                            let status = check.conclusion.as_deref().unwrap_or(&check.status);
                            let color = match status {
                                "success" => skin.positive,
                                "failure" | "timed_out" | "error" => skin.negative,
                                "in_progress" | "running" | "created" | "manual" | "queued"
                                | "pending" | "waiting" => skin.warning,
                                _ => skin.muted,
                            };
                            let mut row = div()
                                .h_flex()
                                .gap_2()
                                .py_1()
                                .child(div().text_color(color).child("●"))
                                .child(
                                    div()
                                        .v_flex()
                                        .flex_1()
                                        .min_w_0()
                                        .child(div().text_ellipsis().child(check.name.clone()))
                                        .child(
                                            div()
                                                .text_size(px(11.))
                                                .text_color(color)
                                                .child(status.replace('_', " ")),
                                        ),
                                );
                            if let Some(url) = &check.url {
                                let url = url.clone();
                                row = row.child(
                                    Button::new(format!("check-{kind}-{i}"))
                                        .ghost()
                                        .small()
                                        .cursor_pointer()
                                        .label("↗")
                                        .accessibility_label("Open check details")
                                        .on_click(move |_, _, cx| cx.open_url(&url)),
                                );
                            }
                            body = body.child(row);
                        }
                    }
                    for notice in &overview.notices {
                        body = body.child(
                            div()
                                .text_size(px(11.))
                                .text_color(skin.warning)
                                .whitespace_normal()
                                .child(notice.clone()),
                        );
                    }
                    if overview.captured_at.is_some()
                        && overview.checks.is_empty()
                        && overview.notices.is_empty()
                    {
                        body = body.child("This revision reported no checks.");
                    }
                }
            }
        }
        pane.child(body)
            .w(px(self.overview_width))
            .flex_shrink_0()
            .into_any_element()
    }
}
