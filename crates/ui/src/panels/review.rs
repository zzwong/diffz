use crate::{app::Workbench, theme::Skin};
use diffz_core::review::{OutboxState, Verdict};
use gpui_kit::component::{
    Disableable, Sizable, StyledExt,
    button::*,
    input::{Input, Textarea},
};
use gpui_kit::{prelude::*, *};

impl Workbench {
    pub(super) fn preview_panel(
        &self,
        mut card: Stateful<Div>,
        skin: Skin,
        cx: &mut Context<Self>,
    ) -> Stateful<Div> {
        if self
            .active
            .as_ref()
            .and_then(|a| a.snapshot.remote.as_ref())
            .is_some_and(|t| t.compare.is_some())
        {
            return card.child(
                "This is a compare, which is read-only: reviews, comments, and approvals cannot be sent to the provider. Local drafts stay on this machine and can be exported.",
            );
        }
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
                    .disabled(self.busy || provider.as_ref().is_some_and(|p| !p.supports(v)))
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
            card = card
                .child(div().p_3().bg(skin.raised).child(format!("{} / {} · Review #{}\nAccount: {} @ {}\nReviewed commit: {}\nVerdict: {:?}\nFingerprint: {}",p.target.repository.owner,p.target.repository.name,p.target.pr,p.target.account,p.target.repository.host,p.target.head,p.verdict,p.fingerprint)))
                .child(div().whitespace_normal().child(format!("Summary: {}", p.summary)));
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
                    .disabled(self.busy || !self.services.writes_enabled_for(&p.target.provider))
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
        card
    }

    pub(super) fn export_panel(
        &self,
        mut card: Stateful<Div>,
        skin: Skin,
        cx: &mut Context<Self>,
    ) -> Stateful<Div> {
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
        card
    }

    pub(super) fn outbox_panel(
        &self,
        mut card: Stateful<Div>,
        skin: Skin,
        cx: &mut Context<Self>,
    ) -> Stateful<Div> {
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
                        .on_click(cx.listener(move |a, _, _, c| a.reconcile(id.clone(), c))),
                );
            }
            card = card.child(row);
        }
        if self.outbox.is_empty() {
            card = card.child("No prepared or submitted reviews.");
        }
        card
    }
}
