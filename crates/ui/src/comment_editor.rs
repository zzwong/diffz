mod markdown;
use self::markdown::{Edit, Format, code_block, format, slash_query, table};
use crate::app::Workbench;
use crate::icons::AppIcon;
use diffz_core::domain::SavedReply;
use gpui_kit::component::{
    Disableable, Sizable, StyledExt, button::*, input::Textarea, text::TextView,
};
use gpui_kit::{prelude::*, *};
use std::ops::Range;

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum SlashMenu {
    Commands,
    Table,
    Language,
    Replies,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum SlashCommand {
    Table,
    Language,
    Replies,
    QuoteSource,
}

impl SlashCommand {
    pub const ALL: [Self; 4] = [
        Self::Table,
        Self::Language,
        Self::Replies,
        Self::QuoteSource,
    ];

    pub fn title(self) -> &'static str {
        match self {
            Self::Table => "Table",
            Self::Language => "Code with language",
            Self::Replies => "Saved reply",
            Self::QuoteSource => "Quote selected line",
        }
    }

    pub fn detail(self) -> &'static str {
        match self {
            Self::Table => "Choose rows and columns",
            Self::Language => "Insert a highlighted code fence",
            Self::Replies => "Reuse a local comment",
            Self::QuoteSource => "Include the line being reviewed",
        }
    }

    fn icon(self) -> &'static str {
        match self {
            Self::Table => AppIcon::Rows2.path(),
            Self::Language => "icons/diffz/comment-code-block.svg",
            Self::Replies => AppIcon::MessageSquare.path(),
            Self::QuoteSource => "icons/diffz/comment-quote.svg",
        }
    }
}

pub(crate) const LANGUAGES: [(&str, &str); 6] = [
    ("Rust", "rust"),
    ("TypeScript", "typescript"),
    ("Python", "python"),
    ("Shell", "bash"),
    ("JSON", "json"),
    ("Plain text", "text"),
];

impl Workbench {
    pub(crate) fn reset_comment_editor(&mut self) {
        self.comment_preview = false;
        self.comment_slash = None;
        self.comment_slash_range = None;
        self.comment_slash_query.clear();
        self.comment_slash_index = 0;
    }

    pub(crate) fn sync_comment_slash(&mut self, cx: &mut Context<Self>) {
        let input = self.draft_input.read(cx);
        let text = input.value();
        if let Some((range, query)) = slash_query(&text, input.cursor()) {
            if self.comment_slash_query != query {
                self.comment_slash_index = 0;
            }
            self.comment_slash = Some(SlashMenu::Commands);
            self.comment_slash_range = Some(range);
            self.comment_slash_query = query;
        } else if self.comment_slash_range.is_some() {
            self.comment_slash = None;
            self.comment_slash_range = None;
            self.comment_slash_query.clear();
        }
        cx.notify();
    }

    fn edit_comment(
        &mut self,
        edit: Edit,
        replace_range: Option<Range<usize>>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let input = self.draft_input.clone();
        input.update(cx, |input, cx| {
            let range = replace_range.unwrap_or_else(|| input.selected_range());
            input.set_selected_range(range.clone(), cx);
            input.replace(edit.text, window, cx);
            let start = range.start;
            input.set_selected_range(start + edit.selection.start..start + edit.selection.end, cx);
        });
        input.read(cx).focus_handle(cx).focus(window, cx);
        self.comment_slash = None;
        self.comment_slash_range = None;
        self.comment_slash_query.clear();
        self.comment_slash_index = 0;
        cx.notify();
    }

    fn format_comment(&mut self, action: Format, window: &mut Window, cx: &mut Context<Self>) {
        let selected = {
            let input = self.draft_input.read(cx);
            let text = input.value();
            text.get(input.selected_range()).unwrap_or("").to_owned()
        };
        self.edit_comment(format(action, &selected), None, window, cx);
    }

    fn insert_slash_edit(&mut self, edit: Edit, window: &mut Window, cx: &mut Context<Self>) {
        let range = self.comment_slash_range.clone().filter(|range| {
            let input = self.draft_input.read(cx);
            input.cursor() == range.end
                && input
                    .value()
                    .get(range.clone())
                    .is_some_and(|text| text.starts_with('/'))
        });
        self.edit_comment(edit, range, window, cx);
    }

