//! GPUI Kit supplies the standard controls; review state stays outside them.
use crate::{
    app::{Panel, SourceMode, Workbench},
    commands::{COMMANDS, Command, SHEET, SheetGroup, SheetKeys},
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
        let request = match self.source_mode.clone() {
            SourceMode::Remote(provider) => OpenRequest::Remote {
                provider,
                address: text,
            },
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

    pub fn panel_view(&self, window: &Window, cx: &mut Context<Self>) -> AnyElement {
        let skin = self.skin();
        let title = match self.panel {
            Panel::Keys => "Keyboard shortcuts",
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
                    let highlighted = self.theme_index == usize::from(!dark);
                    card = card.child(
                        Button::new(("builtin-theme", dark as usize))
                            .accessibility_label(if active {
                                format!("{label}, current theme")
                            } else {
                                label.to_string()
                            })
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
                            .child("No themes here. Drop a folder holding theme.toml or colors.toml into one of the theme directories."),
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
                    if let Some(t) = &entry.theme {
                        row = row.child(
                            h_flex()
                                .gap_2()
                                .child(swatch(t.skin.base))
                                .child(swatch(t.skin.accent))
                                .child(swatch(t.skin.positive))
                                .child(swatch(t.skin.negative)),
                        );
                    }
                    let tag = match &entry.theme {
                        Some(t) if t.mode == Mode::Dark => "dark",
                        Some(_) => "light",
                        None => "unreadable",
                    };
                    row = row
                        .accessibility_label(format!(
                            "{name}, {tag}{}",
                            if active { ", current theme" } else { "" }
                        ))
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
                let providers = self.services.providers();
                let remote_modes = providers.iter().map(|p| {
                    (
                        SourceMode::Remote(p.id()),
                        p.open_label().to_string(),
                        p.address_hint().to_string(),
                    )
                });
                let local_modes = [
                    (SourceMode::Patch, "Patch file", "/path/to/change.patch"),
                    (SourceMode::Compare, "Branches", "/path/to/repository"),
                    (SourceMode::Staged, "Staged", "/path/to/repository"),
                    (SourceMode::Worktree, "Working tree", "/path/to/repository"),
                ]
                .map(|(mode, label, hint)| (mode, label.to_string(), hint.to_string()));
                for (ix, (mode, label, placeholder)) in remote_modes.chain(local_modes).enumerate()
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
                                a.source_mode = mode.clone();
                                a.open_input.update(c, |input, c| {
                                    input.set_placeholder(placeholder.clone(), w, c)
                                });
                                a.open_input.read(c).focus_handle(c).focus(w, c);
                                c.notify();
                            })),
                    );
                }
                let (label, help) = match &self.source_mode {
                    SourceMode::Remote(id) => providers
                        .iter()
                        .find(|p| p.id() == *id)
                        .map_or((String::new(), String::new()), |p| {
                            (p.address_label().to_string(), p.address_help().to_string())
                        }),
                    SourceMode::Patch => (
                        "Patch file".into(),
                        "Open a unified diff stored on this computer.".into(),
                    ),
                    SourceMode::Compare => (
                        "Repository".into(),
                        "Compare two revisions from a local repository.".into(),
                    ),
                    SourceMode::Staged => (
                        "Repository".into(),
                        "Review changes currently staged for the next commit.".into(),
                    ),
                    SourceMode::Worktree => (
                        "Repository".into(),
                        "Review unstaged changes together with new files.".into(),
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
                let provider = self
                    .active
                    .as_ref()
                    .and_then(|a| a.snapshot.remote.as_ref())
                    .and_then(|t| self.services.provider(&t.provider));
                if let Some(note) = provider
                    .as_ref()
                    .and_then(|p| p.preview_note().map(str::to_owned))
                {
                    card = card.child(note);
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
                            .disabled(
                                self.busy || provider.as_ref().is_some_and(|p| !p.supports(v)),
                            )
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
                    let provider = self.services.provider(&p.target.provider);
                    let host = provider
                        .as_ref()
                        .map_or_else(|| p.target.provider.to_string(), |r| r.name().to_string());
                    card = card.child(
                        Button::new("publish-exact")
                            .cursor_pointer()
                            .primary()
                            .label(format!("Send this review unchanged to {host}"))
                            .disabled(
                                self.busy || !self.services.writes_enabled_for(&p.target.provider),
                            )
                            .on_click(cx.listener(|a, _, _, c| a.publish(c))),
                    );
                    if !self.services.writes_enabled_for(&p.target.provider) {
                        card = card.child(match &provider {
                            Some(r) => format!(
                                "{host} publication is off. Relaunch with {} to confirm and publish.",
                                r.write_flag()
                            ),
                            None => format!("No provider named {host} is available to publish."),
                        });
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
            Panel::Keys => {
                let single = f32::from(window.viewport_size().width) < 620.;
                let split = if single {
                    SHEET.len()
                } else {
                    sheet_split(SHEET)
                };
                let mut columns = div().h_flex().items_start().gap_6().w_full();
                let mut next_row = 0usize;
                for groups in [&SHEET[..split], &SHEET[split..]] {
                    if groups.is_empty() {
                        continue;
                    }
                    let mut column = div().v_flex().flex_1().min_w_0().gap_3();
                    for group in groups {
                        let mut section = div().v_flex().gap_1().child(
                            div()
                                .text_size(px(11.))
                                .text_color(skin.muted)
                                .child(group.name),
                        );
                        for row in group.rows {
                            let keys = crate::commands::sheet_keys(row);
                            let caps =
                                div()
                                    .h_flex()
                                    .gap_1()
                                    .flex_shrink_0()
                                    .children(keys.iter().map(|key| {
                                        crate::chrome::keycap(skin.border, key.clone())
                                    }));
                            let ix = next_row;
                            next_row += 1;
                            section = section.child(match row.keys {
                                SheetKeys::Run(commands) => {
                                    let command = commands[0];
                                    Button::new(("sheet-row", ix))
                                        .cursor_pointer()
                                        .accessibility_label(format!(
                                            "{} ({})",
                                            row.label,
                                            keys.join(" ")
                                        ))
                                        .ghost()
                                        .small()
                                        .w_full()
                                        .justify_start()
                                        .child(
                                            div()
                                                .flex_1()
                                                .min_w_0()
                                                .text_left()
                                                .text_ellipsis()
                                                .text_size(px(12.))
                                                .child(row.label),
                                        )
                                        .child(caps)
                                        .on_click(cx.listener(move |a, _, w, c| {
                                            a.return_focus
                                                .take()
                                                .unwrap_or_else(|| a.diff_focus.clone())
                                                .focus(w, c);
                                            a.panel = Panel::None;
                                            a.command(command, w, c);
                                        }))
                                        .into_any_element()
                                }
                                SheetKeys::Fixed(_) => div()
                                    .h_flex()
                                    .gap_1()
                                    .px_2()
                                    .py(px(3.))
                                    .child(
                                        div()
                                            .flex_1()
                                            .min_w_0()
                                            .text_ellipsis()
                                            .text_size(px(12.))
                                            .child(row.label),
                                    )
                                    .child(caps)
                                    .into_any_element(),
                            });
                        }
                        column = column.child(section);
                    }
                    columns = columns.child(column);
                }
                card = card.child(columns);
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
/// Split the sheet's groups between two columns of about the same height, in
/// the order they are written.
fn sheet_split(groups: &[SheetGroup]) -> usize {
    let total: usize = groups.iter().map(|g| g.rows.len()).sum();
    let mut before = 0;
    let mut best = (usize::MAX, groups.len());
    for (ix, group) in groups.iter().enumerate() {
        before += group.rows.len();
        let gap = before.abs_diff(total - before);
        if gap < best.0 {
            best = (gap, ix + 1);
        }
    }
    best.1
}
/// Decide whether a status message belongs in a panel; navigation updates from
/// file changes and edge scrolling remain in the main view.
fn panel_status(status: &str) -> bool {
    !(status.is_empty()
        || status.starts_with("Snapshot loaded.")
        || status.starts_with("Start of file")
        || status.starts_with("End of file")
        || status.starts_with("Keep pulling")
        || status.starts_with("End of the changed-file list")
        || status.starts_with("File "))
}
#[cfg(test)]
mod tests {
    #[test]
    fn navigation_statuses_stay_out_of_panels() {
        assert!(!super::panel_status(
            "End of file · scroll again to pull the next file in"
        ));
        assert!(!super::panel_status("Keep pulling for the next file"));
        assert!(!super::panel_status("File 3 of 42"));
        assert!(super::panel_status("Could not save settings: disk full"));
    }
    #[test]
    fn the_sheet_columns_come_out_about_even() {
        let split = super::sheet_split(crate::commands::SHEET);
        let rows = |groups: &[crate::commands::SheetGroup]| -> usize {
            groups.iter().map(|g| g.rows.len()).sum()
        };
        let left = rows(&crate::commands::SHEET[..split]);
        let right = rows(&crate::commands::SHEET[split..]);
        assert!(split > 0 && split < crate::commands::SHEET.len());
        assert!(left.abs_diff(right) * 3 <= left + right, "{left} / {right}");
    }
}
