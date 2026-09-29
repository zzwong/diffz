use super::*;

impl Workbench {
    pub fn command(&mut self, command: Command, window: &mut Window, cx: &mut Context<Self>) {
        if matches!(
            command,
            Command::Open | Command::Palette | Command::Preview | Command::Export
        ) || (command == Command::Keys && self.panel == Panel::None)
        {
            self.return_focus = window.focused(cx);
        }
        match command {
            Command::Recent => self.show_recents(window, cx),
            Command::Themes => self.show_themes(window, cx),
            Command::Open => {
                self.panel = Panel::Open;
                self.open_input.read(cx).focus_handle(cx).focus(window, cx);
            }
            Command::PasteOpen => {
                let text = cx
                    .read_from_clipboard()
                    .and_then(|item| item.text())
                    .map(|text| text.trim().to_string());
                match text.and_then(|text| Some((self.services.detect(&text)?, text))) {
                    // Clipboard text is not the user's own request: a patch path or an address
                    // on a host they have not signed in to waits for Enter.
                    Some((found, text)) => match found.request {
                        request @ OpenRequest::Remote { .. } => {
                            let services = self.services.clone();
                            let job = request.clone();
                            cx.spawn_in(window, async move |this, cx| {
                                let host = cx
                                    .background_spawn(async move { services.unconfirmed_host(&job) })
                                    .await;
                                let _ = this.update_in(cx, |this, window, cx| match host {
                                    Some(host) => {
                                        let note = format!("Press Enter to open {host}");
                                        this.prefill_open(text, note, window, cx)
                                    }
                                    None => {
                                        if let Err(refused) = this.take_handoff(Some(request), cx) {
                                            this.status = refused;
                                        }
                                        cx.notify();
                                    }
                                });
                            })
                            .detach();
                        }
                        _ => self.prefill_open(text, "Press Enter to open this patch file".into(), window, cx),
                    },
                    None => {
                        self.status = "The clipboard holds no pull request, merge request, compare address, or patch file.".into()
                    }
                }
                cx.notify();
            }
            Command::Palette => {
                self.panel = Panel::Palette;
                self.palette_index = 0;
                self.palette_scroll.scroll_to_item(0);
                self.palette_input
                    .read(cx)
                    .focus_handle(cx)
                    .focus(window, cx);
            }
            Command::Find => {
                self.find_visible = true;
                self.find_input.read(cx).focus_handle(cx).focus(window, cx);
            }
            Command::Files => {
                self.files_visible = !self.files_visible;
                self.reset_files_peek();
                if !self.files_visible {
                    self.diff_focus.focus(window, cx);
                }
                self.schedule_view_save(cx);
            }
            Command::Inspector => {
                self.inspector_visible = !self.inspector_visible;
                if self.inspector_visible {
                    self.overview_focus.focus(window, cx);
                }
                if !self.inspector_visible {
                    self.diff_focus.focus(window, cx);
                }
                self.schedule_view_save(cx);
            }
            Command::Wrap | Command::Split | Command::ZoomIn | Command::ZoomOut => {
                if let Some(view) = &self.viewport {
                    let mut v = view.borrow_mut();
                    let mut split = v.split;
                    let mut wrap = v.wrap;
                    let mut font = v.font_size;
                    match command {
                        Command::Wrap => wrap = !wrap,
                        Command::Split => split = !split,
                        Command::ZoomIn => font += 1.0,
                        Command::ZoomOut => font -= 1.0,
                        _ => {}
                    }
                    v.configure(split, wrap, font);
                    self.settings.split = split;
                    self.settings.font_size = v.font_size;
                    if matches!(command, Command::Wrap) {
                        self.settings.wrap = Some(wrap);
                    }
                }
                self.save_settings(cx);
            }
            Command::Rich => {
                self.settings.rich = !self.settings.rich;
                self.save_settings(cx);
            }
            Command::RichInline => {
                self.settings.rich_inline = !self.settings.rich_inline;
                self.save_settings(cx);
            }
            Command::Theme => {
                if self.theme.is_some() {
                    // Clearing the selected theme keeps the palette's light or dark choice.
                    self.apply_theme(None, window, cx);
                    self.status = "Theme removed; built-in colours are back".into();
                } else {
                    self.dark = !self.dark;
                    self.settings.dark = self.dark;
                    crate::app::apply_appearance(self.dark, None, Some(window), cx);
                    self.save_settings(cx);
                }
            }
            Command::Copy => {
                if let (Some(a), Some(v)) = (&self.active, &self.viewport)
                    && let Some(s) = &v.borrow().selection
                {
                    match a.snapshot.copy_selection(s) {
                        Ok(text) => {
                            cx.write_to_clipboard(ClipboardItem::new_string(text));
                            self.status =
                                "Copied the original source bytes without wrap breaks or gutter text."
                                    .into();
                        }
                        Err(e) => self.status = e,
                    }
                }
            }
            Command::Comment => self.new_draft(window, cx),
            Command::CommentFile => self.new_draft_file(window, cx),
            Command::Preview => {
                self.panel = Panel::Preview;
                self.summary_input
                    .read(cx)
                    .focus_handle(cx)
                    .focus(window, cx);
                let summary = self
                    .active
                    .as_ref()
                    .map_or_else(String::new, |a| a.view.review_summary.clone());
                self.summary_input
                    .update(cx, |s, cx| s.set_value(summary, window, cx));
            }
            Command::Refresh => {
                // A refresh of a review resumed from Recents must query the provider,
                // rather than reuse the local store.
                let remote = self
                    .active
                    .as_ref()
                    .and_then(|a| a.snapshot.remote.as_ref())
                    .and_then(|t| Some(self.services.provider(&t.provider)?.reopen(t)));
                let request = match (self.last_request.clone(), remote) {
                    (Some(OpenRequest::Resume(_)) | None, Some(remote)) => Some(remote),
                    (Some(request), _) => Some(request),
                    (None, None) => None,
                };
                if let Some(request) = request {
                    self.open(request, true, cx)
                } else {
                    self.status = "Nothing can refresh until a source is open.".into();
                }
            }
            Command::NextHunk | Command::PreviousHunk => {
                let forward = command == Command::NextHunk;
                let target = self
                    .viewport
                    .as_ref()
                    .and_then(|v| v.borrow_mut().next_hunk(forward));
                self.status = if let Some((index, total)) = target {
                    format!("Hunk {index} of {total}")
                } else if forward {
                    "This file has no later hunk. Press ] to move to the next file.".into()
                } else {
                    "This file has no earlier hunk. Press [ to move to the previous file.".into()
                };
                self.diff_focus.focus(window, cx);
            }
            Command::NextFile | Command::PreviousFile => {
                self.step_file(command == Command::NextFile, cx);
            }
            Command::Keys => {
                if self.panel == Panel::Keys {
                    self.command(Command::Cancel, window, cx);
                } else {
                    self.panel = Panel::Keys;
                    self.panel_focus.focus(window, cx);
                }
            }
            Command::Export => self.begin_export(cx),
            Command::Cancel => {
                if self.panel != Panel::None {
                    self.panel = Panel::None;
                    self.line_context = None;
                    self.selected_draft = None;
                    self.thread_root = None;
                } else if self.inspector_visible
                    && (self.overview_hovered || self.overview_focus.contains_focused(window, cx))
                {
                    self.inspector_visible = false;
                    self.overview_hovered = false;
                } else if self.find_visible {
                    self.find_visible = false;
                    if let Some(v) = &self.viewport {
                        v.borrow_mut().active_search = None;
                    }
                } else if let Some(v) = &self.viewport {
                    v.borrow_mut().selection = None;
                }
                self.return_focus
                    .take()
                    .unwrap_or_else(|| self.diff_focus.clone())
                    .focus(window, cx);
            }
        }
        cx.notify();
    }
    pub fn navigate_hit(&mut self, forward: bool, cx: &mut Context<Self>) {
        if self.search_hits.is_empty() {
            self.status = "The loaded patch context contains no matches.".into();
            cx.notify();
            return;
        }
        if self.search_index >= self.search_hits.len() {
            self.search_index = if forward {
                0
            } else {
                self.search_hits.len() - 1
            };
        } else if forward {
            self.search_index = (self.search_index + 1) % self.search_hits.len();
        } else {
            self.search_index =
                (self.search_index + self.search_hits.len() - 1) % self.search_hits.len();
        }
        let h = self.search_hits[self.search_index].clone();
        self.select_file(h.file.clone(), cx);
        if let (Some(a), Some(v)) = (&self.active, &self.viewport) {
            let p = SourcePoint {
                snapshot: a.snapshot.id.clone(),
                file: h.file,
                side: h.side,
                line: h.line,
                byte_column: h.bytes.start,
            };
            let q = SourcePoint {
                byte_column: h.bytes.end,
                ..p.clone()
            };
            let mut v = v.borrow_mut();
            v.reveal_range(p.clone(), h.bytes.end);
            v.selection = Some(SourceSelection { start: p, end: q });
            v.active_search = v.selection.clone();
        }
        self.schedule_view_save(cx);
        cx.notify();
    }
    pub fn accept_offer(&mut self, cx: &mut Context<Self>) {
        if self.unsaved() {
            self.status = "Save your drafts before changing revisions.".into();
            cx.notify();
            return;
        }
        if let Some(opened) = self.offered.take() {
            self.install(opened, cx);
            self.status="The new revision is active. Older drafts stay in their session; comments were not remapped.".into();
        }
    }
}
