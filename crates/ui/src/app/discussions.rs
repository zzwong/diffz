//! Session-only discussion summaries. No snapshot mutation or persisted history.
use super::*;
use diffz_core::discussion::DiscussionList;
use gpui_kit::component::{Disableable, Sizable, StyledExt, button::*};

#[derive(Default)]
pub(crate) struct DiscussionRead {
    generation: u64,
    cancel: Cancellation,
    pub loading: bool,
    pub result: Option<Result<DiscussionList, String>>,
    page: usize,
}
impl Drop for DiscussionRead {
    fn drop(&mut self) {
        self.cancel.cancel();
    }
}
impl DiscussionRead {
    pub fn cancel_pending(&mut self) {
        if self.loading {
            self.cancel.cancel();
            self.generation = self.generation.wrapping_add(1);
            self.loading = false;
        }
    }
    pub fn reset(&mut self) {
        self.cancel.cancel();
        self.generation = self.generation.wrapping_add(1);
        self.cancel = Cancellation::default();
        self.loading = false;
        self.result = None;
        self.page = 0;
    }
    fn begin(&mut self) -> (u64, Cancellation) {
        self.reset();
        self.loading = true;
        (self.generation, self.cancel.clone())
    }
    fn complete(&mut self, generation: u64, result: Result<DiscussionList, String>) -> bool {
        if generation != self.generation || self.cancel.cancelled() || !self.loading {
            return false;
        }
        self.loading = false;
        self.result = Some(result);
        true
    }
}
impl Workbench {
    pub(crate) fn load_discussions(&mut self, cx: &mut Context<Self>) {
        if self.loading || self.discussions.loading {
            return;
        }
        let Some(target) = self.active.as_ref().and_then(|a| a.snapshot.remote.clone()) else {
            return;
        };
        if target.provider != ProviderId::ENTIRE {
            return;
        }
        let (generation, cancel) = self.discussions.begin();
        let services = self.services.clone();
        cx.spawn(async move |this, cx| {
            let result = cx
                .background_spawn(async move {
                    services
                        .discussions(&target, cancel)
                        .map_err(|e| e.to_string())
                })
                .await;
            let _ = this.update(cx, |app, cx| {
                if app.discussions.complete(generation, result) {
                    cx.notify();
                }
            });
        })
        .detach();
        cx.notify();
    }
    pub(crate) fn discussion_summaries(&self, cx: &mut Context<Self>) -> AnyElement {
        let skin = self.skin();
        let read = &self.discussions;
        let mut pane = div().v_flex().gap_2().child("Remote discussion summaries · session only")
            .child(div().text_size(px(11.)).text_color(skin.muted).whitespace_normal()
                .child("Read-only summaries for this pinned Change. Access filtering applies; this is not a complete project history. Bodies and replies are not loaded. Findings have no verified inline anchors here."))
            .child(Button::new("load-discussion-summaries").small()
                .label(if read.loading { "Loading…" } else if read.result.is_some() { "Reload summaries" } else { "Load summaries" })
                .disabled(self.loading || read.loading)
                .on_click(cx.listener(|app, _, _, cx| app.load_discussions(cx))));
        match &read.result {
            None if !read.loading => {
                pane = pane.child("Not loaded. Saved comments below are separate.");
            }
            None => {}
            Some(Err(error)) => {
                pane = pane.child(div().text_color(skin.warning).whitespace_normal().child(
                    format!("Unavailable: {error}. This is not an empty history."),
                ));
            }
            Some(Ok(list)) => {
                pane = pane.child(div().text_size(px(11.)).whitespace_normal().child(if list.has_more {
                    "Partial: the reader limit was reached. Additional discussions may exist; reloading is not continuation."
                } else { "Loaded accessible summaries; access filtering still applies. Reload to check for changes." }));
                if list.items.is_empty() {
                    pane =
                        pane.child("No accessible discussion summaries returned for this Change.");
                }
                let pages = list.items.len().div_ceil(50).max(1);
                let page = read.page.min(pages - 1);
                if pages > 1 {
                    pane = pane.child(
                        div()
                            .h_flex()
                            .gap_2()
                            .child(
                                Button::new("discussion-prev")
                                    .small()
                                    .label("Previous")
                                    .disabled(page == 0)
                                    .on_click(cx.listener(|a, _, _, c| {
                                        a.discussions.page = a.discussions.page.saturating_sub(1);
                                        c.notify();
                                    })),
                            )
                            .child(format!("Page {} / {} of loaded summaries", page + 1, pages))
                            .child(
                                Button::new("discussion-next")
                                    .small()
                                    .label("Next")
                                    .disabled(page + 1 >= pages)
                                    .on_click(cx.listener(move |a, _, _, c| {
                                        a.discussions.page =
                                            (a.discussions.page + 1).min(pages - 1);
                                        c.notify();
                                    })),
                            ),
                    );
                }
                for summary in list.items.iter().skip(page * 50).take(50) {
                    pane = pane.child(
                        div()
                            .v_flex()
                            .gap_1()
                            .border_b_1()
                            .border_color(skin.border)
                            .pb_2()
                            .child(div().whitespace_normal().child(summary.title.clone()))
                            .child(
                                div()
                                    .text_size(px(11.))
                                    .text_color(skin.muted)
                                    .whitespace_normal()
                                    .child(format!(
                                        "{} · {} · {} messages · updated {}",
                                        summary.kind,
                                        if summary.resolved {
                                            "resolved"
                                        } else {
                                            "unresolved"
                                        },
                                        summary.message_count,
                                        summary.updated_at
                                    )),
                            )
                            .child(div().text_size(px(11.)).whitespace_normal().child(format!(
                                "ID {} · author {}",
                                summary.id,
                                summary.created_by.as_deref().unwrap_or("unknown")
                            ))),
                    );
                }
            }
        }
        pane.into_any_element()
    }
}

