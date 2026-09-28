use crate::{
    app::Workbench,
    commands::{Command, tip},
    icons::AppIcon,
};
use diffz_core::patch::ChangeKind;
use gpui_kit::component::{Icon, IconName, Sizable, StyledExt, button::*, input::Input};
use gpui_kit::{prelude::*, *};

impl Workbench {
    pub(crate) fn activate_tree(
        &mut self,
        index: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
        focus_source: bool,
    ) {
        let Some(row) = self.browser.rows.get(index).cloned() else {
            return;
        };
        self.browser.cursor = index;
        if let Some(id) = row.file {
            self.select_file(id, cx);
            if focus_source {
                self.diff_focus.focus(window, cx);
            }
        } else {
            if !self.browser.collapsed.remove(&row.path) {
                self.browser.collapsed.insert(row.path);
            }
            self.filter_files(cx);
            self.browser.focus.focus(window, cx);
        }
        cx.notify();
    }
    pub(crate) fn files(&self, cx: &mut Context<Self>) -> AnyElement {
        let rows = self.browser.rows.clone();
        let selected = self.viewport.as_ref().map(|v| v.borrow().file.clone());
        let entity = cx.entity();
        let skin = self.skin();
        let cursor = self.browser.cursor;
        let snapshot = self.active.as_ref().map(|a| a.snapshot.clone());
        let viewed = self
            .active
            .as_ref()
            .map(|a| a.view.preferences.viewed.clone())
            .unwrap_or_default();
        let counts = snapshot
            .as_ref()
            .map(|s| diffz_core::review_details::thread_counts(&s.comments))
            .unwrap_or_default();
        let mut marks: std::collections::HashMap<
            String,
            (usize, diffz_core::annotation::Severity),
        > = Default::default();
        for n in self.annotations.iter() {
            let slot = marks
                .entry(n.anchor.path().to_string())
                .or_insert((0, n.severity));
            *slot = (slot.0 + 1, slot.1.max(n.severity));
        }
        // A compare's releases per path: the latest one's name, and every name for the tooltip.
        let released: std::collections::HashMap<String, (String, String)> = snapshot
            .as_ref()
            .map(|s| {
                let head = s
                    .remote
                    .as_ref()
                    .and_then(|t| t.compare.as_ref())
                    .map_or("head", |c| c.head.as_str());
                let releases = &s.overview.releases;
                diffz_core::review_details::releases_by_path(releases)
                    .into_iter()
                    .map(|(path, touched)| {
                        let names: Vec<&str> =
                            touched.iter().map(|&i| releases[i].name(head)).collect();
                        let latest = names.last().copied().unwrap_or_default();
                        let badge = match names.len() {
                            1 => latest.to_owned(),
                            n => format!("{latest} +{}", n - 1),
                        };
                        (path.to_owned(), (badge, names.join(", ")))
                    })
                    .collect()
            })
            .unwrap_or_default();
        let release_filter = snapshot.as_ref().and_then(|s| {
            let r = s.overview.releases.get(self.release_filter?)?;
            let head = s.remote.as_ref()?.compare.as_ref()?.head.as_str();
            Some(r.name(head).to_owned())
        });
        let visible_ids: Vec<String> = self
            .browser
            .visible_files
            .iter()
            .map(|id| id.0.clone())
            .collect();
        let (done, total) = diffz_core::review_details::reviewed_progress(&viewed, &visible_ids);
        div()
            .id("file-tree")
            .track_focus(&self.browser.focus)
            .tab_stop(true)
            .key_context("WorkbenchTree")
            .aria_label(
                "Changed files. Move with Up and Down; Enter opens, Left collapses, Right expands.",
            )
            .v_flex()
            .w(px(self.files_width))
            .flex_shrink_0()
            .h_full()
            .bg(skin.surface)
            .border_r_1()
            .border_color(skin.border)
            .child(
                div()
                    .h_flex()
                    .items_center()
                    .px_3()
                    .h(px(38.))
                    .text_size(px(11.))
                    .text_color(skin.muted)
                    .child(
                        Button::new("switch-review")
                            .ghost()
                            .small()
                            .cursor_pointer()
                            .icon(IconName::RotateCw)
                            .label("Recent reviews")
                            .accessibility_label("Recent reviews")
                            .tooltip(tip(
                                "Go from one saved PR or merge request to another",
                                Command::Recent,
                            ))
                            .on_click(cx.listener(|a, _, w, c| a.show_recents(w, c))),
                    )
                    .child(div().flex_1())
                    .child(format!("{done}/{total} files")),
            )
            .when(total > 0, |d| {
                d.child(
                    div().w_full().h(px(2.)).bg(skin.border).child(
                        div()
                            .h_full()
                            .bg(skin.positive)
                            .w(relative(done as f32 / total as f32)),
                    ),
                )
            })
            .child(
                div()
                    .px_2()
                    .pt_2()
                    .pb_2()
                    .child(Input::new(&self.filter_input).small()),
            )
            .when_some(release_filter, |d, name| {
                d.child(
                    div()
                        .h_flex()
                        .items_center()
                        .gap_1()
                        .px_3()
                        .pb_2()
                        .text_size(px(11.))
                        .text_color(skin.accent)
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .text_ellipsis()
                                .child(format!("Only files {name} touched")),
                        )
                        .child(
                            Button::new("release-filter-clear")
                                .ghost()
                                .xsmall()
                                .cursor_pointer()
                                .label("Show all")
                                .tooltip("Show every changed file again")
                                .on_click(cx.listener(|a, _, _, c| a.filter_release(None, c))),
                        ),
                )
            })
            .child(
                list(self.browser.list.clone(), move |index, _, _| {
                    let Some(row) = rows.get(index) else {
                        return div().into_any_element();
                    };
                    let e = entity.clone();
                    let is_selected = row
                        .file
                        .as_ref()
                        .is_some_and(|id| Some(id) == selected.as_ref());
                    let reviewed = row
                        .file
                        .as_ref()
                        .is_some_and(|id| viewed.get(&id.0) == Some(&true));
                    let stats = row
                        .file
                        .as_ref()
                        .and_then(|id| snapshot.as_ref()?.file(id))
                        .map(|f| (f.additions(), f.deletions(), f.kind));
                    let comment_count = row
                        .file
                        .as_ref()
                        .and_then(|id| snapshot.as_ref()?.file(id))
                        .map(|f| counts.get(&f.display_path()).copied().unwrap_or(0))
                        .unwrap_or(0);
                    let mark = row
                        .file
                        .as_ref()
                        .and_then(|id| snapshot.as_ref()?.file(id))
                        .and_then(|f| marks.get(&f.display_path()).copied());
                    let release = row
                        .file
                        .as_ref()
                        .and_then(|_| released.get(&row.path))
                        .cloned();
                    let icon = if row.file.is_some() {
                        div()
                            .h_flex()
                            .items_center()
                            .gap_0p5()
                            .child(Icon::new(AppIcon::for_path(&row.path)).size(px(13.)))
                            .when(reviewed, |d| {
                                d.child(
                                    Icon::new(IconName::Check)
                                        .size(px(9.))
                                        .text_color(skin.positive),
                                )
                            })
                    } else {
                        div().child(
                            Icon::new(if row.expanded {
                                IconName::ChevronDown
                            } else {
                                IconName::ChevronRight
                            })
                            .size(px(13.))
                            .text_color(skin.muted),
                        )
                    };
                    div()
                        .h_flex()
                        .items_center()
                        .h(px(29.))
                        .w_full()
                        .pl(px(5. + row.depth as f32 * 12.))
                        .pr_1()
                        .bg(if is_selected {
                            skin.accent.opacity(0.13)
                        } else {
                            skin.surface
                        })
                        .border_l_2()
                        .border_color(if is_selected {
                            skin.accent
                        } else {
                            skin.surface
                        })
                        .child(
                            Button::new(("tree", index))
                                .cursor_pointer()
                                .ghost()
                                .small()
                                .w_full()
                                .justify_start()
                                .tab_stop(false)
                                .tooltip(match &release {
                                    Some((_, names)) => format!("{}\nReleases: {names}", row.path),
                                    None => row.path.clone(),
                                })
                                .accessibility_label(row.path.clone())
                                .child(icon)
                                .child(
                                    div()
                                        .flex_1()
                                        .min_w_0()
                                        .text_ellipsis()
                                        .text_size(px(12.))
                                        .text_color(if is_selected && !reviewed {
                                            skin.text
                                        } else {
                                            skin.muted
                                        })
                                        .font_family(crate::theme::code_font())
                                        .child(row.label.clone()),
                                )
                                .when_some(release, |b, (badge, _)| {
                                    b.child(
                                        div()
                                            .flex_shrink_0()
                                            .max_w(px(96.))
                                            .text_ellipsis()
                                            .px_1()
                                            .rounded_sm()
                                            .bg(skin.muted.opacity(0.12))
                                            .text_color(skin.muted)
                                            .text_size(px(10.))
                                            .font_family(crate::theme::code_font())
                                            .child(badge),
                                    )
                                })
                                .when(comment_count > 0, |b| {
                                    b.child(
                                        div()
                                            .h_flex()
                                            .gap_0p5()
                                            .px_1()
                                            .rounded_sm()
                                            .bg(skin.accent.opacity(0.12))
                                            .text_color(skin.accent)
                                            .text_size(px(10.))
                                            .child(Icon::new(AppIcon::MessageSquare).size(px(9.)))
                                            .child(format!("{comment_count}")),
                                    )
                                })
                                .when_some(mark, |b, (count, severity)| {
                                    let color = skin.severity(severity);
                                    b.child(
                                        div()
                                            .px_1()
                                            .rounded_sm()
                                            .bg(color.opacity(0.14))
                                            .text_color(color)
                                            .text_size(px(10.))
                                            .child(format!("{count}")),
                                    )
                                })
                                .when_some(stats, |b, (adds, dels, kind)| {
                                    let (badge, color) = match kind {
                                        ChangeKind::Added => ("A", skin.positive),
                                        ChangeKind::Deleted => ("D", skin.negative),
                                        ChangeKind::Modified => ("M", skin.warning),
                                        ChangeKind::Renamed => ("R", skin.accent),
                                        ChangeKind::Copied => ("C", skin.accent),
                                    };
                                    b.child(
                                        div()
                                            .h_flex()
                                            .gap_1()
                                            .text_size(px(10.))
                                            .child(
                                                div()
                                                    .text_color(if reviewed {
                                                        skin.muted.opacity(0.7)
                                                    } else {
                                                        skin.positive
                                                    })
                                                    .child(format!("+{adds}")),
                                            )
                                            .child(
                                                div()
                                                    .text_color(if reviewed {
                                                        skin.muted.opacity(0.7)
                                                    } else {
                                                        skin.negative
                                                    })
                                                    .child(format!("−{dels}")),
                                            )
                                            .child(div().text_color(color).child(badge)),
                                    )
                                })
                                .when(index == cursor, |b| {
                                    b.border_color(skin.accent.opacity(0.35))
                                })
                                .on_click(move |_, w, c| {
                                    e.update(c, |a, c| a.activate_tree(index, w, c, true));
                                }),
                        )
                        .into_any_element()
                })
                .flex_1()
                .min_h_0(),
            )
            .into_any_element()
    }
    pub(crate) fn toggle_reviewed(&mut self, cx: &mut Context<Self>) {
        if let (Some(a), Some(v)) = (&mut self.active, &self.viewport) {
            let value = a
                .view
                .preferences
                .viewed
                .entry(v.borrow().file.0.clone())
                .or_default();
            *value = !*value;
        }
        self.schedule_view_save(cx);
        cx.notify();
    }
}
