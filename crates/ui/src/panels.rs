//! GPUI Kit supplies the standard controls; review state stays outside them.
use crate::{
    app::{Panel, SourceMode, Workbench},
    commands::{COMMANDS, Command},
};
use diffz_core::{palette::Mode, provider::OpenRequest, review::*};
use gpui_kit::component::{
    Disableable, Sizable, StyledExt,
    button::*,
    h_flex,
    input::{Input, Textarea},
};
use gpui_kit::{prelude::*, *};
use std::path::PathBuf;
impl Workbench {
    /// Open the selected source from either Enter or the panel button.
    pub(crate) fn submit_open(&mut self, cx: &mut Context<Self>) {
        let text = self.open_input.read(cx).value().trim().to_string();
        if self.loading || text.is_empty() {
            return;
        }
        let request = match self.source_mode {
            SourceMode::GitHub => OpenRequest::GitHub(text),
            SourceMode::GitLab => OpenRequest::GitLab(text),
            SourceMode::Patch => OpenRequest::Patch(PathBuf::from(text)),
            SourceMode::Compare => OpenRequest::LocalGit {
                root: PathBuf::from(text),
                base: self.base_input.read(cx).value().to_string(),
                head: self.head_input.read(cx).value().to_string(),
            },
            SourceMode::Staged => OpenRequest::LocalIndex(PathBuf::from(text)),
            SourceMode::Worktree => OpenRequest::LocalWorktree(PathBuf::from(text)),
        };
        self.open(request, false, cx);
    }
    pub(crate) fn modal_backdrop(&self, cx: &mut Context<Self>) -> Stateful<Div> {
        div()
            .id("modal-backdrop")
            .absolute()
            .inset_0()
            .occlude()
            .bg(self.skin().base.opacity(0.72))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|a, _, w, c| {
                    a.command(Command::Cancel, w, c);
                    c.stop_propagation();
                }),
            )
    }

    pub fn panel_view(&self, cx: &mut Context<Self>) -> AnyElement {
        let skin = self.skin();
        let title = match self.panel {
            Panel::Open => "Open source",
            Panel::Recent => "Recent reviews",
            Panel::Themes => "Themes",
            Panel::Palette => "Commands",
            Panel::Preview => "Review preview",
            Panel::Export => "Export context",
            Panel::Outbox => "Review outbox",
            Panel::None => "",
            Panel::Line => "Line comment",
        };
        let mut card = div()
            .id("panel-body")
            .occlude()
            .on_mouse_down(MouseButton::Left, |_, _, c| c.stop_propagation())
            .v_flex()
            .w_full()
            .max_w(px(if self.panel == Panel::Palette {
                580.
            } else {
                680.
            }))
            .track_focus(&self.panel_focus)
            .tab_group()
            .max_h_full()
            .overflow_y_scroll()
            .p_4()
            .gap_2()
            .bg(skin.surface)
            .border_1()
            .border_color(skin.border)
            .rounded_lg()
            .child(
                div()
                    .h_flex()
                    .gap_2()
                    .child(div().flex_1().text_size(px(20.)).child(title))
                    .child(
                        Button::new("close-panel")
                            .cursor_pointer()
                            .ghost()
                            .small()
                            .label("Esc")
                            .on_click(cx.listener(|a, _, w, c| a.command(Command::Cancel, w, c))),
                    ),
            );
        match self.panel {
            Panel::Recent => {
                card = card.child(
                    div()
                        .h_flex()
                        .gap_2()
                        .child(
                            Button::new("recent-open")
                                .cursor_pointer()
                                .small()
                                .label("Open another review")
                                .tooltip(crate::commands::tip("Open source", Command::Open))
                                .on_click(cx.listener(|a, _, w, c| a.command(Command::Open, w, c))),
                        )
                        .child(
                            Button::new("clear-recents")
                                .cursor_pointer()
                                .ghost()
                                .small()
                                .label("Clear recent history")
                                .disabled(self.recent.is_empty())
                                .on_click(cx.listener(|a, _, _, c| a.hide_recent(None, c))),
                        ),
                );
                card = card.child(
                    div()
                        .text_size(px(12.))
                        .text_color(skin.muted)
                        .child("Saved reviews and drafts remain after recent history is removed."),
                );
                if self.recent.is_empty() {
                    card = card.child(
                        "There are no recent reviews. Open a PR, merge request, or local diff first.",
                    );
                }
                for (ix, recent) in self.recent.iter().enumerate() {
                    let id = recent.id.clone();
                    let remove = id.clone();
                    let active = self.active.as_ref().is_some_and(|a| a.snapshot.id == id);
                    card = card.child(
                        div()
                            .h_flex()
                            .gap_1()
                            .child(
                                Button::new(("switch-recent", ix))
                                    .ghost()
                                    .small()
                                    .flex_1()
                                    .min_w_0()
                                    .justify_start()
                                    .tooltip(recent.title.clone())
                                    .accessibility_label(recent.title.clone())
                                    .child(
                                        div().flex_1().min_w_0().text_left().text_ellipsis().child(
                                            format!(
                                                "{}{}",
                                                if active { "● " } else { "" },
                                                recent.title
                                            ),
                                        ),
                                    )
                                    .on_click(cx.listener(move |a, _, w, c| {
                                        a.diff_focus.focus(w, c);
                                        a.open(OpenRequest::Resume(id.clone()), false, c)
                                    })),
                            )
                            .child(
                                Button::new(("remove-recent", ix))
                                    .ghost()
                                    .small()
                                    .label("×")
                                    .tooltip("Remove from recent history")
                                    .accessibility_label(format!(
                                        "Remove {} from history",
                                        recent.title
                                    ))
                                    .on_click(cx.listener(move |a, _, _, c| {
                                        a.hide_recent(Some(remove.clone()), c)
                                    })),
                            ),
                    );
                }
            }
            Panel::Themes => {
                let swatch = |c: diffz_core::palette::Rgb| {
                    div()
                        .size(px(10.))
                        .rounded_sm()
                        .bg(rgb(((c.0 as u32) << 16) | ((c.1 as u32) << 8) | c.2 as u32))
                };
                for (dark, label) in [(true, "Built-in dark"), (false, "Built-in light")] {
                    let active = self.settings.theme.is_none() && self.dark == dark;
                    let highlighted = self.theme_index == dark as usize;
                    card = card.child(
                        Button::new(("builtin-theme", dark as usize))
                            .cursor_pointer()
                            .ghost()
                            .small()
                            .flex_1()
                            .min_w_0()
                            .justify_start()
                            .when(active, |b| {
                                b.bg(skin.accent.opacity(0.14)).text_color(skin.accent)
                            })
                            .when(highlighted, |b| {
                                b.border_1().border_color(skin.accent.opacity(0.55))
                            })
                            .child(div().flex_1().text_left().child(label))
                            .on_click(cx.listener(move |a, _, w, c| {
                                a.set_builtin(dark, w, c);
                                a.command(Command::Cancel, w, c);
                            })),
                    );
                }
                if self.themes.is_empty() {
                    card = card.child(
                        div()
                            .text_color(skin.muted)
                            .child("No Omarchy themes here. Drop a colors.toml theme into one of the theme directories."),
                    );
                }
                for (ix, entry) in self.themes.iter().enumerate() {
                    let reference = entry.reference.clone();
                    let name = entry.name.clone();
                    let active = self.settings.theme.as_deref() == Some(reference.as_str());
                    let highlighted = self.theme_index == ix + 2;
                    let mut row = Button::new(("theme", ix))
                        .cursor_pointer()
                        .ghost()
                        .small()
                        .flex_1()
                        .min_w_0()
                        .justify_start()
                        .when(active, |b| {
                            b.bg(skin.accent.opacity(0.14)).text_color(skin.accent)
                        })
                        .when(highlighted, |b| {
                            b.border_1().border_color(skin.accent.opacity(0.55))
                        });
                    if let Some(p) = &entry.palette {
                        row = row.child(
                            h_flex()
                                .gap_2()
                                .child(swatch(p.background))
                                .child(swatch(p.accent))
                                .child(swatch(p.green))
                                .child(swatch(p.red)),
                        );
                    }
                    let tag = match &entry.palette {
                        Some(p) if p.mode == Mode::Dark => "dark",
                        Some(_) => "light",
                        None => "unreadable",
                    };
                    row = row
                        .child(div().flex_1().min_w_0().text_ellipsis().child(name))
                        .child(div().text_size(px(11.)).text_color(skin.muted).child(tag))
                        .on_click(cx.listener(move |a, _, w, c| {
                            a.apply_theme(Some(&reference), w, c);
                            a.command(Command::Cancel, w, c);
                        }));
                    card = card.child(row);
                }
            }
            Panel::Open => {
                let mut modes = div().h_flex().gap_2();
                for (ix, (mode, label)) in [
                    (SourceMode::GitHub, "GitHub PR"),
                    (SourceMode::GitLab, "GitLab MR"),
                    (SourceMode::Patch, "Patch file"),
                    (SourceMode::Compare, "Branches"),
                    (SourceMode::Staged, "Staged"),
                    (SourceMode::Worktree, "Working tree"),
                ]
                .into_iter()
                .enumerate()
                {
                    modes = modes.child(
                        Button::new(("mode", ix))
                            .cursor_pointer()
                            .ghost()
                            .small()
                            .when(self.source_mode == mode, |b| {
                                b.bg(skin.accent.opacity(0.15)).text_color(skin.accent)
                            })
                            .label(label)
                            .on_click(cx.listener(move |a, _, w, c| {
                                a.source_mode = mode;
                                let placeholder = match mode {
                                    SourceMode::GitLab => {
                                        "Enter group/project!123 or a GitLab merge request URL"
                                    }
                                    SourceMode::GitHub => {
                                        "Enter owner/repo#123 or a GitHub pull request URL"
                                    }
                                    SourceMode::Patch => "/path/to/change.patch",
                                    _ => "/path/to/repository",
                                };
                                a.open_input
                                    .update(c, |input, c| input.set_placeholder(placeholder, w, c));
                                a.open_input.read(c).focus_handle(c).focus(w, c);
                                c.notify();
                            })),
                    );
                }
                let (label, help) = match self.source_mode {
                    SourceMode::GitLab => (
                        "Merge request",
                        "Uses glab with GitLab.com or a self-managed HTTPS server.",
                    ),
                    SourceMode::GitHub => (
                        "Pull request",
                        "Enter a PR link or use owner/repository#number.",
                    ),
                    SourceMode::Patch => {
                        ("Patch file", "Open a unified diff stored on this computer.")
                    }
                    SourceMode::Compare => (
                        "Repository",
                        "Compare two revisions from a local repository.",
                    ),
                    SourceMode::Staged => (
                        "Repository",
                        "Review changes currently staged for the next commit.",
                    ),
                    SourceMode::Worktree => (
                        "Repository",
                        "Review unstaged changes together with new files.",
                    ),
                };
                card = card
                    .child(modes)
                    .child(div().mt_2().text_size(px(12.)).child(label))
                    .child(Input::new(&self.open_input))
                    .child(div().text_size(px(11.)).text_color(skin.muted).child(help));
                if self.source_mode == SourceMode::Compare {
                    card = card.child(
                        div()
                            .h_flex()
                            .gap_2()
                            .child("Base")
                            .child(Input::new(&self.base_input))
                            .child("Head")
                            .child(Input::new(&self.head_input)),
                    );
                }
                card = card.child(
                    Button::new("do-open")
                        .cursor_pointer()
                        .primary()
                        .label(if self.loading {
                            "Opening…"
                        } else {
                            "Open review"
                        })
                        .disabled(
                            self.loading || self.open_input.read(cx).value().trim().is_empty(),
                        )
                        .on_click(cx.listener(|a, _, _, cx| a.submit_open(cx))),
                );
                card = card.child(
                    div()
                        .mt_3()
                        .text_size(px(11.))
                        .text_color(skin.muted)
                        .child("Try a sample"),
                );
                let mut demos = div().h_flex().gap_2();
                for (id, label) in [
                    ("F01", "Markdown"),
                    ("F02", "Long URL"),
                    ("F03", "Unicode"),
                    ("F07", "Split diff"),
                    ("F18", "Review flow"),
                ] {
                    demos = demos.child(
                        Button::new(format!("fixture-{id}"))
                            .cursor_pointer()
                            .ghost()
                            .small()
                            .label(label)
                            .on_click(cx.listener(move |a, _, w, c| {
                                a.diff_focus.focus(w, c);
                                a.open(OpenRequest::Fixture(id.into()), false, c)
                            })),
                    );
                }
                card = card.child(demos);
                card = card.child(
                    div()
                        .text_color(skin.muted)
                        .child("Recent reviews · stored on this device"),
                );
                for (ix, recent) in self.recent.iter().take(6).enumerate() {
                    let id = recent.id.clone();
                    card = card.child(
                        Button::new(("recent", ix))
                            .cursor_pointer()
                            .ghost()
                            .small()
                            .w_full()
                            .justify_start()
                            .tooltip(recent.title.clone())
                            .accessibility_label(recent.title.clone())
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .text_ellipsis()
                                    .child(recent.title.clone()),
                            )
                            .on_click(cx.listener(move |a, _, w, c| {
                                a.diff_focus.focus(w, c);
                                a.open(OpenRequest::Resume(id.clone()), false, c)
                            })),
                    );
                }
            }
            Panel::Palette => {
                card = card.child(Input::new(&self.palette_input));
                let matches = self.palette_matches(cx);
                let mut choices = div()
                    .id("palette-choices")
                    .v_flex()
                    .gap_1()
                    .max_h(px(320.))
                    .overflow_y_scroll()
                    .track_scroll(&self.palette_scroll);
                for (position, &ix) in matches.iter().enumerate() {
                    let (command, label, key) = COMMANDS[ix];
                    choices = choices.child(
                        Button::new(("command", ix))
                            .cursor_pointer()
                            .accessibility_label(label)
                            .flex_shrink_0()
                            .ghost()
                            .small()
                            .w_full()
                            .justify_start()
                            .when(position == self.palette_index, |b| {
                                b.bg(skin.accent.opacity(0.14)).text_color(skin.accent)
                            })
                            .child(div().flex_1().child(label))
                            .child(
                                div()
                                    .text_size(px(11.))
                                    .text_color(skin.muted)
                                    .child(crate::commands::display_key(key)),
                            )
                            .on_click(cx.listener(move |a, _, w, c| {
                                a.return_focus
                                    .take()
                                    .unwrap_or_else(|| a.diff_focus.clone())
                                    .focus(w, c);
                                a.panel = Panel::None;
                                a.command(command, w, c);
                            })),
                    );
                }
                if matches.is_empty() {
                    choices = choices.child(
                        div()
                            .p_4()
                            .text_color(skin.muted)
                            .child("No matching commands"),
                    );
                }
                card = card.child(choices).child(
                    div()
                        .h_flex()
                        .pt_2()
                        .border_t_1()
                        .border_color(skin.border)
                        .text_size(px(11.))
                        .text_color(skin.muted)
                        .child("↑ ↓ navigate    ↵ run    Esc close")
                        .child(div().flex_1())
                        .child(format!("{} commands", matches.len())),
                );
            }
            Panel::Preview => {
                let gitlab = self
                    .active
                    .as_ref()
                    .and_then(|a| a.snapshot.remote.as_ref())
                    .is_some_and(|t| t.provider == diffz_core::domain::ProviderKind::GitLab);
                if gitlab {
                    card=card.child("GitLab accepts comments and approval. Choose one source line for each inline draft.");
                }
                card = card.child(
                    div()
                        .text_color(skin.muted)
                        .text_size(px(12.))
                        .child("Write a review summary. Inspect all comments before you publish."),
                );
                let pending = self
                    .active
                    .as_ref()
                    .map_or(0, |a| a.drafts.iter().filter(|d| !d.published).count());
                let mut verdicts = div().h_flex().gap_2();
                for (ix, (v, name)) in [
                    (Verdict::Comment, "Comment"),
                    (Verdict::Approve, "Approve"),
                    (Verdict::RequestChanges, "Request changes"),
                ]
                .into_iter()
                .enumerate()
                {
                    verdicts = verdicts.child(
                        Button::new(("verdict", ix))
                            .cursor_pointer()
                            .label(format!(
                                "{} {name}",
                                if self.verdict == v { "●" } else { "○" }
                            ))
                            .disabled(self.busy || (gitlab && v == Verdict::RequestChanges))
                            .on_click(cx.listener(move |a, _, _, c| {
                                a.verdict = v;
                                a.prepared = None;
                                if let Some(active) = &mut a.active {
                                    active.view.review_verdict = Some(v);
                                }
                                a.schedule_view_save(c);
                                c.notify();
                            })),
                    );
                }
                card = card
                    .child(verdicts)
                    .child(
                        Textarea::new(&self.summary_input)
                            .h(px(130.))
                            .readonly(self.busy)
                            .aria_label("Review summary"),
                    )
                    .child(
                        div()
                            .h_flex()
                            .gap_2()
                            .child(
                                div()
                                    .flex_1()
                                    .text_size(px(11.))
                                    .text_color(skin.muted)
                                    .child(match pending {
                                        0 => "Nothing local yet; only the summary forms the review"
                                            .to_owned(),
                                        1 => "1 local comment is part of the review".to_owned(),
                                        n => format!("{n} local comments join the review"),
                                    }),
                            )
                            .child(
                                Button::new("freeze")
                                    .primary()
                                    .small()
                                    .cursor_pointer()
                                    .label("Continue to review")
                                    .disabled(self.busy)
                                    .on_click(cx.listener(|a, _, _, c| a.prepare(c))),
                            ),
                    );
                if let Some(p) = &self.prepared {
                    card=card.child(div().p_3().bg(skin.raised).child(format!("{} / {} · Review #{}\nAccount: {} @ {}\nReviewed commit: {}\nVerdict: {:?}\nFingerprint: {}",p.target.repository.owner,p.target.repository.name,p.target.pr,p.target.account,p.target.repository.host,p.target.head,p.verdict,p.fingerprint)))
                        .child(div().whitespace_normal().child(format!("Summary: {}",p.summary)));
                    for (ix, c) in p.comments.iter().enumerate() {
                        card = card.child(
                            div()
                                .v_flex()
                                .p_3()
                                .gap_2()
                                .bg(skin.raised)
                                .child(format!(
                                    "{} · {} · {}:{}–{}",
                                    ix + 1,
                                    c.path,
                                    c.side.api(),
                                    c.start_line,
                                    c.line
                                ))
                                .child(div().whitespace_normal().child(c.body.clone())),
                        );
                    }
                    card = card.child(
                        Button::new("publish-exact")
                            .cursor_pointer()
                            .primary()
                            .label(
                                if p.target.provider == diffz_core::domain::ProviderKind::GitLab {
                                    "Send this review unchanged to GitLab"
                                } else {
                                    "Send this review unchanged to GitHub"
                                },
                            )
                            .disabled(
                                self.busy || !self.services.writes_enabled_for(p.target.provider),
                            )
                            .on_click(cx.listener(|a, _, _, c| a.publish(c))),
                    );
                    if !self.services.writes_enabled_for(p.target.provider) {
                        card=card.child(if p.target.provider==diffz_core::domain::ProviderKind::GitLab {"GitLab publication is off. Relaunch with --allow-gitlab-writes to confirm and publish."}else{"GitHub publication is off. Relaunch with --allow-github-writes to confirm and publish."});
                    }
                }
            }
            Panel::Export => {
                card=card.child("Export includes only the chosen source range and local notes. Check for secrets. Nothing goes to a model or provider.");
                if let Some(c) = &self.export_context {
                    card = card
                        .child(div().text_size(px(12.)).child(format!(
                            "{} · {}:{}–{} · {} source bytes · {} notes",
                            c.title,
                            c.selection.start.side.api(),
                            c.selection.start.line,
                            c.selection.end.line,
                            c.text.len(),
                            c.notes.len()
                        )))
                        .child(
                            div()
                                .p_3()
                                .bg(skin.raised)
                                .whitespace_normal()
                                .child(c.text.clone()),
                        );
                    for note in &c.notes {
                        card = card.child(
                            div()
                                .p_2()
                                .whitespace_normal()
                                .child(format!("Note: {note}")),
                        );
                    }
                }
                card = card.child(Input::new(&self.export_input)).child(
                    Button::new("export-file")
                        .cursor_pointer()
                        .primary()
                        .label("Create new private JSON file")
                        .on_click(cx.listener(|a, _, _, c| a.export(c))),
                );
            }
            Panel::Outbox => {
                card=card.child("Unknown outcomes need reconciliation before any retry. Reconciliation reads only the original provider. Zero or multiple matches leave the result unknown.");
                for (ix, e) in self.outbox.iter().enumerate() {
                    let mut row = div()
                        .v_flex()
                        .p_3()
                        .gap_2()
                        .bg(skin.raised)
                        .child(format!(
                            "{:?} · {}/{}#{} · commit {}",
                            e.state,
                            e.prepared.target.repository.owner,
                            e.prepared.target.repository.name,
                            e.prepared.target.pr,
                            e.prepared.target.head
                        ))
                        .child(format!(
                            "Operation {} · {} comments",
                            e.prepared.id.0,
                            e.prepared.comments.len()
                        ));
                    if let Some(d) = &e.diagnostic {
                        row = row.child(div().whitespace_normal().child(d.clone()));
                    }
                    if e.state == OutboxState::UnknownOutcome {
                        let id = e.prepared.id.clone();
                        row = row.child(
                            Button::new(("reconcile", ix))
                                .cursor_pointer()
                                .label("Reconcile — read only")
                                .disabled(self.busy)
                                .on_click(
                                    cx.listener(move |a, _, _, c| a.reconcile(id.clone(), c)),
                                ),
                        );
                    }
                    card = card.child(row);
                }
                if self.outbox.is_empty() {
                    card = card.child("No prepared or submitted reviews.");
                }
            }
            Panel::None | Panel::Line => {}
        }
        if panel_status(&self.status) {
            card = card.child(
                div()
                    .mt_2()
                    .text_size(px(11.))
                    .text_color(skin.muted)
                    .whitespace_normal()
                    .child(self.status.clone()),
            );
        }
        self.modal_backdrop(cx)
            .flex()
            .items_start()
            .when(self.panel != Panel::Recent, |d| d.justify_center())
            .p_6()
            .pt(px(72.))
            .child(card)
            .into_any_element()
    }
}
/// Decide whether a status message belongs in a panel; navigation updates from
/// file changes and edge scrolling remain in the main view.
fn panel_status(status: &str) -> bool {
    !(status.is_empty()
        || status.starts_with("Snapshot loaded.")
        || status.starts_with("Start of file")
        || status.starts_with("End of file")
        || status.starts_with("End of the changed-file list")
        || status.starts_with("File "))
}
#[cfg(test)]
mod tests {
    #[test]
    fn navigation_statuses_stay_out_of_panels() {
        assert!(!super::panel_status(
            "End of file · scroll again for the next file"
        ));
        assert!(!super::panel_status("File 3 of 42"));
        assert!(super::panel_status("Could not save settings: disk full"));
    }
}