#[cfg(test)]
mod tests {
    use super::{DiscussionList, DiscussionRead};
    fn empty() -> DiscussionList {
        DiscussionList {
            items: vec![],
            has_more: false,
            event_cursors: vec![],
        }
    }
    #[test]
    fn replacement_and_same_head_refresh_reject_old_completion() {
        let mut read = DiscussionRead::default();
        let (old, token) = read.begin();
        read.reset(); // open/install invalidation, independent of snapshot ID
        assert!(token.cancelled());
        let (current, _) = read.begin();
        assert!(!read.complete(old, Ok(empty())));
        assert!(read.loading);
        assert!(read.complete(current, Ok(empty())));
        assert!(!read.complete(current, Err("duplicate".into())));
        assert!(matches!(read.result, Some(Ok(_))));
    }
    #[test]
    fn leaving_comments_cancels_pending_but_preserves_loaded_result() {
        let mut read = DiscussionRead::default();
        let (id, token) = read.begin();
        read.cancel_pending();
        assert!(token.cancelled());
        assert!(!read.complete(id, Ok(empty())));
        assert!(!read.loading && read.result.is_none());
        let (id, _) = read.begin();
        assert!(read.complete(id, Ok(empty())));
        read.cancel_pending();
        assert!(matches!(read.result, Some(Ok(_))));
    }
    #[test]
    fn dropping_session_cancels_inflight_read() {
        let mut read = DiscussionRead::default();
        let (_, token) = read.begin();
        drop(read);
        assert!(token.cancelled());
    }
    #[test]
    fn errors_and_partial_success_remain_distinct_from_empty() {
        let mut read = DiscussionRead::default();
        let (id, _) = read.begin();
        assert!(read.complete(id, Err("denied".into())));
        assert!(matches!(read.result, Some(Err(_))));
        let (id, _) = read.begin();
        assert!(read.result.is_none());
        let mut partial = empty();
        partial.has_more = true;
        assert!(read.complete(id, Ok(partial)));
        assert!(read.result.as_ref().unwrap().as_ref().unwrap().has_more);
        read.reset();
        assert!(!read.loading && read.result.is_none());
    }
}