    fn matching_comment_commands(&self) -> Vec<SlashCommand> {
        SlashCommand::ALL
            .into_iter()
            .filter(|cmd| {
                if *cmd == SlashCommand::QuoteSource
                    && !self.line_context.as_ref().is_some_and(|s| s.end.line > 0)
                {
                    return false;
                }
                cmd.title()
                    .to_ascii_lowercase()
                    .contains(&self.comment_slash_query)
                    || cmd
                        .detail()
                        .to_ascii_lowercase()
                        .contains(&self.comment_slash_query)
            })
            .collect()
    }

    fn quote_source_comment(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(selection) = &self.line_context else {
            return;
        };
        let Some(active) = &self.active else {
            return;
        };
        let Some(file) = active.snapshot.file(&selection.start.file) else {
            return;
        };
        let Some(row) = file.line(selection.end.side, selection.end.line) else {
            return;
        };
        let quote = format!(
            "> {}:{}\n> {}\n\n",
            file.display_path(),
            selection.end.line,
            row.text.trim_end()
        );
        let at = quote.len();
        self.insert_slash_edit(Edit::caret(quote, at), window, cx);
    }

    fn choose_slash_command(
        &mut self,
        cmd: SlashCommand,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.comment_slash_index = if cmd == SlashCommand::Table { 7 } else { 0 };
        match cmd {
            SlashCommand::Table => self.comment_slash = Some(SlashMenu::Table),
            SlashCommand::Language => self.comment_slash = Some(SlashMenu::Language),
            SlashCommand::Replies => self.comment_slash = Some(SlashMenu::Replies),
            SlashCommand::QuoteSource => {
                self.quote_source_comment(window, cx);
                return;
            }
        }
        cx.notify();
    }

    pub(crate) fn handle_comment_menu_key(
        &mut self,
        event: &KeyDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let mods = event.keystroke.modifiers;
        if mods.platform || mods.control || mods.alt {
            return false;
        }
        match event.keystroke.key.as_str() {
            "escape" => self.dismiss_comment_menu(cx),
            "down" => self.move_comment_menu_vertical(1, cx),
            "up" => self.move_comment_menu_vertical(-1, cx),
            "right" => self.move_comment_menu_horizontal(1, cx),
            "left" => self.move_comment_menu_horizontal(-1, cx),
            "enter" => self.activate_comment_menu(window, cx),
            _ => false,
        }
    }

    pub(crate) fn dismiss_comment_menu(&mut self, cx: &mut Context<Self>) -> bool {
        if self.comment_slash.is_none() {
            return false;
        }
        self.comment_slash = None;
        self.comment_slash_range = None;
        cx.stop_propagation();
        cx.notify();
        true
    }

    fn comment_menu_count(&self, menu: SlashMenu) -> usize {
        match menu {
            SlashMenu::Commands => self.matching_comment_commands().len(),
            SlashMenu::Table => 25,
            SlashMenu::Language => LANGUAGES.len(),
            SlashMenu::Replies => self.settings.saved_replies.len() + 1,
        }
    }

    pub(crate) fn move_comment_menu(&mut self, delta: isize, cx: &mut Context<Self>) -> bool {
        let Some(menu) = self.comment_slash else {
            return false;
        };
        let count = self.comment_menu_count(menu);
        if count == 0 {
            return false;
        }
        self.comment_slash_index =
            (self.comment_slash_index as isize + delta).rem_euclid(count as isize) as usize;
        cx.stop_propagation();
        cx.notify();
        true
    }

    pub(crate) fn move_comment_menu_vertical(
        &mut self,
        delta: isize,
        cx: &mut Context<Self>,
    ) -> bool {
        if self.comment_slash == Some(SlashMenu::Table) {
            let column = self.comment_slash_index % 5;
            let row = (self.comment_slash_index / 5) as isize;
            self.comment_slash_index = (row + delta).clamp(0, 4) as usize * 5 + column;
            cx.stop_propagation();
            cx.notify();
            true
        } else {
            self.move_comment_menu(delta, cx)
        }
    }

