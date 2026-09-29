use crate::{app::Workbench, commands::Command};
use diffz_core::{
    annotation::Anchor,
    review_details::{ThreadLocation, short_timestamp, thread_location, visible_markdown},
};
use gpui_kit::component::{Disableable, Sizable, StyledExt, button::*, text::TextView};
use gpui_kit::{prelude::*, *};
use std::collections::BTreeMap;

mod thread;

impl Workbench {
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
                    "left" => a.overview_tab = (a.overview_tab + 3) % 4,
                    "right" => a.overview_tab = (a.overview_tab + 1) % 4,
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
        for (i, label) in ["Description", "Comments", "Checks", "Annotations"]
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
                                if remote.compare.is_some() {
                                    "Read-only compare"
                                } else if remote.merged {
                                    "Merged review"
                                } else if remote.open {
                                    "Open review"
                                } else {
                                    "Closed review"
                                },
                                &remote.head[..remote.head.len().min(10)]
                            ),
                        ));
                    }
                    if let Some(timeline) = self.release_timeline(cx) {
                        body = body.child(timeline);
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
                        "{} drafts saved locally · {} review threads · {} conversation comments",
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
                                let location = thread_location(&active.snapshot, t);
                                let location_label = match &location {
                                    ThreadLocation::Line(_) => {
                                        format!("{}:{}", filename, t.line.unwrap_or(0))
                                    }
                                    ThreadLocation::File(_) => {
                                        format!("{} · file comment", filename)
                                    }
                                    ThreadLocation::Outdated => format!("{} · outdated", filename),
                                };
                                let go_label = match &location {
                                    ThreadLocation::Line(_) => Some("Go to line ↗"),
                                    ThreadLocation::File(_) => Some("Go to file ↗"),
                                    ThreadLocation::Outdated => None,
                                };
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
                                                            .when(
                                                                matches!(
                                                                    location,
                                                                    ThreadLocation::Outdated
                                                                ),
                                                                |d| d.text_color(skin.warning),
                                                            )
                                                            .child(location_label),
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
                                            .when_some(go_label, |d, go_label| {
                                                d.child(
                                                    Button::new(("overview-goto", index))
                                                        .ghost()
                                                        .xsmall()
                                                        .cursor_pointer()
                                                        .text_color(skin.muted)
                                                        .label(go_label)
                                                        .tooltip(
                                                            "Show this thread location in the diff",
                                                        )
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
                3 => {
                    if self.annotations.is_empty() {
                        body = body.child(
                            div()
                                .text_size(px(12.))
                                .text_color(skin.muted)
                                .child("No annotations for this review."),
                        );
                    }
                    for (index, n) in self.annotations.iter().enumerate() {
                        let color = skin.severity(n.severity);
                        let place = match &n.anchor {
                            Anchor::File { path } => path.clone(),
                            Anchor::Lines {
                                path, start, end, ..
                            } if start == end => format!("{path}:{start}"),
                            Anchor::Lines {
                                path, start, end, ..
                            } => format!("{path}:{start}–{end}"),
                        };
                        body = body.child(
                            Button::new(("annotation", index))
                                .ghost()
                                .small()
                                .cursor_pointer()
                                .justify_start()
                                .child(
                                    div()
                                        .v_flex()
                                        .items_start()
                                        .gap_0p5()
                                        .child(
                                            div()
                                                .h_flex()
                                                .gap_2()
                                                .child(div().size(px(8.)).rounded_full().bg(color))
                                                .child(
                                                    div().text_size(px(12.)).child(n.title.clone()),
                                                ),
                                        )
                                        .child(
                                            div()
                                                .text_size(px(11.))
                                                .text_color(skin.muted)
                                                .font_family(crate::theme::code_font())
                                                .child(format!("{place} · {}", n.source)),
                                        ),
                                )
                                .on_click(
                                    cx.listener(move |a, _, w, c| {
                                        a.jump_to_annotation(index, w, c)
                                    }),
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
    /// A compare's releases, oldest first: each narrows the tree or opens as its own compare.
    fn release_timeline(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let skin = self.skin();
        let snapshot = &self.active.as_ref()?.snapshot;
        let refs = snapshot.remote.as_ref()?.compare.as_ref()?;
        let releases = &snapshot.overview.releases;
        if releases.is_empty() {
            return self.releases_pending.as_ref().map(|_| {
                div()
                    .text_size(px(12.))
                    .text_color(skin.muted)
                    .child("Loading releases…")
                    .into_any_element()
            });
        }
        let mut timeline = div().v_flex().gap_2().flex_shrink_0().child(
            div()
                .h_flex()
                .items_center()
                .gap_2()
                .child(
                    div()
                        .flex_1()
                        .text_size(px(12.))
                        .font_weight(FontWeight::MEDIUM)
                        .child(format!("Releases · {}", releases.len())),
                )
                .when(self.release_filter.is_some(), |d| {
                    d.child(
                        Button::new("release-all")
                            .ghost()
                            .xsmall()
                            .cursor_pointer()
                            .label("Show all files")
                            .tooltip("Stop narrowing the file tree to one release")
                            .on_click(cx.listener(|a, _, _, c| a.filter_release(None, c))),
                    )
                }),
        );
        for (index, r) in releases.iter().enumerate() {
            let selected = self.release_filter == Some(index);
            let open_notes = self.release_notes.contains(&index);
            let (additions, deletions) = r
                .files
                .iter()
                .fold((0, 0), |(a, d), f| (a + f.additions, d + f.deletions));
            let name = match &r.tag {
                Some(tag) => tag.clone(),
                None => format!("{} (untagged)", refs.head),
            };
            let mut card = div()
                .v_flex()
                .gap_1()
                .p_2()
                .w_full()
                .flex_shrink_0()
                .border_1()
                .border_color(if selected { skin.accent } else { skin.border })
                .rounded_md()
                .bg(if selected {
                    skin.accent.opacity(0.08)
                } else {
                    skin.base.opacity(0.45)
                })
                .child(
                    div()
                        .h_flex()
                        .gap_2()
                        .items_center()
                        .child(div().size(px(8.)).rounded_full().bg(if selected {
                            skin.accent
                        } else {
                            skin.muted
                        }))
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .text_ellipsis()
                                .font_family(crate::theme::code_font())
                                .text_size(px(12.))
                                .font_weight(FontWeight::MEDIUM)
                                .child(name.clone()),
                        )
                        .when_some(r.date.as_deref(), |d, at| {
                            d.child(
                                div()
                                    .text_size(px(11.))
                                    .text_color(skin.muted)
                                    .child(short_timestamp(at)),
                            )
                        }),
                )
                .child(
                    div()
                        .h_flex()
                        .gap_2()
                        .text_size(px(11.))
                        .text_color(skin.muted)
                        .child(format!("{} commits · {} files", r.commits, r.files.len()))
                        .child(
                            div()
                                .text_color(skin.positive)
                                .child(format!("+{additions}")),
                        )
                        .child(
                            div()
                                .text_color(skin.negative)
                                .child(format!("−{deletions}")),
                        ),
                );
            let mut actions = div()
                .h_flex()
                .gap_1()
                // A release whose files the compare leaves out would narrow the tree to nothing.
                .when(!r.files.is_empty(), |d| {
                    d.child(
                        Button::new(("release-files", index))
                            .ghost()
                            .xsmall()
                            .cursor_pointer()
                            .label(if selected {
                                "Show all files"
                            } else {
                                "Show its files"
                            })
                            .when(selected, |b| b.text_color(skin.accent))
                            .tooltip(format!("Narrow the file tree to the files {name} touched"))
                            .on_click(cx.listener(move |a, _, _, c| {
                                let next = (a.release_filter != Some(index)).then_some(index);
                                a.filter_release(next, c)
                            })),
                    )
                })
                .child(
                    Button::new(("release-open", index))
                        .ghost()
                        .xsmall()
                        .cursor_pointer()
                        .label("Open this step")
                        .tooltip("Open this release's changes as a compare of their own")
                        .on_click(cx.listener(move |a, _, _, c| a.open_release_step(index, c))),
                );
            if r.notes.is_some() {
                actions = actions.child(
                    Button::new(("release-notes", index))
                        .ghost()
                        .xsmall()
                        .cursor_pointer()
                        .label(if open_notes { "Hide notes" } else { "Notes" })
                        .on_click(cx.listener(move |a, _, _, c| {
                            if !a.release_notes.remove(&index) {
                                a.release_notes.insert(index);
                            }
                            c.notify();
                        })),
                );
            }
            if let Some(url) = r.url.clone() {
                actions = actions.child(
                    Button::new(("release-link", index))
                        .ghost()
                        .xsmall()
                        .cursor_pointer()
                        .label("↗")
                        .tooltip(url.clone())
                        .accessibility_label("Open the release page")
                        .on_click(move |_, _, cx| cx.open_url(&url)),
                );
            }
            card = card.child(actions);
            if let Some(notes) = r.notes.as_deref().filter(|_| open_notes) {
                card = card.child(
                    TextView::markdown(format!("release-notes-{index}"), visible_markdown(notes))
                        .selectable(true)
                        .text_size(px(12.)),
                );
            }
            timeline = timeline.child(card);
        }
        Some(timeline.into_any_element())
    }
}
