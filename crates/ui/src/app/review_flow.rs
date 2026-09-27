use super::*;

impl Workbench {
    pub fn new_draft(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(v) = self.viewport.clone() else {
            self.status = "Open a file before adding a comment.".into();
            return;
        };
        let v = v.borrow();
        let selection = v.draft_selection();
        drop(v);
        let Some(s) = selection else {
            if self.active.is_some() && self.viewport.is_some() {
                self.new_draft_file(window, cx);
            } else {
                self.status = "Open a file before adding a comment.".into();
            }
            return;
        };
        if s.start.side != s.end.side || s.start.file != s.end.file {
            self.status = "Comments are limited to a single side of a single file.".into();
            return;
        }
        let (start_line, line) = (s.start.line.min(s.end.line), s.start.line.max(s.end.line));
        let Some(a) = &mut self.active else { return };
        if !a
            .snapshot
            .file(&s.start.file)
            .is_some_and(|f| f.eligible(s.start.side, start_line, line))
        {
            self.status =
                "The selection reaches into unloaded context or across a hunk boundary.".into();
            return;
        }
        self.open_draft_panel(s, window, cx);
    }
    /// Attach the comment to the entire open file, not to one line.
    pub fn new_draft_file(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(a) = &mut self.active else { return };
        let id = self
            .viewport
            .as_ref()
            .map(|v| v.borrow().file.clone())
            .or_else(|| a.view.selected_file.clone());
        let Some(id) = id else {
            self.status = "Nothing to comment on until a file is open.".into();
            return;
        };
        if a.snapshot.file(&id).is_none() {
            return;
        }
        let snapshot = a.snapshot.id.clone();
        // File-level comments sit on the right at line 0 (see domain::Draft::file_level).
        let zero = SourcePoint {
            snapshot,
            file: id,
            side: Side::Right,
            line: 0,
            byte_column: 0,
        };
        let selection = SourceSelection {
            start: zero.clone(),
            end: zero,
        };
        self.open_draft_panel(selection, window, cx);
    }
    fn open_draft_panel(
        &mut self,
        selection: SourceSelection,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let (start_line, line) = (
            selection.start.line.min(selection.end.line),
            selection.start.line.max(selection.end.line),
        );
        let Some(a) = &mut self.active else { return };
        let existing = a
            .drafts
            .iter()
            .find(|d| {
                d.file == selection.start.file
                    && d.side == selection.start.side
                    && d.start_line == start_line
                    && d.line == line
                    && !d.published
            })
            .cloned();
        self.selected_draft = existing.as_ref().map(|d| d.id.clone());
        self.line_context = Some(selection);
        self.thread_root = None;
        self.return_focus = window.focused(cx);
        self.panel = Panel::Line;
        self.reset_comment_editor();
        self.draft_input.update(cx, |input, cx| {
            input.set_value(existing.map_or_else(String::new, |d| d.body), window, cx)
        });
        let focus = self.draft_input.read(cx).focus_handle(cx);
        focus.focus(window, cx);
        window.defer(cx, move |window, cx| focus.focus(window, cx));
        cx.notify();
    }
    pub fn select_draft(&mut self, id: DraftId, window: &mut Window, cx: &mut Context<Self>) {
        let Some(d) = self
            .active
            .as_ref()
            .and_then(|a| a.drafts.iter().find(|d| d.id == id))
            .cloned()
        else {
            return;
        };
        self.select_file(d.file.clone(), cx);
        let start = SourcePoint {
            snapshot: d.snapshot.clone(),
            file: d.file.clone(),
            side: d.side,
            line: d.start_line,
            byte_column: 0,
        };
        let mut end = start.clone();
        end.line = d.line;
        self.line_context = Some(SourceSelection {
            start,
            end: end.clone(),
        });
        if let Some(v) = &self.viewport {
            v.borrow_mut().reveal(end);
        }
        self.thread_root = None;
        self.selected_draft = Some(id);
        self.return_focus = window.focused(cx);
        self.panel = Panel::Line;
        self.reset_comment_editor();
        self.draft_input
            .update(cx, |input, cx| input.set_value(d.body, window, cx));
        self.draft_input.read(cx).focus_handle(cx).focus(window, cx);
        cx.notify();
    }
    pub(super) fn edit_draft(&mut self, body: String, cx: &mut Context<Self>) {
        if self.selected_draft.is_none()
            && !body.trim().is_empty()
            && let (Some(selection), Some(a)) = (&self.line_context, &mut self.active)
        {
            let (start_line, line) = (
                selection.start.line.min(selection.end.line),
                selection.start.line.max(selection.end.line),
            );
            // Selecting line 0 on the right side means a file-level comment (see domain::Draft::file_level).
            let file_level = selection.start.side == Side::Right && start_line == 0 && line == 0;
            let ok = if file_level {
                a.snapshot.file(&selection.start.file).is_some()
            } else {
                a.snapshot
                    .file(&selection.start.file)
                    .is_some_and(|f| f.eligible(selection.start.side, start_line, line))
            };
            if ok {
                let id = DraftId(self.services.fresh_id());
                a.drafts.push(Draft {
                    id: id.clone(),
                    snapshot: a.snapshot.id.clone(),
                    file: selection.start.file.clone(),
                    side: selection.start.side,
                    start_line,
                    line,
                    file_level,
                    body: String::new(),
                    version: 0,
                    saved_version: 0,
                    published: false,
                });
                self.selected_draft = Some(id);
            }
        }
        let Some(d) = self.active.as_mut().and_then(|a| {
            a.drafts
                .iter_mut()
                .find(|d| Some(&d.id) == self.selected_draft.as_ref())
        }) else {
            return;
        };
        if d.published || body == d.body {
            return;
        }
        if body.len() > 64 * 1024 {
            self.status="Draft is over the 64 KiB limit for publication; it stays editable and is saved locally.".into();
        }
        match diffz_core::session::edit_draft(d, body) {
            Ok(true) => {}
            Ok(false) => return,
            Err(e) => {
                self.status = e;
                return;
            }
        }
        let draft = d.clone();
        self.prepared = None;
        self.save_draft(draft, cx);
        cx.notify();
    }
    pub fn save_draft(&mut self, draft: Draft, cx: &mut Context<Self>) {
        let id = draft.id.clone();
        let key = id.clone();
        let services = self.services.clone();
        let snapshot = draft.snapshot.clone();
        let task = cx.spawn(async move |this, cx| {
            smol::Timer::after(Duration::from_millis(180)).await;
            let result = cx
                .background_spawn(async move { services.save_draft(draft) })
                .await;
            let _ = this.update(cx, |app, cx| {
                if let Some(d) = app
                    .active
                    .as_mut()
                    .filter(|a| a.snapshot.id == snapshot)
                    .and_then(|a| a.drafts.iter_mut().find(|d| d.id == id))
                {
                    match result {
                        Ok(version) => diffz_core::session::acknowledge_save(d, version),
                        Err(e) => {
                            app.status = format!(
                                "Draft NOT saved: {}. The text is still in memory.",
                                e.message
                            )
                        }
                    }
                }
                cx.notify();
            });
        });
        self.save_tasks.insert(key, task);
    }
    pub fn discard_selected(&mut self, cx: &mut Context<Self>) {
        let Some(id) = self.selected_draft.clone() else {
            return;
        };
        if self.discard_candidate.as_ref() != Some(&id) {
            self.discard_candidate = Some(id);
            self.status = "Click Confirm discard; that deletes this local draft.".into();
            cx.notify();
            return;
        }
        let Some(d) = self
            .active
            .as_ref()
            .and_then(|a| a.drafts.iter().find(|d| d.id == id))
            .cloned()
        else {
            return;
        };
        if !d.is_saved() {
            self.status = "Let this draft finish saving before you discard it.".into();
            return;
        }
        let services = self.services.clone();
        self.busy = true;
        let job = id.clone();
        cx.spawn(async move |this, cx| {
            let r = cx
                .background_spawn(async move { services.discard_draft(job, d.version) })
                .await;
            let _ = this.update(cx, |app, cx| {
                app.busy = false;
                match r {
                    Ok(()) => {
                        if let Some(a) = &mut app.active {
                            a.drafts.retain(|d| d.id != id);
                        }
                        app.selected_draft = None;
                        app.discard_candidate = None;
                        app.prepared = None;
                        app.status = "Discarded the local draft; nothing was published.".into();
                    }
                    Err(e) => app.status = e.message,
                }
                cx.notify();
            });
        })
        .detach();
    }
    pub fn retry_saves(&mut self, cx: &mut Context<Self>) {
        let drafts = self.active.as_ref().map_or_else(Vec::new, |a| {
            a.drafts
                .iter()
                .filter(|d| !d.is_saved())
                .cloned()
                .collect::<Vec<_>>()
        });
        for d in drafts {
            self.save_draft(d, cx)
        }
        self.schedule_view_save(cx);
    }
    pub fn prepare(&mut self, cx: &mut Context<Self>) {
        if self.busy {
            return;
        }
        let Some(a) = &self.active else { return };
        let services = self.services.clone();
        let id = a.snapshot.id.clone();
        let drafts = a.drafts.iter().filter(|d| !d.published).cloned().collect();
        let verdict = self.verdict;
        let summary = self.summary_input.read(cx).value().to_string();
        self.prepared = None;
        self.busy = true;
        cx.spawn(async move |this, cx| {
            let result = cx
                .background_spawn(async move { services.prepare(&id, drafts, verdict, summary) })
                .await;
            let _ = this.update(cx, |app, cx| {
                app.busy = false;
                match result {
                    Ok(p) => {
                        app.prepared = Some(p);
                        app.status =
                            "Payload frozen. Review each item; nothing has gone out.".into()
                    }
                    Err(e) => app.status = e.message,
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }
    fn mark_confirmed_drafts_published(&mut self, entry: &OutboxEntry) {
        if entry.state == OutboxState::Confirmed
            && let Some(a) = self
                .active
                .as_mut()
                .filter(|a| a.snapshot.id == entry.prepared.snapshot)
        {
            for c in &entry.prepared.comments {
                if let Some(d) = a
                    .drafts
                    .iter_mut()
                    .find(|d| d.id == c.draft && d.version == c.version)
                {
                    d.published = true;
                }
            }
        }
    }
    pub fn publish(&mut self, cx: &mut Context<Self>) {
        if self.busy {
            return;
        }
        let Some(p) = self.prepared.clone() else {
            return;
        };
        let services = self.services.clone();
        self.busy = true;
        cx.spawn(async move |this, cx| {
            let result = cx
                .background_spawn(async move { services.publish(p) })
                .await;
            let _ = this.update(cx, |app, cx| {
                app.busy = false;
                match result {
                    Ok(entry) => {
                        app.status = format!(
                            "Review outcome: {:?}. {}",
                            entry.state,
                            entry.diagnostic.clone().unwrap_or_default()
                        );
                        app.mark_confirmed_drafts_published(&entry);
                        app.prepared = None;
                        app.panel = Panel::Outbox;
                    }
                    Err(e) => app.status = e.message,
                }
                app.refresh_outbox(cx);
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }
    pub fn reconcile(&mut self, id: OperationId, cx: &mut Context<Self>) {
        if self.busy {
            return;
        }
        self.busy = true;
        let services = self.services.clone();
        cx.spawn(async move |this, cx| {
            let r = cx
                .background_spawn(async move { services.reconcile(id) })
                .await;
            let _ = this.update(cx, |app, cx| {
                app.busy = false;
                match r {
                    Ok(entry) => {
                        app.status = format!(
                            "Reconciliation: {:?}. No resend was attempted.",
                            entry.state
                        );
                        app.mark_confirmed_drafts_published(&entry);
                    }
                    Err(e) => app.status = e.message,
                }
                app.refresh_outbox(cx);
                cx.notify();
            });
        })
        .detach();
    }
}
