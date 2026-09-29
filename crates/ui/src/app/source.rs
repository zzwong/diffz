use super::*;

impl Workbench {
    pub fn open(&mut self, request: OpenRequest, refresh: bool, cx: &mut Context<Self>) {
        self.open_pending(request, refresh, None, cx);
    }
    /// Opens a request from a later `diffz` invocation the way the Open panel does. Without one,
    /// the window only comes forward.
    pub fn take_handoff(
        &mut self,
        request: Option<OpenRequest>,
        cx: &mut Context<Self>,
    ) -> Result<(), String> {
        let Some(request) = request else {
            return Ok(());
        };
        if let Some(blocker) = self.handoff_blocker(cx) {
            return Err(blocker.into());
        }
        self.open(request, false, cx);
        Ok(())
    }
    /// Why opening another review now would lose work, if it would. Drafts are saved as they are
    /// typed, but a reply or a comment on a line that takes none lives only in the composer.
    pub(crate) fn handoff_blocker(&self, cx: &App) -> Option<&'static str> {
        let draft = self.selected_draft.as_ref().and_then(|id| {
            let drafts = &self.active.as_ref()?.drafts;
            drafts.iter().find(|d| &d.id == id)
        });
        handoff_blocker(
            self.unsaved() || self.busy,
            self.panel == Panel::Line
                && composing(
                    &self.composer.draft_input.read(cx).value(),
                    draft.map(|d| d.body.as_str()),
                ),
            self.panel == Panel::Preview && self.prepared.is_some(),
        )
    }
    /// Follows the Open panel's text: a recognized source selects its tab and is named beside the
    /// field. Text that is not recognized returns to the tab as it was chosen by hand.
    pub(crate) fn detect_source_mode(
        &mut self,
        text: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let found = self.services.detect(text);
        let mode = mode_after_detection(&self.chosen_mode, found.as_ref().map(|f| &f.request));
        if mode != self.source_mode {
            let hint = self.mode_hint(&mode);
            self.open_input
                .update(cx, |input, cx| input.set_placeholder(hint, window, cx));
            self.source_mode = mode;
        }
        self.detected = found.map(|f| f.label);
        cx.notify();
    }
    fn mode_hint(&self, mode: &SourceMode) -> String {
        match mode {
            SourceMode::Remote(provider) => self
                .services
                .provider(provider)
                .map_or(String::new(), |p| p.address_hint().to_string()),
            SourceMode::Patch => "/path/to/change.patch".into(),
            _ => "/path/to/repository".into(),
        }
    }
    /// Puts `text` in the Open field, named and selected as a detected source would be, and waits
    /// for Enter with `note` beside it.
    pub(crate) fn prefill_open(
        &mut self,
        text: String,
        note: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.panel = Panel::Open;
        self.open_input
            .update(cx, |input, cx| input.set_value(text.clone(), window, cx));
        self.detect_source_mode(&text, window, cx);
        self.open_input.read(cx).focus_handle(cx).focus(window, cx);
        self.status = note;
        cx.notify();
    }
    pub(super) fn open_pending(
        &mut self,
        request: OpenRequest,
        refresh: bool,
        pending: Option<(Cancellation, Task<Result<Opened, ServiceError>>)>,
        cx: &mut Context<Self>,
    ) {
        if self.unsaved() || self.busy {
            self.status = "Save your outstanding changes before the source switches.".into();
            cx.notify();
            return;
        }
        self.open_cancel.cancel();
        let (cancel, pending) = match pending {
            Some((cancel, task)) => (cancel, Some(task)),
            None => (Cancellation::default(), None),
        };
        self.open_cancel = cancel;
        self.open_generation += 1;
        let generation = self.open_generation;
        let cancel = self.open_cancel.clone();
        let services = self.services.clone();
        let job = request.clone();
        self.loading = true;
        self.status = "Loading source; your snapshot and position carry over.".into();
        cx.spawn(async move|this,cx|{
            let result = match pending {
                Some(task) => task.await,
                None => cx.background_spawn(async move { services.open(job, cancel) }).await,
            };
            diffz_core::timing::mark("snapshot opened");
            let _=this.update(cx,|app,cx|{if app.open_generation!=generation{return}app.loading=false;match result{
                Ok(opened)=>{if refresh&&app.active.as_ref().is_some_and(|a|a.snapshot.id!=opened.snapshot.id){app.offered=Some(opened);app.status="New revision available. The snapshot on screen has not changed.".into();}
                    else if refresh{let snapshot=Arc::new(opened.snapshot);if let Some(a)=&mut app.active{a.snapshot=snapshot.clone();}
if let Some(v)=&app.viewport{v.borrow_mut().snapshot=snapshot;}app.status="Source unchanged; comment list and review state refreshed without moving the view.".into();}
                    else{app.last_request=Some(request);app.install(opened,cx);}},Err(e)=>app.status=e.message,
            }cx.notify();});
        }).detach();
        cx.notify();
    }
    pub fn install(&mut self, opened: Opened, cx: &mut Context<Self>) {
        let ack = opened.view.revision;
        let selected = opened
            .view
            .selected_file
            .clone()
            .filter(|f| opened.snapshot.file(f).is_some())
            .or_else(|| opened.snapshot.patch.files.first().map(|f| f.id.clone()));
        self.verdict = opened.view.review_verdict.unwrap_or(Verdict::Comment);
        self.active = Some(Active {
            snapshot: Arc::new(opened.snapshot),
            drafts: opened.drafts,
            view: opened.view,
            view_ack: ack,
        });
        self.viewport = None;
        self.selected_draft = None;
        self.line_context = None;
        self.thread_root = None;
        self.comment_page = 0;
        self.prepared = None;
        self.offered = None;
        self.panel = Panel::None;
        self.search_hits.clear();
        self.drag_start = None;
        self.annotations = Arc::default();
        self.filter_files(cx);
        if let Some(id) = selected {
            self.select_file(id, cx)
        }
        self.annotate(cx);
        self.status =
            "Snapshot loaded. The source holds still until another revision is accepted on purpose."
                .into();
        self.refresh_recent(cx);
        self.refresh_outbox(cx);
    }
    pub fn filter_files(&mut self, cx: &mut Context<Self>) {
        let query = self.filter_input.read(cx).value().to_lowercase();
        let entries = self.active.as_ref().map_or_else(Vec::new, |a| {
            a.snapshot
                .patch
                .files
                .iter()
                .map(|f| (f.id.clone(), f.display_path()))
                .collect()
        });
        self.browser.rebuild(entries, &query);
    }
    pub fn remember_anchor(&mut self) {
        if let (Some(a), Some(v)) = (&mut self.active, &self.viewport) {
            let v = v.borrow();
            if let Some(anchor) = &v.anchor {
                a.view.anchors.insert(v.file.0.clone(), anchor.clone());
            }
        }
    }
    pub fn select_file(&mut self, id: FileId, cx: &mut Context<Self>) {
        self.gesture.reset_boundary();
        self.cancel_scroll_gesture();
        self.remember_anchor();
        let Some(a) = &mut self.active else { return };
        let Some(file) = a.snapshot.file(&id) else {
            return;
        };
        let wrap = self
            .settings
            .wrap
            .unwrap_or_else(|| presentation::default_wrap(&file.display_path()));
        let anchor = a.view.anchors.get(&id.0).cloned();
        a.view.selected_file = Some(id.clone());
        self.viewport = Some(Rc::new(RefCell::new(Viewport::new(
            a.snapshot.clone(),
            id.clone(),
            self.settings.split,
            wrap,
            self.settings.font_size,
            self.font_family.clone(),
            anchor,
        ))));
        self.drag_start = None;
        self.mark_annotations();
        self.schedule_view_save(cx);
        self.highlight(id, cx);
        cx.notify();
    }
    fn annotate(&mut self, cx: &mut Context<Self>) {
        self.annotate_cancel.cancel();
        self.annotate_cancel = Cancellation::default();
        let cancel = self.annotate_cancel.clone();
        let Some(a) = &self.active else { return };
        let snapshot = a.snapshot.clone();
        let registry = self.registry.clone();
        cx.spawn(async move |this, cx| {
            let id = snapshot.id.clone();
            let job_cancel = cancel.clone();
            let (found, problems) = cx
                .background_spawn(async move { registry.annotate(&snapshot, &job_cancel) })
                .await;
            let _ = this.update(cx, |app, cx| {
                if cancel.cancelled() || app.active.as_ref().is_none_or(|a| a.snapshot.id != id) {
                    return;
                }
                app.annotations = Arc::new(found);
                app.mark_annotations();
                if !problems.is_empty() {
                    app.status = format!("Annotations: {}", problems.join("; "));
                }
                cx.notify();
            });
        })
        .detach();
    }
    fn mark_annotations(&mut self) {
        let (Some(a), Some(v)) = (&self.active, &self.viewport) else {
            return;
        };
        let mut v = v.borrow_mut();
        let path = a
            .snapshot
            .file(&v.file)
            .map(|f| f.display_path())
            .unwrap_or_default();
        v.annotations.clear();
        for n in self.annotations.iter() {
            if let diffz_core::annotation::Anchor::Lines {
                path: p,
                side,
                start,
                end,
            } = &n.anchor
                && *p == path
            {
                for line in *start..=*end {
                    let slot = v.annotations.entry((*side, line)).or_insert(n.severity);
                    *slot = (*slot).max(n.severity);
                }
            }
        }
    }
}
/// Whether the composer holds text that `saved`, the body of the draft it edits, does not.
fn composing(text: &str, saved: Option<&str>) -> bool {
    !text.trim().is_empty() && saved != Some(text)
}
fn handoff_blocker(saving: bool, composing: bool, previewing: bool) -> Option<&'static str> {
    if composing {
        Some("a comment is being written in diffz; save or discard it first")
    } else if previewing {
        Some("a review is waiting to be published in diffz; publish it or close the preview first")
    } else if saving {
        Some("diffz is still saving or publishing; try again in a moment")
    } else {
        None
    }
}

