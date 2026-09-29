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
if let Some(v)=&app.viewport{v.borrow_mut().snapshot=snapshot;}
// The release list is read again, so indexes into the old one no longer hold.
app.release_filter=None;app.release_notes.clear();app.filter_files(cx);app.load_releases(cx);app.mark_releases();app.fetch_blame(cx);
app.status="Source unchanged; comment list and review state refreshed without moving the view.".into();}
                    else{app.last_request=Some(request);app.install(opened,cx);}},Err(e)=>app.status=e.message,
            }cx.notify();});
        }).detach();
        cx.notify();
    }
    pub fn install(&mut self, opened: Opened, cx: &mut Context<Self>) {
        let replacing = self.active.is_some();
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
        self.release_filter = None;
        self.release_notes.clear();
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
        self.load_releases(cx);
        self.start_profile_steps(cx);
        self.status =
            "Snapshot loaded. The source holds still until another revision is accepted on purpose."
                .into();
        self.refresh_recent(cx);
        self.refresh_outbox(cx);
        if replacing {
            self.profile_trim_after_switch();
        }
    }
    pub fn filter_files(&mut self, cx: &mut Context<Self>) {
        let query = self.filter_input.read(cx).value().to_lowercase();
        let old_paths = self.active.as_ref().map_or_else(Default::default, |a| {
            a.snapshot
                .patch
                .files
                .iter()
                .filter_map(|f| Some((f.id.clone(), f.moved_from()?.display())))
                .collect()
        });
        let entries = self.active.as_ref().map_or_else(Vec::new, |a| {
            let touched =
                diffz_core::review_details::releases_by_path(&a.snapshot.overview.releases);
            a.snapshot
                .patch
                .files
                .iter()
                .map(|f| (f.id.clone(), f.display_path()))
                .filter(|(_, path)| {
                    self.release_filter
                        .is_none_or(|i| touched.get(path.as_str()).is_some_and(|t| t.contains(&i)))
                })
                .collect()
        });
        self.browser.rebuild(entries, old_paths, &query);
    }
    /// Narrows the tree to the files release `index` touched, or shows every file again.
    pub fn filter_release(&mut self, index: Option<usize>, cx: &mut Context<Self>) {
        self.release_filter = index;
        self.filter_files(cx);
        self.mark_releases();
        self.fetch_blame(cx);
        let selected = self.viewport.as_ref().map(|v| v.borrow().file.clone());
        if let Some(first) = self.browser.visible_files.first().cloned()
            && selected.is_none_or(|s| !self.browser.visible_files.contains(&s))
        {
            self.select_file(first, cx);
        }
        let Some(a) = &self.active else { return };
        self.status = match index.and_then(|i| a.snapshot.overview.releases.get(i)) {
            Some(r) => format!(
                "Showing the {} changed files {} touched.",
                self.browser.visible_files.len(),
                r.name("the head")
            ),
            None => "Showing every changed file.".into(),
        };
        cx.notify();
    }
    /// Opens release `index` on its own, as a compare from the release before it.
    pub fn open_release_step(&mut self, index: usize, cx: &mut Context<Self>) {
        let Some(a) = &self.active else { return };
        let Some(t) = &a.snapshot.remote else { return };
        let (Some(refs), Some(release)) = (&t.compare, a.snapshot.overview.releases.get(index))
        else {
            return;
        };
        let base = index
            .checked_sub(1)
            .and_then(|i| a.snapshot.overview.releases[i].tag.clone())
            .unwrap_or_else(|| refs.base.clone());
        let step = RemoteTarget {
            compare: Some(CompareRefs {
                base,
                head: release.name(&refs.head).to_owned(),
                direct: false,
            }),
            ..t.clone()
        };
        let Some(provider) = self.services.provider(&t.provider) else {
            return;
        };
        self.open(provider.reopen(&step), false, cx);
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
        self.mark_releases();
        self.fetch_blame(cx);
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
    /// Reads a compare's releases after it shows. Only the snapshot the read began for takes
    /// the result; replacing that snapshot, even with a refresh of the same one, cancels it.
    fn load_releases(&mut self, cx: &mut Context<Self>) {
        if let Some(stale) = self.releases_pending.take() {
            stale.cancel();
        }
        let Some(a) = &self.active else { return };
        let is_compare = a
            .snapshot
            .remote
            .as_ref()
            .is_some_and(|t| t.compare.is_some());
        // A resumed compare keeps the releases it saved.
        if !is_compare || !a.snapshot.overview.releases.is_empty() {
            return;
        }
        let cancel = Cancellation::default();
        self.releases_pending = Some(cancel.clone());
        let snapshot = a.snapshot.clone();
        let services = self.services.clone();
        cx.spawn(async move |this, cx| {
            let id = snapshot.id.clone();
            let job_cancel = cancel.clone();
            let result = cx
                .background_spawn(async move { services.releases(&snapshot, job_cancel) })
                .await;
            let _ = this.update(cx, |app, cx| {
                if cancel.cancelled() || app.active.as_ref().is_none_or(|a| a.snapshot.id != id) {
                    return;
                }
                app.releases_pending = None;
                match result {
                    Ok(snapshot) => {
                        let snapshot = Arc::new(snapshot);
                        if let Some(a) = &mut app.active {
                            a.snapshot = snapshot.clone();
                        }
                        if let Some(v) = &app.viewport {
                            v.borrow_mut().snapshot = snapshot;
                        }
                        // Attribution needs the releases, so it starts once they are here.
                        app.mark_releases();
                        app.fetch_blame(cx);
                    }
                    Err(e) => app.status = e.message,
                }
                cx.notify();
            });
        })
        .detach();
    }
    /// Reads release attribution for the shown file and the few after it in the tree. One read
    /// runs at a time, and its result is shown and saved only if no reload came in between.
    fn fetch_blame(&mut self, cx: &mut Context<Self>) {
        /// The shown file and this many after it are read together.
        const AHEAD: usize = 3;
        let Some(a) = &self.active else { return };
        if self.blame_busy {
            return;
        }
        let shown = self.viewport.as_ref().map(|v| v.borrow().file.clone());
        let files = &self.browser.visible_files;
        let start = shown
            .and_then(|id| files.iter().position(|f| *f == id))
            .unwrap_or(0);
        let paths: Vec<String> = files
            .iter()
            .skip(start)
            .take(AHEAD + 1)
            .filter_map(|id| a.snapshot.file(id))
            .filter(|f| diffz_core::review_details::blame_span(&a.snapshot.overview, f).is_some())
            .map(|f| f.display_path())
            .collect();
        if paths.is_empty() {
            return;
        }
        self.blame_busy = true;
        let from = a.snapshot.clone();
        let services = self.services.clone();
        let cancel = self.open_cancel.clone();
        cx.spawn(async move |this, cx| {
            let read = from.clone();
            let cancelled = cancel.clone();
            let result = cx
                .background_spawn(async move {
                    let found = services.blame(read.clone(), paths, cancel)?;
                    Ok::<_, ServiceError>(Arc::new(diffz_core::review_details::with_blame(
                        &read, found,
                    )))
                })
                .await;
            let _ = this.update(cx, |app, cx| {
                app.blame_busy = false;
                match result {
                    Ok(s)
                        if app.active.as_ref().is_some_and(|a| {
                            diffz_core::review_details::blame_applies(&a.snapshot, &from)
                        }) =>
                    {
                        if let Some(v) = &app.viewport {
                            v.borrow_mut().snapshot = s.clone();
                        }
                        if let Some(a) = &mut app.active {
                            a.snapshot = s.clone();
                        }
                        app.mark_releases();
                        let services = app.services.clone();
                        cx.spawn(async move |this, cx| {
                            let saved = cx
                                .background_spawn(async move { services.save_blame(&s) })
                                .await;
                            if let Err(e) = saved {
                                let _ = this.update(cx, |app, cx| {
                                    app.status = e.message;
                                    cx.notify();
                                });
                            }
                        })
                        .detach();
                    }
                    // A reload replaced the snapshot, or cancelled the read, while it ran.
                    Ok(_) => {}
                    Err(_) if cancelled.cancelled() => {}
                    Err(e) => {
                        app.status = e.message;
                        cx.notify();
                        return;
                    }
                }
                app.fetch_blame(cx);
                cx.notify();
            });
        })
        .detach();
    }
    /// Marks the shown file's changed lines with the release each came from, as far as is known.
    fn mark_releases(&mut self) {
        use diffz_core::{patch::RowKind, review_details::*};
        let (Some(a), Some(v)) = (&self.active, &self.viewport) else {
            return;
        };
        let mut v = v.borrow_mut();
        v.releases.clear();
        let s = &a.snapshot;
        let releases = &s.overview.releases;
        let Some(file) = s.file(&v.file).filter(|_| releases.len() >= 2) else {
            return;
        };
        let head = s
            .remote
            .as_ref()
            .and_then(|t| t.compare.as_ref())
            .map_or("head", |c| c.head.as_str());
        let path = file.display_path();
        let touched = releases_by_path(releases)
            .remove(path.as_str())
            .unwrap_or_default();
        let names: Vec<&str> = touched.iter().map(|&i| releases[i].name(head)).collect();
        let removed = removed_in(&names);
        let focus = self.release_filter;
        for row in file.hunks.iter().flat_map(|h| &h.rows) {
            let (key, mark) = match (row.kind, row.old_line, row.new_line) {
                (RowKind::Added, _, Some(line)) => {
                    let Some(b) = s.overview.row_release(&path, row) else {
                        continue;
                    };
                    (
                        (Side::Right, line),
                        crate::viewport::ReleaseMark {
                            release: Some(b.release),
                            label: format!(
                                "Changed in {} · {}",
                                releases[b.release].name(head),
                                &b.commit[..b.commit.len().min(7)]
                            ),
                            dim: focus.is_some_and(|f| f != b.release),
                        },
                    )
                }
                // Blame at the head cannot see a removal, so only a file one release touched names it.
                (RowKind::Removed, Some(line), _) => (
                    (Side::Left, line),
                    crate::viewport::ReleaseMark {
                        release: (touched.len() == 1).then(|| touched[0]),
                        label: removed.clone(),
                        dim: focus.is_some_and(|f| !touched.contains(&f)),
                    },
                ),
                _ => continue,
            };
            v.releases.insert(key, mark);
        }
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
