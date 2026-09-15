use crate::commands::tip;
use crate::icons::AppIcon;
use crate::{
    app::{Panel, Workbench},
    commands::Command,
};
use diffz_core::patch::ChangeKind;
use gpui_kit::component::{Icon, IconName, Sizable, StyledExt, TitleBar, button::*, input::Input};
use gpui_kit::{prelude::*, *};

fn titlebar_left_inset(is_macos: bool, is_fullscreen: bool) -> Option<f32> {
    (is_macos && is_fullscreen).then_some(0.)
}

impl Workbench {
    pub(crate) fn activate_tree(
        &mut self,
        index: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
        focus_source: bool,
    ) {
        let Some(row) = self.tree_rows.get(index).cloned() else {
            return;
        };
        self.tree_cursor = index;
        if let Some(id) = row.file {
            self.select_file(id, cx);
            if focus_source {
                self.diff_focus.focus(window, cx);
            }
        } else {
            if !self.collapsed_dirs.remove(&row.path) {
                self.collapsed_dirs.insert(row.path);
            }
            self.filter_files(cx);
            self.tree_focus.focus(window, cx);
        }
        cx.notify();
    }
    pub(crate) fn files(&self, cx: &mut Context<Self>) -> AnyElement {
        let rows = self.tree_rows.clone();
        let selected = self.viewport.as_ref().map(|v| v.borrow().file.clone());
        let entity = cx.entity();
        let skin = self.skin();
        let cursor = self.tree_cursor;
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
        let visible_ids: Vec<String> = self.visible_files.iter().map(|id| id.0.clone()).collect();
        let (done, total) = diffz_core::review_details::reviewed_progress(&viewed, &visible_ids);
        div()
            .id("file-tree")
            .track_focus(&self.tree_focus)
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
            .child(
                list(self.file_list.clone(), move |index, _, _| {
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
                    let icon = if row.file.is_some() {
                        div()
                            .h_flex()
                            .items_center()
                            .gap_0p5()
                            .child(
                                Icon::new(AppIcon::for_path(&row.path))
                                    .size(px(13.))
                                    .text_color(skin.muted),
                            )
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
                                .tooltip(row.path.clone())
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
    pub(crate) fn topbar(&self, window: &Window, cx: &mut Context<Self>) -> AnyElement {
        let skin = self.skin();
        let title = self
            .active
            .as_ref()
            .map_or("diffz", |a| a.snapshot.title.as_str());
        let remote = self
            .active
            .as_ref()
            .and_then(|a| a.snapshot.remote.as_ref());
        let (pill_label, pill_fg, pill_bg) = match remote {
            None => ("Local", skin.accent, skin.accent.opacity(0.1)),
            Some(r) if r.draft => ("Draft", skin.warning, skin.warning.opacity(0.12)),
            Some(r) if !r.open => ("Closed", skin.muted, skin.muted.opacity(0.12)),
            Some(_) => ("Review", skin.accent, skin.accent.opacity(0.1)),
        };
        let author = self
            .active
            .as_ref()
            .and_then(|a| a.snapshot.overview.author.clone());
        let decision = self
            .active
            .as_ref()
            .and_then(|a| a.snapshot.overview.decision);
        let totals = self.active.as_ref().map(|a| {
            (
                a.snapshot
                    .patch
                    .files
                    .iter()
                    .map(|f| f.additions())
                    .sum::<usize>(),
                a.snapshot
                    .patch
                    .files
                    .iter()
                    .map(|f| f.deletions())
                    .sum::<usize>(),
            )
        });
        let mut titlebar = TitleBar::new();
        if let Some(inset) = titlebar_left_inset(cfg!(target_os = "macos"), window.is_fullscreen())
        {
            titlebar = titlebar.pl(px(inset));
        }
        titlebar
            .on_close_window(cx.listener(|a, _, window, cx| {
                if a.unsaved() || a.busy {
                    a.status = "Let pending changes finish saving before you close.".into();
                    cx.notify();
                } else {
                    window.remove_window();
                }
            }))
            .bg(skin.surface)
            .border_color(skin.border)
            // gpui-component puts children into a flex_shrink_0 bar, so an in-flow row always
            // matches its content width. The absolute inner row uses the wrapper's definite
            // width, and the title can then truncate.
            .child(
                div().relative().flex_1().min_w_0().h_full().child(
                    div()
                        .absolute()
                        .top_0()
                        .left_0()
                        .size_full()
                        .h_flex()
                        .gap_2()
                        .pr_2()
                        .child(
                            Button::new("files")
                                .cursor_pointer()
                                .ghost()
                                .small()
                                .icon(IconName::PanelLeft)
                                .accessibility_label("Toggle file tree")
                                .tooltip(tip("Toggle file tree", Command::Files))
                                .on_click(
                                    cx.listener(|a, _, w, c| a.command(Command::Files, w, c)),
                                ),
                        )
                        .child(
                            div()
                                .px_2()
                                .py_0p5()
                                .rounded_md()
                                .text_size(px(10.))
                                .text_color(pill_fg)
                                .bg(pill_bg)
                                .child(pill_label),
                        )
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .text_ellipsis()
                                .text_size(px(12.))
                                .font_weight(FontWeight::MEDIUM)
                                .child(title.to_owned()),
                        )
                        .when_some(author, |d, author| {
                            d.child(
                                div()
                                    .flex_shrink_0()
                                    .text_size(px(11.))
                                    .text_color(skin.muted)
                                    .child(format!("by {author}")),
                            )
                        })
                        .when_some(decision, |d, decision| {
                            use diffz_core::review_details::ReviewDecision;
                            let fg = match decision {
                                ReviewDecision::Approved => skin.positive,
                                ReviewDecision::ChangesRequested => skin.negative,
                            };
                            d.child(
                                div()
                                    .flex_shrink_0()
                                    .px_2()
                                    .py_0p5()
                                    .rounded_md()
                                    .border_1()
                                    .border_color(fg.opacity(0.5))
                                    .bg(fg.opacity(0.1))
                                    .text_size(px(10.))
                                    .text_color(fg)
                                    .child(decision.label()),
                            )
                        })
                        .when_some(totals, |d, (adds, dels)| {
                            d.child(
                                div()
                                    .h_flex()
                                    .flex_shrink_0()
                                    .gap_2()
                                    .text_size(px(11.))
                                    .child(
                                        div().text_color(skin.positive).child(format!("+{adds}")),
                                    )
                                    .child(
                                        div().text_color(skin.negative).child(format!("−{dels}")),
                                    ),
                            )
                        })
                        .child(
                            Button::new("open")
                                .flex_shrink_0()
                                .cursor_pointer()
                                .ghost()
                                .small()
                                .label("Open")
                                .tooltip(tip("Open source", Command::Open))
                                .on_click(cx.listener(|a, _, w, c| a.command(Command::Open, w, c))),
                        )
                        .child(
                            Button::new("preview")
                                .flex_shrink_0()
                                .cursor_pointer()
                                .primary()
                                .small()
                                .label("Review")
                                .tooltip(tip("Review summary and local drafts", Command::Preview))
                                .on_click(
                                    cx.listener(|a, _, w, c| a.command(Command::Preview, w, c)),
                                ),
                        ),
                ),
            )
            .into_any_element()
    }
    pub(crate) fn toolbar(&self, cx: &mut Context<Self>) -> AnyElement {
        let skin = self.skin();
        let (wrap, split, font, path, adds, dels) = self
            .viewport
            .as_ref()
            .map(|v| {
                let v = v.borrow();
                let file = v.snapshot.file(&v.file);
                (
                    v.wrap,
                    v.split,
                    v.font_size,
                    file.as_ref().map_or(String::new(), |f| f.display_path()),
                    file.as_ref().map_or(0, |f| f.additions()),
                    file.as_ref().map_or(0, |f| f.deletions()),
                )
            })
            .unwrap_or((true, false, 14., String::new(), 0, 0));
        let prose = self.viewport.is_some() && diffz_core::presentation::default_wrap(&path);
        let rich = self.settings.rich;
        let rich_inline = self.settings.rich_inline;
        let reviewed = self
            .active
            .as_ref()
            .zip(self.viewport.as_ref())
            .is_some_and(|(a, v)| {
                a.view
                    .preferences
                    .viewed
                    .get(&v.borrow().file.0)
                    .copied()
                    .unwrap_or(false)
            });
        div()
            .h_flex()
            .h(px(38.))
            .flex_shrink_0()
            .px_3()
            .gap_1()
            .border_b_1()
            .border_color(skin.border)
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .text_ellipsis_middle()
                    .text_size(px(12.))
                    .font_family(crate::theme::code_font())
                    .child(path),
            )
            .child(
                Button::new("reviewed")
                    .cursor_pointer()
                    .ghost()
                    .small()
                    .label(if reviewed {
                        "✓ Reviewed"
                    } else {
                        "Mark reviewed"
                    })
                    .accessibility_label(if reviewed {
                        "Mark file unreviewed"
                    } else {
                        "Mark file reviewed"
                    })
                    .when(reviewed, |b| {
                        b.text_color(skin.positive).bg(skin.positive.opacity(0.1))
                    })
                    .tooltip("Mark this file reviewed or unreviewed · Space in the file tree")
                    .on_click(cx.listener(|a, _, _, c| a.toggle_reviewed(c))),
            )
            .when(self.viewport.is_some(), |b| {
                b.child(
                    div()
                        .h_flex()
                        .gap_1()
                        .flex_shrink_0()
                        .text_size(px(11.))
                        .child(div().text_color(skin.positive).child(format!("+{adds}")))
                        .child(div().text_color(skin.negative).child(format!("−{dels}"))),
                )
            })
            .child(
                Button::new("wrap")
                    .cursor_pointer()
                    .ghost()
                    .small()
                    .icon(Icon::new(AppIcon::WrapText))
                    .accessibility_label(if wrap {
                        "Soft wrap: on"
                    } else {
                        "Soft wrap: off"
                    })
                    .tooltip(tip(
                        if wrap {
                            "Soft wrap: on"
                        } else {
                            "Soft wrap: off"
                        },
                        Command::Wrap,
                    ))
                    .when(wrap, |b| {
                        b.bg(skin.accent.opacity(0.13)).text_color(skin.accent)
                    })
                    .on_click(cx.listener(|a, _, w, c| a.command(Command::Wrap, w, c))),
            )
            .child(
                Button::new("split")
                    .cursor_pointer()
                    .ghost()
                    .small()
                    .icon(Icon::new(if split {
                        AppIcon::Columns2
                    } else {
                        AppIcon::Rows2
                    }))
                    .accessibility_label(if split {
                        "Switch to unified layout"
                    } else {
                        "Switch to split layout"
                    })
                    .tooltip(tip(
                        if split {
                            "Switch to unified layout"
                        } else {
                            "Switch to split layout"
                        },
                        Command::Split,
                    ))
                    .when(split, |b| {
                        b.bg(skin.accent.opacity(0.13)).text_color(skin.accent)
                    })
                    .on_click(cx.listener(|a, _, w, c| a.command(Command::Split, w, c))),
            )
            .when(prose, |b| {
                b.child(
                    Button::new("rich")
                        .cursor_pointer()
                        .ghost()
                        .small()
                        .icon(IconName::BookOpen)
                        .accessibility_label(if rich {
                            "Show source diff"
                        } else {
                            "Show rendered prose diff"
                        })
                        .tooltip(tip(
                            if rich {
                                "Rich diff: on"
                            } else {
                                "Rich diff: off"
                            },
                            Command::Rich,
                        ))
                        .when(rich, |b| {
                            b.bg(skin.accent.opacity(0.13)).text_color(skin.accent)
                        })
                        .on_click(cx.listener(|a, _, w, c| a.command(Command::Rich, w, c))),
                )
                .when(rich, |b| {
                    b.child(
                        Button::new("rich-inline")
                            .cursor_pointer()
                            .ghost()
                            .small()
                            .icon(IconName::ALargeSmall)
                            .accessibility_label(if rich_inline {
                                "Turn off word marks in rich diff"
                            } else {
                                "Turn on word marks in rich diff"
                            })
                            .tooltip(tip(
                                if rich_inline {
                                    "Word marks: on (prototype)"
                                } else {
                                    "Word marks: off (prototype)"
                                },
                                Command::RichInline,
                            ))
                            .when(rich_inline, |b| {
                                b.bg(skin.accent.opacity(0.13)).text_color(skin.accent)
                            })
                            .on_click(
                                cx.listener(|a, _, w, c| a.command(Command::RichInline, w, c)),
                            ),
                    )
                })
            })
            .child(div().w(px(1.)).h(px(16.)).mx_2().bg(skin.border))
            .child(
                Button::new("smaller")
                    .cursor_pointer()
                    .ghost()
                    .small()
                    .label("−")
                    .tooltip(tip("Smaller code text", Command::ZoomOut))
                    .on_click(cx.listener(|a, _, w, c| a.command(Command::ZoomOut, w, c))),
            )
            .child(
                div()
                    .text_size(px(10.))
                    .text_color(skin.muted)
                    .child(format!("{font:.0}")),
            )
            .child(
                Button::new("larger")
                    .cursor_pointer()
                    .ghost()
                    .small()
                    .label("+")
                    .tooltip(tip("Larger code text", Command::ZoomIn))
                    .on_click(cx.listener(|a, _, w, c| a.command(Command::ZoomIn, w, c))),
            )
            .child(
                Button::new("find")
                    .cursor_pointer()
                    .ghost()
                    .small()
                    .icon(IconName::Search)
                    .accessibility_label("Find in diff")
                    .tooltip(tip("Find in diff", Command::Find))
                    .on_click(cx.listener(|a, _, w, c| a.command(Command::Find, w, c))),
            )
            .child(
                Button::new("notes")
                    .cursor_pointer()
                    .ghost()
                    .small()
                    .label("Overview")
                    .tooltip(tip("Description, comments and checks", Command::Inspector))
                    .on_click(cx.listener(|a, _, w, c| a.command(Command::Inspector, w, c))),
            )
            .into_any_element()
    }
    pub(crate) fn footer(&self, window: &Window, cx: &mut Context<Self>) -> AnyElement {
        let skin = self.skin();
        let primary = if cfg!(target_os = "macos") {
            "⌘"
        } else {
            "Ctrl+"
        };
        // Every ghost button paints its own foreground, so the tone goes on per button.
        let hint = skin.muted.opacity(0.75);
        let mut strip = div()
            .h_flex()
            .gap_1()
            .px_2()
            .h(px(30.))
            .flex_shrink_0()
            .border_t_1()
            .border_color(skin.border)
            .bg(skin.surface)
            .text_size(px(11.))
            .text_color(hint);
        let wide = f32::from(window.viewport_size().width) >= 1150.;
        for (id, key, label, command, extra) in [
            ("hint-open", "primary-o", "open", Command::Open, true),
            ("hint-files", "[ ]", "files", Command::NextFile, false),
            ("hint-next-hunk", "N", "next hunk", Command::NextHunk, false),
            (
                "hint-prev-hunk",
                "P",
                "previous hunk",
                Command::PreviousHunk,
                false,
            ),
            ("hint-find", "primary-f", "find", Command::Find, false),
            ("hint-wrap", "alt-z", "wrap", Command::Wrap, false),
            ("hint-split", "primary-alt-s", "split", Command::Split, true),
            ("hint-copy", "primary-c", "copy", Command::Copy, true),
            ("hint-comment", "C", "comment", Command::Comment, false),
            (
                "hint-comment-file",
                "⇧C",
                "file comment",
                Command::CommentFile,
                false,
            ),
            (
                "hint-review",
                "primary-enter",
                "review",
                Command::Preview,
                true,
            ),
        ] {
            if extra && !wide {
                continue;
            }
            let key = crate::commands::display_key(key);
            strip = strip.child(
                Button::new(id)
                    .accessibility_label(format!("{label} ({key})"))
                    .cursor_pointer()
                    .ghost()
                    .small()
                    .text_color(hint)
                    .child(
                        div()
                            .h_flex()
                            .justify_center()
                            .h(px(18.))
                            .min_w(px(18.))
                            .flex_shrink_0()
                            .px(px(4.))
                            .line_height(px(14.))
                            .border_1()
                            .border_color(skin.border)
                            .rounded_sm()
                            .text_size(px(10.))
                            .child(key),
                    )
                    .child(div().text_size(px(11.)).line_height(px(16.)).child(label))
                    .on_click(cx.listener(move |a, _, w, c| a.command(command, w, c))),
            );
        }
        strip
            .child(
                Button::new("palette")
                    .accessibility_label("Command palette")
                    .cursor_pointer()
                    .ghost()
                    .small()
                    .text_color(hint)
                    .child(
                        div()
                            .h_flex()
                            .justify_center()
                            .h(px(18.))
                            .min_w(px(18.))
                            .flex_shrink_0()
                            .px(px(4.))
                            .line_height(px(14.))
                            .border_1()
                            .border_color(skin.border)
                            .rounded_sm()
                            .text_size(px(10.))
                            .child(format!("{primary}K")),
                    )
                    .child(
                        div()
                            .text_size(px(11.))
                            .line_height(px(16.))
                            .child("commands"),
                    )
                    .on_click(cx.listener(|a, _, w, c| a.command(Command::Palette, w, c))),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .text_ellipsis()
                    .child(if self.unsaved() {
                        "Saving…".into()
                    } else {
                        self.status.clone()
                    }),
            )
            .child(
                Button::new("outbox")
                    .cursor_pointer()
                    .ghost()
                    .small()
                    .label("Outbox")
                    .tooltip("Local submission history")
                    .on_click(cx.listener(|a, _, w, c| {
                        a.return_focus = w.focused(c);
                        a.panel = Panel::Outbox;
                        a.panel_focus.focus(w, c);
                        a.refresh_outbox(c);
                        c.notify();
                    })),
            )
            .child(
                Button::new("theme")
                    .cursor_pointer()
                    .ghost()
                    .small()
                    .icon(if self.dark {
                        IconName::Sun
                    } else {
                        IconName::Moon
                    })
                    .tooltip("Toggle appearance")
                    .accessibility_label("Toggle light or dark appearance")
                    .on_click(cx.listener(|a, _, w, c| a.command(Command::Theme, w, c))),
            )
            .child(
                Button::new("themes")
                    .cursor_pointer()
                    .ghost()
                    .small()
                    .icon(IconName::Palette)
                    .tooltip("Choose theme")
                    .accessibility_label("Choose theme")
                    .on_click(cx.listener(|a, _, w, c| a.command(Command::Themes, w, c))),
            )
            .into_any_element()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[::core::prelude::v1::test]
    fn fullscreen_macos_titlebar_drops_the_dependency_left_inset() {
        assert_eq!(titlebar_left_inset(true, true), Some(0.));
    }

    #[::core::prelude::v1::test]
    fn titlebar_keeps_the_dependency_inset_outside_macos_fullscreen() {
        assert_eq!(titlebar_left_inset(true, false), None);
        assert_eq!(titlebar_left_inset(false, true), None);
    }
}
