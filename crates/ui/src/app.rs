mod appearance;
mod review_flow;
mod source;

pub(crate) use self::appearance::apply_appearance;
use crate::{
    commands,
    file_browser::FileBrowserState,
    reader::FilesPeek,
    theme::Skin,
    viewport::{Decorations, Viewport},
};
use diffz_core::{
    domain::*, export::ContextExport, presentation, provider::*, registry::Registry, review::*,
    theme::Theme as AppTheme,
};
use gpui_kit::component::{
    Root, Theme, ThemeMode,
    input::{InputEvent, InputState, TextareaState},
};
use gpui_kit::{prelude::*, *};
use std::{
    cell::RefCell,
    collections::HashMap,
    path::PathBuf,
    rc::Rc,
    sync::Arc,
    time::{Duration, Instant, SystemTime},
};

pub struct LaunchOptions {
    pub initial: OpenRequest,
    pub registry: Arc<Registry>,
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
    Keys,
}
#[derive(Clone, PartialEq, Eq)]
pub(crate) enum SourceMode {
    Remote(ProviderId),
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
    pub theme: Option<AppTheme>,
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
    pub registry: Arc<Registry>,
    pub active: Option<Active>,
    pub viewport: Option<Rc<RefCell<Viewport>>>,
    pub open_input: Entity<InputState>,
    pub base_input: Entity<InputState>,
    pub head_input: Entity<InputState>,
    pub filter_input: Entity<InputState>,
    pub find_input: Entity<InputState>,
    pub palette_input: Entity<InputState>,
    pub draft_input: Entity<TextareaState>,
    pub comment_preview: bool,
    pub comment_slash: Option<crate::comment_editor::SlashMenu>,
    pub comment_slash_range: Option<std::ops::Range<usize>>,
    pub comment_slash_query: String,
    pub comment_slash_index: usize,
    pub summary_input: Entity<TextareaState>,
    pub export_input: Entity<InputState>,
    pub diff_focus: FocusHandle,
    pub root_focus: FocusHandle,
    pub panel: Panel,
    pub source_mode: SourceMode,
    pub files_visible: bool,
    /// The collapsed file panel floating over the diff while its toggle or itself is hovered.
    pub files_peek: FilesPeek,
    pub files_peek_close: Option<Task<()>>,
    /// Waits out the rest the left-edge zone asks for before the panel appears.
    pub files_peek_dwell: Option<Task<()>>,
    /// Clock of the peek's enter or exit motion; `Some` means a frame loop is running.
    pub files_peek_frame: Option<Instant>,
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
    pub theme: Option<AppTheme>,
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
    pub gesture: crate::scrolling::GestureState,
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
    pub annotations: Arc<Vec<diffz_core::annotation::Annotation>>,
    annotate_cancel: Cancellation,
    pub save_tasks: HashMap<DraftId, Task<()>>,
    pub view_task: Option<Task<()>>,
    pub browser: FileBrowserState,
    pub panel_focus: FocusHandle,
    pub return_focus: Option<FocusHandle>,
    pub scrollbar_hide_task: Option<Task<()>>,
    pub palette_index: usize,
    pub theme_index: usize,
    pub palette_scroll: ScrollHandle,
    _subscriptions: Vec<Subscription>,
}
impl Workbench {
    pub fn new(
        services: Arc<dyn WorkbenchServices>,
        registry: Arc<Registry>,
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
        let first_provider = services.providers().into_iter().next();
        let source_mode = first_provider
            .as_ref()
            .map_or(SourceMode::Patch, |p| SourceMode::Remote(p.id()));
        let placeholder = first_provider
            .as_ref()
            .map_or("/path/to/change.patch".to_string(), |p| {
                p.address_hint().to_string()
            });
        let open_input = cx.new(|cx| InputState::new(window, cx).placeholder(placeholder));
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
                    this.sync_comment_slash(cx);
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
            registry,
            active: None,
            viewport: None,
            open_input,
            base_input,
            head_input,
            filter_input,
            find_input,
            palette_input,
            draft_input,
            comment_preview: false,
            comment_slash: None,
            comment_slash_range: None,
            comment_slash_query: String::new(),
            comment_slash_index: 0,
            summary_input,
            export_input,
            diff_focus: cx.focus_handle().tab_stop(true),
            root_focus: cx.focus_handle(),
            panel: Panel::None,
            source_mode,
            files_visible: true,
            files_peek: FilesPeek::default(),
            files_peek_close: None,
            files_peek_dwell: None,
            files_peek_frame: None,
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
            status: "Open a hosted review, fixture, patch, or local Git comparison.".into(),
            theme: None,
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
            gesture: Default::default(),
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
            annotations: Arc::default(),
            annotate_cancel: Cancellation::default(),
            save_tasks: HashMap::new(),
            view_task: None,
            browser: FileBrowserState::new(cx.focus_handle().tab_stop(true)),
            panel_focus: cx.focus_handle().tab_stop(false),
            return_focus: None,
            scrollbar_hide_task: None,
            palette_index: 0,
            theme_index: 0,
            palette_scroll: ScrollHandle::new(),
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
        self.theme
            .as_ref()
            .map(Skin::from_theme)
            .unwrap_or_else(|| Skin::new(self.dark))
    }
    pub fn unsaved(&self) -> bool {
        self.active
            .as_ref()
            .is_some_and(|a| a.drafts.iter().any(|d| !d.is_saved()) || a.view.revision > a.view_ack)
    }
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
    pub(crate) fn show_recents(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.return_focus = window.focused(cx);
        self.panel = Panel::Recent;
        self.panel_focus.focus(window, cx);
        self.refresh_recent(cx);
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
        let registry = self.registry.clone();
        cx.spawn(async move |this, cx| {
            let job_file = file.clone();
            let job_cancel = cancel.clone();
            let spans = cx
                .background_spawn(async move {
                    highlight_file(&registry, &snapshot, &job_file, &job_cancel)
                })
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
fn highlight_file(
    registry: &Registry,
    snapshot: &Snapshot,
    id: &FileId,
    cancel: &std::sync::atomic::AtomicUsize,
) -> Decorations {
    let mut out = HashMap::new();
    let Some(file) = snapshot.file(id) else {
        return out;
    };
    let path = file.display_path();
    if registry.language_name(&path).is_none() {
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
            let spans = registry.highlight(&path, &source, cancel);
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
                        Workbench::new(
                            services,
                            options.registry,
                            options.font_family,
                            options.theme,
                            window,
                            cx,
                        )
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
                &Registry::builtin(),
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
                &Registry::builtin(),
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
            &Registry::builtin(),
            &s,
            &s.patch.files[0].id,
            &std::sync::atomic::AtomicUsize::new(0),
        );
        assert!(!decorations[&(Side::Right, 1)].is_empty());
        assert!(!decorations.contains_key(&(Side::Right, 2)));
    }
}
