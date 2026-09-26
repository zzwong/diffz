use super::{AppTheme, Panel, ThemeEntry, Workbench};
use diffz_core::palette::{Mode, Palette, Rgb};
use gpui_kit::component::{Theme, ThemeConfig, ThemeConfigColors, ThemeMode, ThemeRegistry};
use gpui_kit::{prelude::*, *};
use std::{
    fs,
    path::PathBuf,
    rc::Rc,
    time::{Duration, SystemTime},
};

impl Workbench {
    pub(crate) fn show_themes(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.return_focus = window.focused(cx);
        self.panel = Panel::Themes;
        self.panel_focus.focus(window, cx);
        self.themes = self
            .registry
            .themes()
            .into_iter()
            .map(|entry| ThemeEntry {
                theme: AppTheme::load(&entry.path).ok(),
                name: entry.label,
                reference: entry.reference,
            })
            .collect();
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
            self.theme = None;
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
        let Some(diffz_core::registry::ThemeEntry { label, path, .. }) =
            self.registry.resolve_theme(reference)
        else {
            self.status = format!("Theme not found: {reference}");
            cx.notify();
            return;
        };
        let theme = match AppTheme::load(&path) {
            Ok(t) => t,
            Err(e) => {
                self.status = format!("Theme {reference}: {e}");
                cx.notify();
                return;
            }
        };
        self.settings.theme = Some(reference.to_string());
        self.theme_path = Some(path);
        self.status = theme_status("Theme", &label, &theme);
        self.install_theme(&label, theme, window, cx);
        self.start_theme_watch(window, cx);
        cx.notify();
    }

    fn install_theme(
        &mut self,
        label: &str,
        theme: AppTheme,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.dark = theme.mode == Mode::Dark;
        self.settings.dark = self.dark;
        self.theme_mtime = newest_mtime(&theme.files);
        apply_appearance(
            self.dark,
            theme.palette.as_ref().map(|p| (label, p)),
            Some(window),
            cx,
        );
        self.theme = Some(theme);
        self.rich_cache = None;
        self.save_settings(cx);
    }

    fn start_theme_watch(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.theme_task = Some(cx.spawn_in(window, async move |this, cx| {
            loop {
                smol::Timer::after(Duration::from_secs(2)).await;
                let Ok(Some(files)) =
                    this.read_with(cx, |a, _| a.theme.as_ref().map(|t| t.files.clone()))
                else {
                    break;
                };
                let mtime = newest_mtime(&files);
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
        let label = self
            .registry
            .resolve_theme(&reference)
            .map_or(reference, |entry| entry.label);
        match AppTheme::load(&path) {
            Ok(theme) => {
                self.status = theme_status("Theme reloaded", &label, &theme);
                self.install_theme(&label, theme, window, cx);
            }
            Err(e) => {
                self.status = format!("Theme {label}: {e}");
            }
        }
        cx.notify();
    }
}

fn newest_mtime(files: &[PathBuf]) -> Option<SystemTime> {
    files
        .iter()
        .filter_map(|f| fs::metadata(f).ok()?.modified().ok())
        .max()
}

fn theme_status(verb: &str, label: &str, theme: &AppTheme) -> String {
    if theme.warnings.is_empty() {
        format!("{verb}: {label}")
    } else {
        format!("{verb}: {label} · ignored {}", theme.warnings.join(", "))
    }
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
