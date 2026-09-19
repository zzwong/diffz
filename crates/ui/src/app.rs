use crate::{
    commands,
    theme::Skin,
    viewport::{Decorations, Viewport},
};
use diffz_core::{
    domain::*,
    export::ContextExport,
    palette::{Mode, Palette, Rgb},
    presentation,
    provider::*,
    review::*,
};
use gpui_kit::component::{
    Root, Theme, ThemeConfig, ThemeConfigColors, ThemeMode, ThemeRegistry,
    input::{InputEvent, InputState, TextareaState},
};
use gpui_kit::{prelude::*, *};
use std::{
    cell::RefCell,
    collections::HashMap,
    fs,
    path::{Path, PathBuf},
    rc::Rc,
    sync::Arc,
    time::{Duration, Instant, SystemTime},
};

pub struct LaunchOptions {
    pub initial: OpenRequest,
    pub font_family: Option<String>,
    pub theme: Option<String>,
}
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum Panel {
    None,
    Open,
    Palette,
    Preview,
    Export,
    Outbox,
    Line,
    Recent,
    Themes,
}
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum SourceMode {
    GitHub,
    GitLab,
    Patch,
    Compare,
    Staged,
    Worktree,
}
pub(crate) struct Active {
    pub snapshot: Arc<Snapshot>,
    pub drafts: Vec<Draft>,
    pub view: SavedView,
    pub view_ack: u64,
}
pub(crate) struct ThemeEntry {
    pub name: String,
    pub reference: String,
    pub palette: Option<Palette>,
}
pub(crate) struct PanelResizeState {
    pub(crate) left: bool,
    pub(crate) start_x: f32,
    pub(crate) start_width: f32,
    pub(crate) last_x: f32,
    pub(crate) last_sample: Instant,
    pub(crate) velocity_x: f32,
}
pub(crate) struct Workbench {
    pub services: Arc<dyn WorkbenchServices>,
    pub active: Option<Active>,
    pub viewport: Option<Rc<RefCell<Viewport>>>,
    pub open_input: Entity<InputState>,
    pub base_input: Entity<InputState>,
    pub head_input: Entity<InputState>,
    pub filter_input: Entity<InputState>,
    pub find_input: Entity<InputState>,
    pub palette_input: Entity<InputState>,
    pub draft_input: Entity<TextareaState>,
    pub summary_input: Entity<TextareaState>,
    pub export_input: Entity<InputState>,
    pub diff_focus: FocusHandle,
    pub root_focus: FocusHandle,
    pub panel: Panel,
    pub source_mode: SourceMode,
    pub files_visible: bool,
    pub inspector_visible: bool,
    pub files_resize_focus: FocusHandle,
    pub overview_resize_focus: FocusHandle,
    pub files_width: f32,
    pub overview_width: f32,
    pub resizing_panel: Option<PanelResizeState>,
    pub find_visible: bool,
    pub find_next_focus: FocusHandle,
    pub dark: bool,
    pub settings: Settings,
    /// Rendered items for the prose file that is open, keyed by snapshot, file, and word-mark mode.
    pub rich_cache: Option<crate::rich_view::RichCache>,
    pub font_family: String,
    pub status: String,
    pub palette: Option<Palette>,
    pub theme_path: Option<PathBuf>,
    pub theme_mtime: Option<SystemTime>,
    pub themes: Vec<ThemeEntry>,
    pub theme_task: Option<Task<()>>,
    pub loading: bool,
    pub busy: bool,
    pub prepared: Option<PreparedReview>,
    pub verdict: Verdict,
    pub selected_draft: Option<DraftId>,
    pub line_context: Option<SourceSelection>,
    pub thread_root: Option<u64>,
    pub thread_scroll: ScrollHandle,
    pub context_busy: bool,
    pub overview_hovered: bool,
    pub overview_focus: FocusHandle,
    pub overview_tab: usize,
    pub comment_page: usize,
    pub boundary_scroll: diffz_core::scroll::BoundaryScroll,
    pub last_wheel: Option<std::time::Instant>,
    /// Inertia after a touchpad gesture; see `scrolling.rs`.
    pub kinetic: diffz_core::scroll::Kinetic,
    /// A finger is on the pad (or a wheel gesture is in progress).
    pub gesture_active: bool,
    /// Decides that the finger lifted when the platform never says so.
    pub gesture_task: Option<Task<()>>,
    /// A coast is being advanced frame by frame.
    pub coast_ticking: bool,
    /// Zero point for the millisecond clock the scroll state machines use.
    pub gesture_epoch: std::time::Instant,
    pub export_context: Option<ContextExport>,
    pub outbox: Vec<OutboxEntry>,
    pub recent: Vec<RecentSession>,
    pub offered: Option<Opened>,
    pub last_request: Option<OpenRequest>,
    pub search_hits: Vec<presentation::SearchHit>,
    pub search_index: usize,
    pub drag_start: Option<SourcePoint>,
    pub scrollbar_drag: bool,
    pub horizontal_drag: bool,
    pub discard_candidate: Option<DraftId>,
    pub open_generation: u64,
    pub open_cancel: Cancellation,
    highlight_cancel: Arc<std::sync::atomic::AtomicUsize>,
    pub save_tasks: HashMap<DraftId, Task<()>>,
    pub view_task: Option<Task<()>>,
    pub tree_rows: Arc<Vec<diffz_core::file_tree::TreeRow>>,
    pub collapsed_dirs: std::collections::HashSet<String>,
    pub tree_cursor: usize,
    pub tree_focus: FocusHandle,
    pub panel_focus: FocusHandle,
    pub return_focus: Option<FocusHandle>,
    pub scrollbar_hide_task: Option<Task<()>>,
    pub palette_index: usize,
    pub theme_index: usize,
    pub palette_scroll: ScrollHandle,
    pub file_list: ListState,
    pub visible_files: Vec<FileId>,
    _subscriptions: Vec<Subscription>,
}
impl Workbench {
    pub fn new(
        services: Arc<dyn WorkbenchServices>,
        font_family: Option<String>,
        theme: Option<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let mut settings = services.settings().unwrap_or_default();
        let theme_changed = theme.is_some() && theme != settings.theme;
        if let Some(theme) = theme {
            settings.theme = Some(theme);
        }
        apply_appearance(settings.dark, None, Some(window), cx);
        let open_input =
            cx.new(|cx| InputState::new(window, cx).placeholder("GitHub PR URL or owner/repo#123"));
        let base_input = cx.new(|cx| InputState::new(window, cx).default_value("main"));
        let head_input = cx.new(|cx| InputState::new(window, cx).default_value("HEAD"));
        let filter_input =
            cx.new(|cx| InputState::new(window, cx).placeholder("Filter changed files"));
        let find_input = cx.new(|cx| {
            InputState::new(window, cx).placeholder("Case-sensitive search of the loaded diff")
        });
        let palette_input = cx.new(|cx| InputState::new(window, cx).placeholder("Filter commands"));
        let draft_input = cx.new(|cx| {
            TextareaState::new(window, cx)
                .rows(4)
                .placeholder("Leave a comment…")
        });
        let summary_input = cx.new(|cx| {
            TextareaState::new(window, cx)
                .rows(4)
                .placeholder("Review summary")
        });
        let export_input = cx.new(|cx| {
            InputState::new(window, cx).placeholder("New .json file, as an absolute path")
        });
        let mut subscriptions = vec![];
        subscriptions.push(cx.subscribe(
            &palette_input,
            |this: &mut Self, _, event: &InputEvent, cx| {
                if matches!(event, InputEvent::Change) {
                    this.palette_index = 0;
                    this.palette_scroll.scroll_to_item(0);
                    cx.notify();
                }
            },
        ));
        subscriptions.push(cx.subscribe(
            &draft_input,
            |this: &mut Self, state, event: &InputEvent, cx| {
                if matches!(event, InputEvent::Change) {
                    this.edit_draft(state.read(cx).value().to_string(), cx);
                }
            },
        ));
        subscriptions.push(cx.subscribe(
            &filter_input,
            |this: &mut Self, _, event: &InputEvent, cx| {
                if matches!(event, InputEvent::Change) {
                    this.filter_files(cx);
                    cx.notify();
                }
            },
        ));
        subscriptions.push(cx.subscribe(
            &find_input,
            |this: &mut Self, _, event: &InputEvent, cx| {
                if matches!(event, InputEvent::Change) {
                    this.search_hits = this.active.as_ref().map_or_else(Vec::new, |a| {
                        presentation::find(&a.snapshot, &this.find_input.read(cx).value(), 500)
                    });
                    this.search_index = this.search_hits.len();
                    if let Some(v) = &this.viewport {
                        v.borrow_mut().active_search = None;
                    }
                    cx.notify();
                }
            },
        ));
        subscriptions.push(cx.subscribe(
            &summary_input,
            |this: &mut Self, state, event: &InputEvent, cx| {
                if matches!(event, InputEvent::Change) {
                    let value = state.read(cx).value().to_string();
                    if let Some(a) = &mut this.active
                        && a.view.review_summary != value
                    {
                        a.view.review_summary = value;
                        this.prepared = None;
                        this.schedule_view_save(cx);
                    }
                    cx.notify();
                }
            },
        ));
        subscriptions.push(cx.observe_window_bounds(window, |_, _, cx| cx.notify()));
        let weak = cx.entity().downgrade();
        window.on_window_should_close(cx,move |_,cx|weak.update(cx,|app,cx|{
            if app.unsaved()||app.busy{app.status="Close blocked: saves/publication still running. Failed drafts are kept in memory; export before closing.".into();cx.notify();false}else{true}
        }).unwrap_or(true));
        let mut this = Self {
            services,
            active: None,
            viewport: None,
            open_input,
            base_input,
            head_input,
            filter_input,
            find_input,
            palette_input,
            draft_input,
            summary_input,
            export_input,
            diff_focus: cx.focus_handle().tab_stop(true),
            root_focus: cx.focus_handle(),
            panel: Panel::None,
            source_mode: SourceMode::GitHub,
            files_visible: true,
            inspector_visible: false,
            files_resize_focus: cx.focus_handle(),
            overview_resize_focus: cx.focus_handle(),
            files_width: 282.,
            overview_width: 400.,
            resizing_panel: None,
            find_visible: false,
            find_next_focus: cx.focus_handle().tab_stop(true),
            dark: settings.dark,
            settings,
            rich_cache: None,
            font_family: font_family.unwrap_or_else(|| {
                if cfg!(target_os = "macos") {
                    "Menlo".into()
                } else {
                    "DejaVu Sans Mono".into()
                }
            }),
            status: "Open a GitHub PR, GitLab MR, fixture, patch, or local Git comparison.".into(),
            palette: None,
            theme_path: None,
            theme_mtime: None,
            themes: vec![],
            theme_task: None,
            loading: false,
            busy: false,
            prepared: None,
            verdict: Verdict::Comment,
            selected_draft: None,
            line_context: None,
            thread_root: None,
            thread_scroll: ScrollHandle::new(),
            context_busy: false,
            overview_hovered: false,
            overview_focus: cx.focus_handle(),
            overview_tab: 0,
            comment_page: 0,
            boundary_scroll: Default::default(),
            last_wheel: None,
            kinetic: Default::default(),
            gesture_active: false,
            gesture_task: None,
            coast_ticking: false,
            gesture_epoch: std::time::Instant::now(),
            export_context: None,
            outbox: vec![],
            recent: vec![],
            offered: None,
            last_request: None,
            search_hits: vec![],
            search_index: 0,
            drag_start: None,
            scrollbar_drag: false,
            horizontal_drag: false,
            discard_candidate: None,
            open_generation: 0,
            open_cancel: Cancellation::default(),
            highlight_cancel: Arc::new(std::sync::atomic::AtomicUsize::new(0)),
            save_tasks: HashMap::new(),
            view_task: None,
            tree_rows: Arc::new(vec![]),
            collapsed_dirs: Default::default(),
            tree_cursor: 0,
            tree_focus: cx.focus_handle().tab_stop(true),
            panel_focus: cx.focus_handle().tab_stop(false),
            return_focus: None,
            scrollbar_hide_task: None,
            palette_index: 0,
            theme_index: 0,
            palette_scroll: ScrollHandle::new(),
            file_list: ListState::new(0, ListAlignment::Top, px(200.)),
            visible_files: vec![],
            _subscriptions: subscriptions,
        };
        if let Some(reference) = this.settings.theme.clone() {
            let welcome = std::mem::take(&mut this.status);
            this.apply_theme(Some(&reference), window, cx);
            this.status = welcome;
        }
        if theme_changed {
            this.save_settings(cx);
        }
        this
    }
    pub(crate) fn skin(&self) -> Skin {
        self.palette
            .as_ref()
            .map(Skin::from_palette)
            .unwrap_or_else(|| Skin::new(self.dark))
    }
    pub fn unsaved(&self) -> bool {
        self.active
            .as_ref()
            .is_some_and(|a| a.drafts.iter().any(|d| !d.is_saved()) || a.view.revision > a.view_ack)
    }
    pub fn open(&mut self, request: OpenRequest, refresh: bool, cx: &mut Context<Self>) {
        self.open_pending(request, refresh, None, cx);
    }
    fn open_pending(
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
        self.filter_files(cx);
        if let Some(id) = selected {
            self.select_file(id, cx)
        }
        self.status =
            "Snapshot loaded. The source holds still until another revision is accepted on purpose."
                .into();
        self.refresh_recent(cx);
        self.refresh_outbox(cx);
    }
    pub fn filter_files(&mut self, cx: &mut Context<Self>) {
        let query = self.filter_input.read(cx).value().to_lowercase();
        self.visible_files = self.active.as_ref().map_or_else(Vec::new, |a| {
            a.snapshot
                .patch
                .files
                .iter()
                .filter(|f| f.display_path().to_lowercase().contains(&query))
                .map(|f| f.id.clone())
                .collect()
        });
        let entries = self.active.as_ref().map_or_else(Vec::new, |a| {
            a.snapshot
                .patch
                .files
                .iter()
                .map(|f| (f.id.clone(), f.display_path()))
                .collect()
        });
        let tree = diffz_core::file_tree::FileTree::new(entries);
        self.visible_files = tree
            .rows(&Default::default(), &query)
            .into_iter()
            .filter_map(|row| row.file)
            .collect();
        self.tree_rows = Arc::new(tree.rows(&self.collapsed_dirs, &query));
        self.tree_cursor = self.tree_cursor.min(self.tree_rows.len().saturating_sub(1));
        self.file_list = ListState::new(self.tree_rows.len(), ListAlignment::Top, px(180.));
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
        self.boundary_scroll = Default::default();
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
        self.schedule_view_save(cx);
        self.highlight(id, cx);
        cx.notify();
    }
    /// Settings are small and global, so each write lands at once.
    pub fn save_settings(&mut self, cx: &mut Context<Self>) {
        if let Err(e) = self.services.save_settings(self.settings.clone()) {
            self.status = format!("Could not save settings: {e}");
        }
        cx.notify();
    }
    pub fn schedule_view_save(&mut self, cx: &mut Context<Self>) {
        // Read the canvas anchor at execution time; its newest value appears only after the next paint.
        let Some(a) = &mut self.active else { return };
        a.view.revision = a.view.revision.saturating_add(1);
        let revision = a.view.revision;
        let id = a.snapshot.id.clone();
        let services = self.services.clone();
        self.view_task = Some(cx.spawn(async move |this, cx| {
            smol::Timer::after(Duration::from_millis(250)).await;
            let view = this
                .update(cx, |app, _| {
                    app.remember_anchor();
                    app.active
                        .as_ref()
                        .filter(|a| a.snapshot.id == id && a.view.revision == revision)
                        .map(|a| a.view.clone())
                })
                .ok()
                .flatten();
            let Some(view) = view else { return };
            let store_id = id.clone();
            let result = cx
                .background_spawn(async move { services.save_view(&store_id, view) })
                .await;
            let _ = this.update(cx, |app, cx| {
                if let Some(a) = app.active.as_mut().filter(|a| a.snapshot.id == id) {
                    match result {
                        Ok(()) => a.view_ack = a.view_ack.max(revision),
                        Err(e) => app.status = format!("View not saved: {}", e.message),
                    }
                }
                cx.notify();
            });
        }));
    }
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
        self.draft_input
            .update(cx, |input, cx| input.set_value(d.body, window, cx));
        self.draft_input.read(cx).focus_handle(cx).focus(window, cx);
        cx.notify();
    }
    fn edit_draft(&mut self, body: String, cx: &mut Context<Self>) {
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
                        if entry.state == OutboxState::Confirmed
                            && let Some(a) = app
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
                        if entry.state == OutboxState::Confirmed
                            && let Some(a) = app
                                .active
                                .as_mut()
                                .filter(|a| a.snapshot.id == entry.prepared.snapshot)
                        {
                            for c in entry.prepared.comments {
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
                    Err(e) => app.status = e.message,
                }
                app.refresh_outbox(cx);
                cx.notify();
            });
        })
        .detach();
    }
    pub(crate) fn show_recents(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.return_focus = window.focused(cx);
        self.panel = Panel::Recent;
        self.panel_focus.focus(window, cx);
        self.refresh_recent(cx);
        cx.notify();
    }
    pub(crate) fn show_themes(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.return_focus = window.focused(cx);
        self.panel = Panel::Themes;
        self.panel_focus.focus(window, cx);
        self.themes.clear();
        if let Some(path) = diffz_core::palette::resolve("current") {
            self.themes.push(ThemeEntry {
                name: "Omarchy current theme".into(),
                reference: "current".into(),
                palette: Palette::load(&path).ok(),
            });
        }
        for (name, path) in diffz_core::palette::discover() {
            self.themes.push(ThemeEntry {
                name: name.clone(),
                reference: name,
                palette: Palette::load(&path).ok(),
            });
        }
        self.theme_index = self
            .settings
            .theme
            .as_deref()
            .and_then(|reference| {
                self.themes
                    .iter()
                    .position(|entry| entry.reference == reference)
                    .map(|index| index + 2)
            })
            .unwrap_or(if self.dark { 0 } else { 1 });
        cx.notify();
    }
    pub(crate) fn set_builtin(&mut self, dark: bool, window: &mut Window, cx: &mut Context<Self>) {
        self.dark = dark;
        self.settings.dark = dark;
        self.apply_theme(None, window, cx);
    }
    pub(crate) fn apply_theme(
        &mut self,
        reference: Option<&str>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(reference) = reference else {
            self.palette = None;
            self.theme_path = None;
            self.theme_mtime = None;
            self.settings.theme = None;
            self.save_settings(cx);
            apply_appearance(self.dark, None, Some(window), cx);
            self.rich_cache = None;
            self.status = "Theme: built-in".into();
            cx.notify();
            return;
        };
        let Some(path) = diffz_core::palette::resolve(reference) else {
            self.status = format!("Theme not found: {reference}");
            cx.notify();
            return;
        };
        let palette = match Palette::load(&path) {
            Ok(p) => p,
            Err(e) => {
                self.status = format!("Theme {reference}: {e}");
                cx.notify();
                return;
            }
        };
        let label = theme_label(reference);
        self.dark = palette.mode == Mode::Dark;
        self.settings.dark = self.dark;
        self.settings.theme = Some(reference.to_string());
        self.theme_mtime = fs::metadata(&path).ok().and_then(|m| m.modified().ok());
        self.theme_path = Some(path);
        apply_appearance(
            self.dark,
            Some((label.as_str(), &palette)),
            Some(window),
            cx,
        );
        self.palette = Some(palette);
        self.rich_cache = None;
        self.save_settings(cx);
        self.status = format!("Theme: {label}");
        self.start_theme_watch(window, cx);
        cx.notify();
    }
    fn start_theme_watch(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.theme_task = Some(cx.spawn_in(window, async move |this, cx| {
            loop {
                smol::Timer::after(Duration::from_secs(2)).await;
                let Ok(Some(path)) = this.read_with(cx, |a, _| a.theme_path.clone()) else {
                    break;
                };
                let mtime = fs::metadata(&path).ok().and_then(|m| m.modified().ok());
                let changed = this
                    .update(cx, |a, _| {
                        if mtime.is_some() && mtime != a.theme_mtime {
                            a.theme_mtime = mtime;
                            true
                        } else {
                            false
                        }
                    })
                    .unwrap_or(false);
                if changed && this.update_in(cx, |a, w, c| a.reload_theme(w, c)).is_err() {
                    break;
                }
            }
        }));
    }
    fn reload_theme(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(path) = self.theme_path.clone() else {
            cx.notify();
            return;
        };
        let reference = self.settings.theme.clone().unwrap_or_default();
        let label = theme_label(&reference);
        match Palette::load(&path) {
            Ok(p) => {
                self.dark = p.mode == Mode::Dark;
                self.settings.dark = self.dark;
                apply_appearance(self.dark, Some((label.as_str(), &p)), Some(window), cx);
                self.palette = Some(p);
                self.rich_cache = None;
                self.save_settings(cx);
                self.status = format!("Theme reloaded: {label}");
            }
            Err(e) => {
                self.status = format!("Theme {label}: {e}");
            }
        }
        cx.notify();
    }
    pub(crate) fn hide_recent(&mut self, id: Option<SnapshotId>, cx: &mut Context<Self>) {
        let services = self.services.clone();
        cx.spawn(async move |this, cx| {
            let result = cx
                .background_spawn(async move { services.hide_recent(id) })
                .await;
            let _ = this.update(cx, |a, c| {
                match result {
                    Ok(()) => {
                        a.status =
                            "Gone from the recent list. Drafts and saved reviews are kept.".into();
                        a.refresh_recent(c);
                    }
                    Err(e) => a.status = e.message,
                }
                c.notify();
            });
        })
        .detach();
    }
    pub fn refresh_recent(&mut self, cx: &mut Context<Self>) {
        let s = self.services.clone();
        cx.spawn(async move |this, cx| {
            let r = cx.background_spawn(async move { s.recent() }).await;
            let _ = this.update(cx, |app, cx| {
                if let Ok(v) = r {
                    app.recent = v;
                }
                cx.notify();
            });
        })
        .detach();
    }
    pub fn refresh_outbox(&mut self, cx: &mut Context<Self>) {
        let s = self.services.clone();
        cx.spawn(async move |this, cx| {
            let r = cx.background_spawn(async move { s.outbox() }).await;
            let _ = this.update(cx, |app, cx| {
                if let Ok(v) = r {
                    app.outbox = v;
                }
                cx.notify();
            });
        })
        .detach();
    }
    pub fn begin_export(&mut self, cx: &mut Context<Self>) {
        let Some(a) = &self.active else { return };
        let s = self
            .viewport
            .as_ref()
            .and_then(|v| v.borrow().selection.clone());
        let Some(selection) = s else {
            self.status =
                "Source text must be selected first. An export never pulls in a whole repository on its own."
                    .into();
            return;
        };
        let notes = a
            .drafts
            .iter()
            .filter(|d| {
                d.file == selection.start.file
                    && d.side == selection.start.side
                    && d.line >= selection.start.line.min(selection.end.line)
                    && d.start_line <= selection.start.line.max(selection.end.line)
            })
            .map(|d| d.body.clone())
            .collect();
        match ContextExport::selected(&a.snapshot, selection, notes) {
            Ok(c) => {
                self.export_context = Some(c);
                self.panel = Panel::Export;
            }
            Err(e) => self.status = e,
        }
        cx.notify();
    }
    pub fn export(&mut self, cx: &mut Context<Self>) {
        let Some(context) = self.export_context.clone() else {
            return;
        };
        let path = PathBuf::from(self.export_input.read(cx).value().as_ref());
        if !path.is_absolute() {
            self.status =
                "Pick an absolute path for a brand-new file; no existing file gets overwritten."
                    .into();
            return;
        }
        let services = self.services.clone();
        cx.spawn(async move |this, cx| {
            let r = cx
                .background_spawn(async move { services.export(context, path) })
                .await;
            let _ = this.update(cx, |app, cx| {
                match r {
                    Ok(()) => {
                        app.status =
                            "Context export finished locally, with no agent and no network request."
                                .into();
                        app.panel = Panel::None;
                    }
                    Err(e) => app.status = e.message,
                }
                cx.notify();
            });
        })
        .detach();
    }
    pub fn highlight(&mut self, file: FileId, cx: &mut Context<Self>) {
        self.highlight_cancel
            .store(1, std::sync::atomic::Ordering::Relaxed);
        self.highlight_cancel = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let cancel = self.highlight_cancel.clone();
        let Some(a) = &self.active else { return };
        let snapshot = a.snapshot.clone();
        let id = snapshot.id.clone();
        cx.spawn(async move |this, cx| {
            let job_file = file.clone();
            let job_cancel = cancel.clone();
            let spans = cx
                .background_spawn(async move { highlight_file(&snapshot, &job_file, &job_cancel) })
                .await;
            let _ = this.update(cx, |app, cx| {
                if cancel.load(std::sync::atomic::Ordering::Relaxed) != 0 {
                    return;
                }
                if let Some(v) = &app.viewport {
                    let mut v = v.borrow_mut();
                    if v.snapshot.id == id && v.file == file {
                        v.decorations = Arc::new(spans);
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }
}
fn theme_label(reference: &str) -> String {
    if reference == "current" || reference == "omarchy" {
        return "Omarchy current theme".into();
    }
    if reference.contains('/') || reference.starts_with('~') {
        let path = Path::new(reference);
        let dir = if path.file_name().is_some_and(|n| n == "colors.toml") {
            path.parent().unwrap_or(path)
        } else {
            path
        };
        return dir
            .file_name()
            .and_then(|n| n.to_str())
            .map(String::from)
            .unwrap_or_else(|| reference.to_string());
    }
    reference.to_string()
}

fn highlight_file(
    snapshot: &Snapshot,
    id: &FileId,
    cancel: &std::sync::atomic::AtomicUsize,
) -> Decorations {
    let mut out = HashMap::new();
    let Some(file) = snapshot.file(id) else {
        return out;
    };
    let path = file.display_path();
    if diffz_core::syntax::language_name(&path).is_none() {
        return out;
    }
    for h in &file.hunks {
        for side in [Side::Left, Side::Right] {
            if cancel.load(std::sync::atomic::Ordering::Relaxed) != 0 {
                return HashMap::new();
            }
            let rows: Vec<_> = h.rows.iter().filter(|r| r.number(side).is_some()).collect();
            // Apply the syntax budget before joining source lines into another buffer.
            let bytes =
                rows.iter().map(|r| r.text.len()).sum::<usize>() + rows.len().saturating_sub(1);
            if bytes > 256 * 1024 {
                continue;
            }
            let source = rows.iter().map(|r| &*r.text).collect::<Vec<_>>().join("\n");
            let spans = diffz_core::syntax::highlight(&path, &source, cancel);
            for (row, line_spans) in rows.into_iter().zip(spans) {
                if !line_spans.is_empty()
                    && let Some(n) = row.number(side)
                {
                    out.insert((side, n), line_spans);
                }
            }
        }
    }
    out
}
/// Flip the entire window, native title bar included, between light and dark.
/// A palette drives both the gpui-component theme and the native appearance;
/// `None` brings the built-in light and dark component themes back.
pub(crate) fn apply_appearance(
    dark: bool,
    palette: Option<(&str, &Palette)>,
    window: Option<&mut Window>,
    cx: &mut App,
) {
    cx.set_window_appearance(Some(if dark {
        WindowAppearance::Dark
    } else {
        WindowAppearance::Light
    }));
    let mode = if dark {
        ThemeMode::Dark
    } else {
        ThemeMode::Light
    };
    if let Some((name, p)) = palette {
        let mut colors = ThemeConfigColors::default();
        colors.background = hex(p.background);
        colors.foreground = hex(p.foreground);
        colors.border = hex(p.muted);
        colors.muted = hex(p.lighter_background);
        colors.muted_foreground = hex(p.dark_foreground);
        colors.accent = hex(p.selection);
        colors.accent_foreground = hex(p.foreground);
        colors.primary = hex(p.accent);
        colors.primary_foreground = hex(p.background);
        colors.primary_hover = hex(p.accent.mix(p.foreground, 0.15));
        colors.primary_active = hex(p.accent.mix(p.background, 0.15));
        colors.secondary = hex(p.lighter_background);
        colors.secondary_foreground = hex(p.foreground);
        colors.secondary_hover = hex(p.selection);
        colors.secondary_active = hex(p.muted);
        colors.input = hex(p.muted);
        colors.ring = hex(p.accent);
        colors.selection = hex(p.selection);
        colors.popover = hex(p.dark_background);
        colors.popover_foreground = hex(p.foreground);
        colors.list = hex(p.background);
        colors.list_hover = hex(p.lighter_background);
        colors.list_active = hex(p.selection);
        colors.list_active_border = hex(p.accent);
        colors.sidebar = hex(p.dark_background);
        colors.sidebar_foreground = hex(p.foreground);
        colors.sidebar_border = hex(p.muted);
        colors.sidebar_accent = hex(p.selection);
        colors.sidebar_primary = hex(p.accent);
        colors.button = hex(p.lighter_background);
        colors.button_hover = hex(p.selection);
        colors.button_active = hex(p.muted);
        colors.button_foreground = hex(p.foreground);
        colors.button_primary = hex(p.accent);
        colors.button_primary_foreground = hex(p.background);
        colors.scrollbar = hex(p.background);
        colors.scrollbar_thumb = hex(p.muted);
        colors.scrollbar_thumb_hover = hex(p.dark_foreground);
        colors.link = hex(p.blue);
        colors.caret = hex(p.bright_foreground);
        colors.success = hex(p.green);
        colors.warning = hex(p.yellow);
        colors.danger = hex(p.red);
        colors.info = hex(p.blue);
        let config = ThemeConfig {
            name: name.into(),
            mode: if p.mode == Mode::Dark {
                ThemeMode::Dark
            } else {
                ThemeMode::Light
            },
            is_default: false,
            colors,
            ..Default::default()
        };
        Theme::global_mut(cx).apply_config(&Rc::new(config));
        Theme::change(mode, window, cx);
    } else {
        let (light, dark_cfg) = {
            let r = ThemeRegistry::global(cx);
            (
                r.default_light_theme().clone(),
                r.default_dark_theme().clone(),
            )
        };
        let theme = Theme::global_mut(cx);
        theme.apply_config(&light);
        theme.apply_config(&dark_cfg);
        Theme::change(mode, window, cx);
    }
}

fn hex(rgb: Rgb) -> Option<gpui_kit::SharedString> {
    Some(rgb.hex().into())
}
static WINDOW_FAILED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
/// Runs the desktop application. Returns `false` when no window could be created.
pub fn launch(services: Arc<dyn WorkbenchServices>, options: LaunchOptions) -> bool {
    gpui_kit::application()
        .with_assets(crate::icons::Assets)
        .run(move |cx| {
            diffz_core::timing::mark("gpui run");
            // Read the initial source while the native window and component layer initialize.
            let initial_cancel = Cancellation::default();
            let cancel = initial_cancel.clone();
            let initial_services = services.clone();
            let request = options.initial.clone();
            let initial_task = cx
                .background_executor()
                .spawn(async move { initial_services.open(request, cancel) });
            gpui_kit::init(cx);
            commands::bind(cx);
            cx.on_action(|_: &commands::Quit, cx| cx.quit());
            cx.set_menus(commands::menus());
            cx.activate(true);
            crate::theme::configure_typography(cx);
            Theme::change(ThemeMode::Dark, None, cx);
            cx.set_window_appearance(Some(WindowAppearance::Dark));
            let options_window = WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(Bounds::centered(
                    None,
                    size(px(1360.), px(900.)),
                    cx,
                ))),
                window_min_size: Some(size(px(780.), px(520.))),
                #[cfg(target_os = "linux")]
                app_id: Some("io.github.zzwong.Diffz".to_string()),
                ..gpui_kit::component::TitleBar::window_options()
            };
            cx.spawn(async move |cx| {
                let result = cx.open_window(options_window, |window, cx| {
                    diffz_core::timing::mark("window");
                    window.set_window_title("diffz");
                    let view = cx.new(|cx| {
                        Workbench::new(services, options.font_family, options.theme, window, cx)
                    });
                    view.update(cx, |app, cx| {
                        app.open_pending(
                            options.initial,
                            false,
                            Some((initial_cancel, initial_task)),
                            cx,
                        );
                    });
                    view.read(cx).diff_focus.clone().focus(window, cx);
                    cx.new(|cx| Root::new(view, window, cx))
                });
                if let Err(e) = result {
                    eprintln!("could not create native window: {e}");
                    WINDOW_FAILED.store(true, std::sync::atomic::Ordering::Release);
                    cx.update(|cx| cx.quit());
                }
            })
            .detach();
            cx.on_window_closed(|cx, _| {
                if cx.windows().is_empty() {
                    cx.quit();
                }
            })
            .detach();
        });
    !WINDOW_FAILED.load(std::sync::atomic::Ordering::Acquire)
}

#[cfg(test)]
mod performance_tests {
    use super::*;

    fn snapshot(path: &str, lines: &str, count: usize) -> Snapshot {
        let patch = format!(
            "diff --git a/{path} b/{path}\n--- a/{path}\n+++ b/{path}\n@@ -0,0 +1,{count} @@\n{lines}"
        );
        Snapshot::new(
            "test".into(),
            diffz_core::patch::parse_patch(patch.as_bytes(), Default::default()).unwrap(),
            None,
            vec![],
        )
    }

    #[::core::prelude::v1::test]
    fn plain_files_do_not_allocate_decoration_entries() {
        let s = snapshot("a.txt", "+plain text\n+another line\n", 2);
        assert!(
            highlight_file(
                &s,
                &s.patch.files[0].id,
                &std::sync::atomic::AtomicUsize::new(0)
            )
            .is_empty()
        );
    }

    #[::core::prelude::v1::test]
    fn cancelled_highlighting_drops_pending_decorations() {
        let s = snapshot("a.rs", "+fn main() {}\n", 1);
        assert!(
            highlight_file(
                &s,
                &s.patch.files[0].id,
                &std::sync::atomic::AtomicUsize::new(1)
            )
            .is_empty()
        );
    }

    #[cfg(feature = "syntax")]
    #[::core::prelude::v1::test]
    fn highlighted_files_keep_only_nonempty_spans() {
        let s = snapshot("a.rs", "+fn main() {}\n+\n", 2);
        let decorations = highlight_file(
            &s,
            &s.patch.files[0].id,
            &std::sync::atomic::AtomicUsize::new(0),
        );
        assert!(!decorations[&(Side::Right, 1)].is_empty());
        assert!(!decorations.contains_key(&(Side::Right, 2)));
    }
}