    pub(crate) fn move_comment_menu_horizontal(
        &mut self,
        delta: isize,
        cx: &mut Context<Self>,
    ) -> bool {
        if self.comment_slash == Some(SlashMenu::Table) {
            let row = self.comment_slash_index / 5;
            let column = (self.comment_slash_index % 5) as isize;
            self.comment_slash_index = row * 5 + (column + delta).clamp(0, 4) as usize;
            cx.stop_propagation();
            cx.notify();
            true
        } else {
            self.move_comment_menu(delta, cx)
        }
    }

    pub(crate) fn activate_comment_menu(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let Some(menu) = self.comment_slash else {
            return false;
        };
        let commands = self.matching_comment_commands();
        let count = self.comment_menu_count(menu);
        if count == 0 {
            return false;
        }
        match menu {
            SlashMenu::Commands => self.choose_slash_command(
                commands[self.comment_slash_index.min(count - 1)],
                window,
                cx,
            ),
            SlashMenu::Table => {
                let index = self.comment_slash_index.min(24);
                self.insert_slash_edit(table(index % 5 + 1, index / 5 + 1), window, cx);
            }
            SlashMenu::Language => self.insert_slash_edit(
                code_block(LANGUAGES[self.comment_slash_index.min(count - 1)].1, ""),
                window,
                cx,
            ),
            SlashMenu::Replies if self.comment_slash_index == 0 => self.save_comment_reply(cx),
            SlashMenu::Replies => {
                let body = self.settings.saved_replies[self.comment_slash_index.min(count - 1) - 1]
                    .body
                    .clone();
                self.insert_slash_edit(Edit::caret(body.clone(), body.len()), window, cx);
            }
        }
        cx.stop_propagation();
        cx.notify();
        true
    }

    fn save_comment_reply(&mut self, cx: &mut Context<Self>) {
        let mut body = self.draft_input.read(cx).value().to_string();
        if let Some(range) = &self.comment_slash_range
            && body.get(range.clone()).is_some()
        {
            body.replace_range(range.clone(), "");
        }
        let body = body.trim().to_string();
        if body.is_empty() {
            return;
        }
        let title = body
            .lines()
            .next()
            .unwrap_or("Reply")
            .chars()
            .take(42)
            .collect::<String>();
        if !self.settings.saved_replies.iter().any(|r| r.body == body) {
            if self.settings.saved_replies.len() == 30 {
                self.settings.saved_replies.remove(0);
            }
            self.settings.saved_replies.push(SavedReply { title, body });
            self.save_settings(cx);
        }
        self.comment_slash = Some(SlashMenu::Replies);
        cx.notify();
    }