/// The tab the Open panel shows: the one a detected source belongs to, otherwise the one chosen.
fn mode_after_detection(chosen: &SourceMode, found: Option<&OpenRequest>) -> SourceMode {
    match found {
        Some(OpenRequest::Remote { provider, .. }) => SourceMode::Remote(provider.clone()),
        Some(_) => SourceMode::Patch,
        None => chosen.clone(),
    }
}
#[cfg(test)]
mod tests {
    use super::{SourceMode, composing, handoff_blocker, mode_after_detection};
    use diffz_core::{domain::ProviderId, provider::OpenRequest};

    #[test]
    fn unsaved_comments_block_a_handoff() {
        assert!(composing("a reply", None));
        assert!(composing("edited", Some("draft")));
        assert!(!composing("draft", Some("draft")));
        assert!(!composing(" \n", None));
        let blocker = handoff_blocker(false, true, false).unwrap();
        assert!(blocker.contains("comment is being written"), "{blocker}");
        assert!(handoff_blocker(false, false, true).is_some());
        assert!(handoff_blocker(true, false, false).is_some());
        assert_eq!(handoff_blocker(false, false, false), None);
    }
    #[test]
    fn a_passing_match_does_not_replace_the_chosen_tab() {
        let chosen = SourceMode::Compare;
        let patch = OpenRequest::Patch("/Users/me/notes".into());
        assert_eq!(
            mode_after_detection(&chosen, Some(&patch)),
            SourceMode::Patch
        );
        // Typing on past the match returns to the tab picked by hand.
        assert_eq!(mode_after_detection(&chosen, None), chosen);
        let remote = OpenRequest::Remote {
            provider: ProviderId::GITLAB,
            address: "group/project!7".into(),
        };
        assert_eq!(
            mode_after_detection(&chosen, Some(&remote)),
            SourceMode::Remote(ProviderId::GITLAB)
        );
    }
}
