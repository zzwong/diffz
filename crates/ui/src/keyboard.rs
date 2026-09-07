use crate::{
    app::{Panel, Workbench},
    commands::{COMMANDS, Command},
    viewport::Motion,
};
use gpui_kit::*;
impl Workbench {
    pub(crate) fn palette_matches(&self, cx: &App) -> Vec<usize> {
        let q = self.palette_input.read(cx).value().to_lowercase();
        COMMANDS
            .iter()
            .enumerate()
            .filter_map(|(i, (_, label, _))| label.to_lowercase().contains(&q).then_some(i))
            .collect()
    }
    pub(crate) fn handle_keys(
        &mut self,
        event: &KeyDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let key = event.keystroke.key.as_str();
        let mods = event.keystroke.modifiers;
        if self.panel == Panel::None
            && self.find_visible
            && key == "enter"
            && (self.find_input.read(cx).focus_handle(cx).is_focused(window)
                || self.find_next_focus.contains_focused(window, cx))
        {
            self.navigate_hit(!mods.shift, cx);
            self.find_next_focus.focus(window, cx);
            cx.stop_propagation();
            return;
        }
        if self.panel == Panel::Open
            && key == "enter"
            && [&self.open_input, &self.base_input, &self.head_input]
                .iter()
                .any(|i| i.read(cx).focus_handle(cx).is_focused(window))
        {
            self.submit_open(cx);
            cx.stop_propagation();
            return;
        }
        if self.panel == Panel::Line && key == "enter" && (mods.platform || mods.control) {
            self.command(Command::Cancel, window, cx);
            cx.stop_propagation();
            return;
        }
        if key == "escape" {
            self.command(Command::Cancel, window, cx);
            cx.stop_propagation();
            return;
        }
        if key == "tab" {
            for _ in 0..256 {
                if mods.shift {
                    window.focus_prev(cx)
                } else {
                    window.focus_next(cx)
                }
                if self.panel == Panel::None || self.panel_focus.contains_focused(window, cx) {
                    break;
                }
            }
            cx.stop_propagation();
            cx.notify();
            return;
        }
        if self.panel == Panel::Palette
            && self
                .palette_input
                .read(cx)
                .focus_handle(cx)
                .is_focused(window)
        {
            let matches = self.palette_matches(cx);
            match key {
                "up" => self.palette_index = self.palette_index.saturating_sub(1),
                "down" => {
                    self.palette_index =
                        (self.palette_index + 1).min(matches.len().saturating_sub(1))
                }
                "enter" => {
                    if let Some(&i) = matches.get(self.palette_index) {
                        self.return_focus
                            .take()
                            .unwrap_or_else(|| self.diff_focus.clone())
                            .focus(window, cx);
                        self.panel = Panel::None;
                        self.command(COMMANDS[i].0, window, cx);
                    }
                }
                _ => return,
            }
            self.palette_scroll.scroll_to_item(self.palette_index);
            cx.stop_propagation();
            cx.notify();
            return;
        }
        if self.panel == Panel::Themes && self.panel_focus.contains_focused(window, cx) {
            let last = self.themes.len() + 1;
            match key {
                "up" => self.theme_index = self.theme_index.saturating_sub(1),
                "down" => self.theme_index = self.theme_index.saturating_add(1).min(last),
                "home" => self.theme_index = 0,
                "end" => self.theme_index = last,
                "enter" => match self.theme_index {
                    0 => {
                        self.set_builtin(true, window, cx);
                        self.command(Command::Cancel, window, cx);
                    }
                    1 => {
                        self.set_builtin(false, window, cx);
                        self.command(Command::Cancel, window, cx);
                    }
                    index => {
                        if let Some(reference) = self
                            .themes
                            .get(index.saturating_sub(2))
                            .map(|entry| entry.reference.clone())
                        {
                            self.apply_theme(Some(&reference), window, cx);
                            self.command(Command::Cancel, window, cx);
                        }
                    }
                },
                _ => return,
            }
            cx.stop_propagation();
            cx.notify();
            return;
        }
        if self.panel != Panel::None {
            return;
        }
        if self.diff_focus.is_focused(window) && mods.alt && matches!(key, "up" | "down") {
            self.expand_context(
                if key == "up" {
                    crate::viewport::ContextRequest::Above
                } else {
                    crate::viewport::ContextRequest::Below
                },
                cx,
            );
            cx.stop_propagation();
            return;
        }
        if self.tree_focus.is_focused(window) {
            match key {
                "down" => {
                    self.tree_cursor =
                        (self.tree_cursor + 1).min(self.tree_rows.len().saturating_sub(1))
                }
                "up" => self.tree_cursor = self.tree_cursor.saturating_sub(1),
                "home" => self.tree_cursor = 0,
                "end" => self.tree_cursor = self.tree_rows.len().saturating_sub(1),
                "enter" => self.activate_tree(self.tree_cursor, window, cx, true),
                "right" | "left" => {
                    if let Some(row) = self.tree_rows.get(self.tree_cursor).cloned() {
                        if row.file.is_none() && row.expanded == (key == "left") {
                            self.activate_tree(self.tree_cursor, window, cx, false);
                        } else if key == "left" && row.depth > 0 {
                            if let Some(i) = (0..self.tree_cursor)
                                .rev()
                                .find(|&i| self.tree_rows[i].depth < row.depth)
                            {
                                self.tree_cursor = i;
                            }
                        } else if key == "right" {
                            self.tree_cursor =
                                (self.tree_cursor + 1).min(self.tree_rows.len().saturating_sub(1));
                        }
                    }
                }
                "space" => {
                    if let Some(id) = self
                        .tree_rows
                        .get(self.tree_cursor)
                        .and_then(|r| r.file.clone())
                    {
                        self.select_file(id, cx);
                        self.toggle_reviewed(cx);
                    }
                }
                _ => return,
            }
            self.file_list.scroll_to_reveal_item(self.tree_cursor);
            cx.stop_propagation();
            cx.notify();
            return;
        }
        if self.diff_focus.is_focused(window) {
            let document = mods.platform || mods.control;
            let motion = match key {
                "up" => Some(Motion::Up),
                "down" => Some(Motion::Down),
                "left" => Some(Motion::Left),
                "right" => Some(Motion::Right),
                "home" => Some(if document {
                    Motion::First
                } else {
                    Motion::Home
                }),
                "end" => Some(if document { Motion::Last } else { Motion::End }),
                _ => None,
            };
            if let Some(v) = &self.viewport {
                if let Some(motion) = motion {
                    v.borrow_mut().move_selection(motion, mods.shift);
                } else if key == "pageup" || key == "pagedown" {
                    let mut v = v.borrow_mut();
                    let height = v
                        .last
                        .as_ref()
                        .map_or(500., |f| f32::from(f.bounds.size.height));
                    v.scroll(0., height * if key == "pageup" { -0.85 } else { 0.85 });
                } else {
                    return;
                }
            }
            self.schedule_view_save(cx);
            cx.stop_propagation();
            cx.notify();
        }
    }
}