    pub(crate) fn comment_composer(&self, readonly: bool, cx: &mut Context<Self>) -> AnyElement {
        let skin = self.skin();
        let actions = [
            (Format::Heading, "heading", "Heading"),
            (Format::Bold, "bold", "Bold"),
            (Format::Italic, "italic", "Italic"),
            (Format::Quote, "quote", "Quote"),
            (Format::InlineCode, "inline-code", "Inline code"),
            (Format::CodeBlock, "code-block", "Code block"),
            (Format::Link, "link", "Link"),
            (Format::Bulleted, "bulleted", "Bulleted list"),
            (Format::Numbered, "numbered", "Numbered list"),
            (Format::Checklist, "checklist", "Checklist"),
        ];
        let mut toolbar = div().h_flex().flex_wrap().gap_1().items_center();
        for (ix, (action, icon, label)) in actions.into_iter().enumerate() {
            toolbar = toolbar.child(
                Button::new(("comment-format", ix))
                    .ghost()
                    .small()
                    .cursor_pointer()
                    .icon(
                        gpui_kit::component::Icon::default()
                            .path(format!("icons/diffz/comment-{icon}.svg"))
                            .size(px(16.)),
                    )
                    .w(px(31.))
                    .h(px(30.))
                    .tooltip(label)
                    .accessibility_label(label)
                    .disabled(readonly)
                    .on_click(cx.listener(move |a, _, w, c| a.format_comment(action, w, c))),
            );
        }
        toolbar = toolbar
            .child(div().w(px(1.)).h(px(20.)).bg(skin.border))
            .child(
                Button::new("comment-slash")
                    .ghost()
                    .small()
                    .label("/")
                    .tooltip("Insert table, code, source quote, or saved reply")
                    .disabled(readonly)
                    .on_click(cx.listener(|a, _, _, c| {
                        a.comment_slash = if a.comment_slash.is_some() {
                            None
                        } else {
                            Some(SlashMenu::Commands)
                        };
                        a.comment_slash_range = None;
                        a.comment_slash_query.clear();
                        a.comment_slash_index = 0;
                        c.notify();
                    })),
            )
            .child(div().flex_1())
            .child(
                Button::new("comment-preview")
                    .ghost()
                    .small()
                    .label(if self.comment_preview {
                        "Write"
                    } else {
                        "Preview"
                    })
                    .on_click(cx.listener(|a, _, w, c| {
                        a.comment_preview = !a.comment_preview;
                        a.comment_slash = None;
                        if !a.comment_preview {
                            a.draft_input.read(c).focus_handle(c).focus(w, c);
                        }
                        c.notify();
                    })),
            );
        let mut composer = div()
            .id("comment-editor")
            .v_flex()
            .gap_2()
            .child(toolbar)
            .child(if self.comment_preview {
                let body = self.draft_input.read(cx).value().to_string();
                div()
                    .min_h(px(112.))
                    .p_3()
                    .rounded_md()
                    .border_1()
                    .border_color(skin.border)
                    .bg(skin.base)
                    .child(
                        TextView::markdown(
                            "comment-preview-body",
                            if body.trim().is_empty() {
                                "Nothing to preview yet.".to_string()
                            } else {
                                body
                            },
                        )
                        .text_size(px(14.)),
                    )
                    .into_any_element()
            } else {
                Textarea::new(&self.draft_input)
                    .h(px(112.))
                    .readonly(readonly)
                    .aria_label("Local review comment in Markdown")
                    .into_any_element()
            });
        if let Some(menu) = self
            .comment_slash
            .filter(|_| !self.comment_preview && !readonly)
        {
            composer = composer.child(self.comment_slash_menu(menu, cx));
        }
        composer.into_any_element()
    }

    fn comment_slash_menu(&self, menu: SlashMenu, cx: &mut Context<Self>) -> AnyElement {
        let skin = self.skin();
        let mut card = div()
            .id("comment-slash-menu")
            .v_flex()
            .gap_2()
            .p_3()
            .w(px(356.))
            .max_w_full()
            .rounded_md()
            .border_1()
            .border_color(skin.border)
            .bg(skin.raised)
            .shadow_lg()
            .max_h(px(332.))
            .overflow_y_scroll();
        let heading = match menu {
            SlashMenu::Commands => "INSERT INTO COMMENT",
            SlashMenu::Table => "INSERT TABLE",
            SlashMenu::Language => "CODE LANGUAGE",
            SlashMenu::Replies => "SAVED REPLIES",
        };
        card = card.child(
            div()
                .h_flex()
                .items_center()
                .justify_between()
                .px_1()
                .child(
                    div()
                        .text_size(px(10.))
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(skin.muted)
                        .child(heading),
                )
                .when(menu != SlashMenu::Commands, |row| {
                    row.child(
                        Button::new("comment-menu-back")
                            .ghost()
                            .small()
                            .label("← Back")
                            .on_click(cx.listener(|a, _, _, c| {
                                a.comment_slash = Some(SlashMenu::Commands);
                                a.comment_slash_index = 0;
                                c.notify();
                            })),
                    )
                }),
        );
        match menu {
            SlashMenu::Commands => {
                let commands = self.matching_comment_commands();
                if commands.is_empty() {
                    card = card.child(
                        div()
                            .p_3()
                            .text_size(px(12.))
                            .text_color(skin.muted)
                            .child("No matching commands"),
                    );
                }
                for (ix, cmd) in commands.into_iter().enumerate() {
                    let selected = ix == self.comment_slash_index;
                    card = card.child(
                        div()
                            .id(("comment-command", ix))
                            .h_flex()
                            .items_center()
                            .gap_3()
                            .px_2()
                            .py_2()
                            .rounded_md()
                            .cursor_pointer()
                            .when(selected, |row| row.bg(skin.selection))
                            .hover(|row| row.bg(skin.selection))
                            .on_hover(cx.listener(move |a, hovered: &bool, _, c| {
                                if *hovered && a.comment_slash_index != ix {
                                    a.comment_slash_index = ix;
                                    c.notify();
                                }
                            }))
                            .child(
                                div()
                                    .w(px(32.))
                                    .h(px(32.))
                                    .flex_shrink_0()
                                    .flex()
                                    .items_center()
                                    .justify_center()
                                    .rounded_md()
                                    .border_1()
                                    .border_color(skin.border)
                                    .bg(skin.surface)
                                    .child(
                                        gpui_kit::component::Icon::default()
                                            .path(cmd.icon())
                                            .size(px(16.))
                                            .text_color(if selected {
                                                skin.accent
                                            } else {
                                                skin.text
                                            }),
                                    ),
                            )
                            .child(
                                div()
                                    .v_flex()
                                    .flex_1()
                                    .min_w_0()
                                    .gap_1()
                                    .child(
                                        div()
                                            .text_size(px(13.))
                                            .font_weight(FontWeight::MEDIUM)
                                            .text_color(skin.text)
                                            .child(cmd.title()),
                                    )
                                    .child(
                                        div()
                                            .text_size(px(11.))
                                            .text_color(skin.muted)
                                            .child(cmd.detail()),
                                    ),
                            )
                            .child(div().text_color(skin.muted).text_size(px(15.)).child("›"))
                            .on_click(
                                cx.listener(move |a, _, w, c| a.choose_slash_command(cmd, w, c)),
                            ),
                    );
                }
            }
            SlashMenu::Table => {
                let columns = self.comment_slash_index % 5 + 1;
                let rows = self.comment_slash_index / 5 + 1;
                card = card.child(
                    div()
                        .h_flex()
                        .items_baseline()
                        .gap_2()
                        .px_1()
                        .child(
                            div()
                                .font_weight(FontWeight::MEDIUM)
                                .text_size(px(17.))
                                .text_color(skin.text)
                                .child(format!("{columns} × {rows}")),
                        )
                        .child(
                            div()
                                .text_size(px(11.))
                                .text_color(skin.muted)
                                .child("columns × rows"),
                        ),
                );
                card = card.child(
                    div()
                        .px_1()
                        .text_size(px(10.))
                        .text_color(skin.muted)
                        .child("COLUMNS →"),
                );
                for grid_row in 1..=5 {
                    let mut row = div().h_flex().items_center().gap_1();
                    for grid_column in 1..=5 {
                        let index = (grid_row - 1) * 5 + grid_column - 1;
                        let highlighted = grid_row <= rows && grid_column <= columns;
                        row = row.child(
                            div()
                                .id(("table-cell", index))
                                .w(px(31.))
                                .h(px(26.))
                                .border_1()
                                .border_color(if highlighted {
                                    skin.accent
                                } else {
                                    skin.border
                                })
                                .bg(if highlighted {
                                    skin.selection
                                } else {
                                    skin.surface
                                })
                                .rounded_sm()
                                .cursor_pointer()
                                .aria_label(format!("{grid_column} columns, {grid_row} rows"))
                                .on_hover(cx.listener(move |a, hovered: &bool, _, c| {
                                    if *hovered && a.comment_slash_index != index {
                                        a.comment_slash_index = index;
                                        c.notify();
                                    }
                                }))
                                .on_click(cx.listener(move |a, _, w, c| {
                                    a.insert_slash_edit(table(grid_column, grid_row), w, c)
                                })),
                        );
                    }
                    row = row.child(
                        div()
                            .pl_2()
                            .text_size(px(10.))
                            .text_color(skin.muted)
                            .child(format!("{grid_row}")),
                    );
                    card = card.child(row);
                }
                card = card.child(
                    div()
                        .px_1()
                        .text_size(px(11.))
                        .text_color(skin.muted)
                        .child("ROWS ↓  ·  Hover or use arrow keys, then Enter"),
                );
            }
            SlashMenu::Language => {
                for (ix, (label, language)) in LANGUAGES.into_iter().enumerate() {
                    card = card.child(
                        div()
                            .id(("code-language", ix))
                            .h_flex()
                            .items_center()
                            .justify_between()
                            .px_2()
                            .py_2()
                            .rounded_md()
                            .when(self.comment_slash_index == ix, |row| row.bg(skin.selection))
                            .hover(|row| row.bg(skin.selection))
                            .cursor_pointer()
                            .child(div().text_size(px(13.)).text_color(skin.text).child(label))
                            .child(
                                div()
                                    .font_family(crate::theme::code_font())
                                    .text_size(px(10.))
                                    .text_color(skin.muted)
                                    .child(format!("```{language}")),
                            )
                            .on_hover(cx.listener(move |a, hovered: &bool, _, c| {
                                if *hovered && a.comment_slash_index != ix {
                                    a.comment_slash_index = ix;
                                    c.notify();
                                }
                            }))
                            .on_click(cx.listener(move |a, _, w, c| {
                                a.insert_slash_edit(code_block(language, ""), w, c)
                            })),
                    );
                }
            }
            SlashMenu::Replies => {
                card = card.child(
                    Button::new("save-reply")
                        .secondary()
                        .small()
                        .when(self.comment_slash_index == 0, |button| button.outline())
                        .cursor_pointer()
                        .label("＋  Save current comment")
                        .on_hover(cx.listener(|a, hovered: &bool, _, c| {
                            if *hovered && a.comment_slash_index != 0 {
                                a.comment_slash_index = 0;
                                c.notify();
                            }
                        }))
                        .on_click(cx.listener(|a, _, _, c| a.save_comment_reply(c))),
                );
                if self.settings.saved_replies.is_empty() {
                    card = card.child(
                        div()
                            .p_2()
                            .text_size(px(11.))
                            .text_color(skin.muted)
                            .child("Write a comment, then save it for future reviews."),
                    );
                }
                for (ix, reply) in self.settings.saved_replies.iter().enumerate() {
                    let body = reply.body.clone();
                    card = card.child(
                        div()
                            .h_flex()
                            .items_center()
                            .gap_2()
                            .rounded_md()
                            .when(self.comment_slash_index == ix + 1, |row| {
                                row.bg(skin.selection)
                            })
                            .child(
                                Button::new(("saved-reply", ix))
                                    .ghost()
                                    .small()
                                    .flex_1()
                                    .cursor_pointer()
                                    .label(reply.title.clone())
                                    .on_hover(cx.listener(move |a, hovered: &bool, _, c| {
                                        if *hovered && a.comment_slash_index != ix + 1 {
                                            a.comment_slash_index = ix + 1;
                                            c.notify();
                                        }
                                    }))
                                    .on_click(cx.listener(move |a, _, w, c| {
                                        let at = body.len();
                                        a.insert_slash_edit(Edit::caret(body.clone(), at), w, c);
                                    })),
                            )
                            .child(
                                Button::new(("remove-reply", ix))
                                    .ghost()
                                    .small()
                                    .label("×")
                                    .tooltip("Remove saved reply")
                                    .on_click(cx.listener(move |a, _, _, c| {
                                        a.settings.saved_replies.remove(ix);
                                        a.comment_slash_index = 0;
                                        a.save_settings(c);
                                        c.notify();
                                    })),
                            ),
                    );
                }
            }
        }
        if matches!(menu, SlashMenu::Commands | SlashMenu::Language) {
            card = card.child(
                div()
                    .px_1()
                    .pt_1()
                    .text_size(px(10.))
                    .text_color(skin.muted)
                    .child("↑↓ Navigate  ·  Enter Select  ·  Esc Close"),
            );
        }
        card.into_any_element()
    }
}
