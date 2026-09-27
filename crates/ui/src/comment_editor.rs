mod markdown;
use self::markdown::{Edit, Format, code_block, format, slash_query, table};
use crate::app::Workbench;
use crate::icons::AppIcon;
use crate::theme::Skin;
use diffz_core::domain::SavedReply;
use diffz_core::registry::LanguageProvider;
use diffz_core::syntax::BuiltinGrammars;
use gpui_kit::component::{
    Disableable, Sizable, StyledExt,
    button::*,
    input::{Input, Textarea},
};
use gpui_kit::{prelude::*, *};
use std::{ops::Range, sync::atomic::AtomicUsize};

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

pub(crate) const MAX_TABLE_DIMENSION: usize = 20;

fn preview_code_styles(
    block: &gpui_kit::base::text::CodeBlock,
    skin: Skin,
) -> Vec<(Range<usize>, HighlightStyle)> {
    let Some(language) = block.lang() else {
        return Vec::new();
    };
    let path = match language.as_ref() {
        "rust" | "rs" => "preview.rs",
        "typescript" | "ts" => "preview.ts",
        "javascript" | "js" => "preview.js",
        "python" | "py" => "preview.py",
        "bash" | "shell" | "sh" => "preview.sh",
        "json" => "preview.json",
        _ => return Vec::new(),
    };
    let code = block.code();
    let Some(lines) = BuiltinGrammars.highlight(path, code.as_ref(), &AtomicUsize::new(0)) else {
        return Vec::new();
    };
    let mut offset = 0;
    let mut styles = Vec::new();
    for (line, spans) in code.split_inclusive('\n').zip(lines) {
        for span in spans {
            styles.push((
                offset + span.bytes.start..offset + span.bytes.end,
                HighlightStyle {
                    color: Some(skin.token(span.token)),
                    ..Default::default()
                },
            ));
        }
        offset += line.len();
    }
    styles
}

fn table_step_button(
    button: gpui_kit::base::Button,
    axis: &'static str,
    action: &'static str,
    glyph: &'static str,
    skin: Skin,
) -> gpui_kit::base::Button {
    button
        .w(px(25.))
        .h_full()
        .rounded_sm()
        .cursor_pointer()
        .text_size(px(17.))
        .text_color(skin.muted)
        .accessibility_label(format!("{action} {axis}"))
        .hover(|button| button.bg(skin.selection).text_color(skin.accent))
        .active(|button| button.bg(skin.accent).text_color(skin.base))
        .child(glyph)
}

impl Workbench {
    pub(crate) fn reset_comment_editor(&mut self) {
        self.comment_preview = false;
        self.comment_slash = None;
        self.comment_slash_range = None;
        self.comment_slash_query.clear();
        self.comment_slash_index = 0;
        self.table_hover_dimensions = None;
        self.insert_table_hovered = false;
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
        self.comment_slash_index = 0;
        match cmd {
            SlashCommand::Table => {
                self.table_hover_dimensions = None;
                self.insert_table_hovered = false;
                self.comment_slash = Some(SlashMenu::Table);
            }
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
            "down" => self.move_comment_menu_vertical(1, window, cx),
            "up" => self.move_comment_menu_vertical(-1, window, cx),
            "right" => self.move_comment_menu_horizontal(1, window, cx),
            "left" => self.move_comment_menu_horizontal(-1, window, cx),
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

    fn valid_table_dimensions(&self, cx: &App) -> Option<(usize, usize)> {
        let parse = |input: &Entity<gpui_kit::component::input::InputState>| {
            input
                .read(cx)
                .value()
                .parse::<usize>()
                .ok()
                .filter(|value| (1..=MAX_TABLE_DIMENSION).contains(value))
        };
        Some((
            parse(&self.table_columns_input)?,
            parse(&self.table_rows_input)?,
        ))
    }

    fn table_number_focused(&self, window: &Window, cx: &App) -> bool {
        [&self.table_columns_input, &self.table_rows_input]
            .into_iter()
            .any(|input| input.read(cx).focus_handle(cx).is_focused(window))
    }

    fn set_table_dimensions(
        &mut self,
        columns: usize,
        rows: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.table_columns = columns;
        self.table_rows = rows;
        self.table_columns_input.update(cx, |input, cx| {
            input.set_value(columns.to_string(), window, cx)
        });
        self.table_rows_input.update(cx, |input, cx| {
            input.set_value(rows.to_string(), window, cx)
        });
        cx.notify();
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
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        if self.comment_slash == Some(SlashMenu::Table) {
            if self.table_number_focused(window, cx) {
                return false;
            }
            self.set_table_dimensions(
                self.table_columns.min(5),
                (self.table_rows as isize + delta).clamp(1, 5) as usize,
                window,
                cx,
            );
            cx.stop_propagation();
            true
        } else {
            self.move_comment_menu(delta, cx)
        }
    }

    pub(crate) fn move_comment_menu_horizontal(
        &mut self,
        delta: isize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        if self.comment_slash == Some(SlashMenu::Table) {
            if self.table_number_focused(window, cx) {
                return false;
            }
            self.set_table_dimensions(
                (self.table_columns as isize + delta).clamp(1, 5) as usize,
                self.table_rows.min(5),
                window,
                cx,
            );
            cx.stop_propagation();
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
                if let Some((columns, rows)) = self.valid_table_dimensions(cx) {
                    self.insert_slash_edit(table(columns, rows), window, cx);
                }
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

    fn current_reply_body(&self, cx: &App) -> String {
        let mut body = self.draft_input.read(cx).value().to_string();
        if let Some(range) = &self.comment_slash_range
            && body.get(range.clone()).is_some()
        {
            body.replace_range(range.clone(), "");
        }
        body.trim().to_string()
    }

    fn save_comment_reply(&mut self, cx: &mut Context<Self>) {
        let body = self.current_reply_body(cx);
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
            self.settings.saved_replies.push(SavedReply {
                title,
                body: body.clone(),
            });
            self.save_settings(cx);
        }
        self.comment_slash = Some(SlashMenu::Replies);
        self.comment_slash_index = self
            .settings
            .saved_replies
            .iter()
            .position(|reply| reply.body == body)
            .map_or(0, |index| index + 1);
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
                    .when(self.comment_preview, |button| button.secondary())
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
        div()
            .id("comment-editor")
            .v_flex()
            .gap_2()
            .child(toolbar)
            .child(if self.comment_preview {
                let body = self.draft_input.read(cx).value().to_string();
                let preview_style = gpui_kit::base::text::TextViewStyle::default()
                    .with_foreground(skin.text)
                    .with_muted_foreground(skin.muted)
                    .with_link(skin.accent)
                    .with_selection(skin.selection)
                    .with_code_background(skin.surface)
                    .with_border(skin.border)
                    .with_paragraph_gap(rems(0.8))
                    .with_heading_base_font_size(px(16.))
                    .with_heading_font_size(|level, base| match level {
                        1 => base * 1.6,
                        2 => base * 1.35,
                        3 => base * 1.15,
                        _ => base,
                    })
                    .with_code_block(
                        StyleRefinement::default()
                            .bg(skin.surface)
                            .border_1()
                            .border_color(skin.border),
                    )
                    .with_table(
                        StyleRefinement::default()
                            .border_1()
                            .border_color(skin.border),
                    )
                    .with_table_head(
                        StyleRefinement::default()
                            .bg(skin.selection)
                            .text_color(skin.text),
                    )
                    .with_table_cell(StyleRefinement::default().bg(skin.surface))
                    .with_inline_code(HighlightStyle {
                        color: Some(skin.accent),
                        background_color: Some(skin.surface),
                        ..Default::default()
                    })
                    .with_dark(self.dark);
                div()
                    .v_flex()
                    .rounded_md()
                    .border_1()
                    .border_color(skin.border)
                    .bg(skin.base)
                    .child(
                        div()
                            .h_flex()
                            .items_center()
                            .px_3()
                            .py_2()
                            .border_b_1()
                            .border_color(skin.border)
                            .bg(skin.surface)
                            .child(
                                div()
                                    .text_size(px(12.))
                                    .font_weight(FontWeight::MEDIUM)
                                    .text_color(skin.text)
                                    .child("Preview"),
                            ),
                    )
                    .child(
                        div()
                            .id("comment-preview-scroll")
                            .min_h(px(148.))
                            .max_h(px(320.))
                            .p_4()
                            .overflow_y_scroll()
                            .child(if body.trim().is_empty() {
                                div()
                                    .text_size(px(12.))
                                    .text_color(skin.muted)
                                    .child("Your rendered comment will appear here.")
                                    .into_any_element()
                            } else {
                                gpui_kit::base::text::TextView::markdown(
                                    "comment-preview-body",
                                    body,
                                )
                                .style(preview_style)
                                .code_block_highlighter(move |block| {
                                    preview_code_styles(block, skin)
                                })
                                .text_size(px(14.))
                                .selectable(true)
                                .into_any_element()
                            }),
                    )
                    .into_any_element()
            } else {
                Textarea::new(&self.draft_input)
                    .h(px(112.))
                    .readonly(readonly)
                    .aria_label("Local review comment in Markdown")
                    .into_any_element()
            })
            .into_any_element()
    }

    fn table_dimension_control(
        &self,
        state: &Entity<gpui_kit::component::input::InputState>,
        axis: &'static str,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let skin = self.skin();
        let focused = state.read(cx).focus_handle(cx).is_focused(window);
        let control = gpui_kit::base::NumberInput::new(state)
            .size_full()
            .decrement_button(move |button| table_step_button(button, axis, "Decrease", "−", skin))
            .input(
                Input::new(state)
                    .appearance(false)
                    .small()
                    .h_full()
                    .text_align(TextAlign::Center),
            )
            .increment_button(move |button| table_step_button(button, axis, "Increase", "+", skin));
        div()
            .h(px(28.))
            .rounded_md()
            .border_1()
            .border_color(if focused { skin.accent } else { skin.border })
            .bg(skin.surface)
            .hover(|frame| frame.border_color(skin.accent))
            .child(control)
            .into_any_element()
    }

    pub(crate) fn comment_slash_menu(
        &self,
        menu: SlashMenu,
        width: f32,
        max_height: f32,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let skin = self.skin();
        let mut card = div()
            .id("comment-slash-menu")
            .occlude()
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .v_flex()
            .gap_2()
            .p_3()
            .w(px(width))
            .rounded_md()
            .border_1()
            .border_color(skin.border)
            .bg(skin.raised)
            .shadow_lg()
            .max_h(px(max_height.min(332.)))
            .overflow_y_scroll();
        let heading = match menu {
            SlashMenu::Commands => "Insert",
            SlashMenu::Table => "Table",
            SlashMenu::Language => "Code language",
            SlashMenu::Replies => "Saved replies",
        };
        card = card.child(
            div()
                .h_flex()
                .items_center()
                .justify_between()
                .px_1()
                .child(
                    div()
                        .text_size(px(12.))
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(skin.text)
                        .child(heading),
                )
                .when(menu != SlashMenu::Commands, |row| {
                    row.child(
                        gpui_kit::base::Button::new("comment-menu-back")
                            .px_2()
                            .py_1()
                            .rounded_sm()
                            .cursor_pointer()
                            .text_size(px(11.))
                            .text_color(skin.muted)
                            .hover(|style| style.bg(skin.selection).text_color(skin.text))
                            .active(|style| style.bg(skin.accent.opacity(0.3)))
                            .child("← Back")
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
                            .active(|row| row.bg(skin.accent.opacity(0.3)))
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
                let columns = self.table_columns;
                let rows = self.table_rows;
                let valid = self.valid_table_dimensions(cx).is_some();
                let mut grid = div().v_flex().gap_1();
                for grid_row in 1..=5 {
                    let mut row = div().h_flex().gap_1();
                    for grid_column in 1..=5 {
                        let (preview_columns, preview_rows) =
                            self.table_hover_dimensions.unwrap_or((columns, rows));
                        let highlighted =
                            grid_row <= preview_rows && grid_column <= preview_columns;
                        row = row.child(
                            div()
                                .id(("table-cell", (grid_row - 1) * 5 + grid_column - 1))
                                .w(px(23.))
                                .h(px(23.))
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
                                .hover(|cell| cell.bg(skin.accent).border_color(skin.accent))
                                .active(|cell| cell.bg(skin.accent.opacity(0.75)))
                                .on_hover(cx.listener(move |a, hovered: &bool, _, c| {
                                    let current = (grid_column, grid_row);
                                    if *hovered && a.table_hover_dimensions != Some(current) {
                                        a.table_hover_dimensions = Some(current);
                                        c.notify();
                                    } else if !*hovered && a.table_hover_dimensions == Some(current)
                                    {
                                        a.table_hover_dimensions = None;
                                        c.notify();
                                    }
                                }))
                                .on_click(cx.listener(move |a, _, w, c| {
                                    a.insert_slash_edit(table(grid_column, grid_row), w, c)
                                })),
                        );
                    }
                    grid = grid.child(row);
                }
                let dimensions = div()
                    .v_flex()
                    .gap_2()
                    .w(px(118.))
                    .child(
                        div()
                            .v_flex()
                            .gap_1()
                            .child(
                                div()
                                    .text_size(px(11.))
                                    .text_color(skin.muted)
                                    .child("Columns"),
                            )
                            .child(self.table_dimension_control(
                                &self.table_columns_input,
                                "columns",
                                window,
                                cx,
                            )),
                    )
                    .child(
                        div()
                            .v_flex()
                            .gap_1()
                            .child(
                                div()
                                    .text_size(px(11.))
                                    .text_color(skin.muted)
                                    .child("Rows"),
                            )
                            .child(self.table_dimension_control(
                                &self.table_rows_input,
                                "rows",
                                window,
                                cx,
                            )),
                    );
                card = card.child(
                    div()
                        .h_flex()
                        .items_start()
                        .gap_3()
                        .px_1()
                        .child(grid)
                        .child(dimensions),
                );
                card = card.child(
                    div()
                        .h_flex()
                        .items_center()
                        .justify_end()
                        .px_1()
                        .pt_2()
                        .border_t_1()
                        .border_color(skin.border)
                        .child(
                            Button::new("insert-table")
                                .primary()
                                .small()
                                .when(!self.insert_table_hovered, |button| button.outline())
                                .label(format!("Insert {columns} × {rows}"))
                                .disabled(!valid)
                                .on_hover(cx.listener(|a, hovered: &bool, _, c| {
                                    if a.insert_table_hovered != *hovered {
                                        a.insert_table_hovered = *hovered;
                                        c.notify();
                                    }
                                }))
                                .on_click(cx.listener(|a, _, w, c| {
                                    if let Some((columns, rows)) = a.valid_table_dimensions(c) {
                                        a.insert_slash_edit(table(columns, rows), w, c);
                                    }
                                })),
                        ),
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
                            .active(|row| row.bg(skin.accent.opacity(0.3)))
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
                let candidate = self.current_reply_body(cx);
                let already_saved = self
                    .settings
                    .saved_replies
                    .iter()
                    .any(|reply| reply.body == candidate);
                let can_save = !candidate.is_empty() && !already_saved;
                card = card.child(
                    div()
                        .id("save-reply")
                        .h_flex()
                        .items_center()
                        .gap_3()
                        .p_2()
                        .rounded_md()
                        .border_1()
                        .border_color(skin.border)
                        .bg(if can_save && self.comment_slash_index == 0 {
                            skin.selection
                        } else {
                            skin.surface
                        })
                        .when(can_save, |row| {
                            row.cursor_pointer()
                                .hover(|s| s.bg(skin.selection).border_color(skin.accent))
                                .active(|s| s.bg(skin.accent.opacity(0.3)))
                        })
                        .child(
                            div()
                                .w(px(32.))
                                .h(px(32.))
                                .flex()
                                .items_center()
                                .justify_center()
                                .rounded_md()
                                .bg(if can_save {
                                    skin.selection
                                } else {
                                    skin.raised
                                })
                                .text_color(if can_save { skin.accent } else { skin.muted })
                                .text_size(px(19.))
                                .child(if already_saved { "✓" } else { "+" }),
                        )
                        .child(
                            div()
                                .v_flex()
                                .gap_1()
                                .child(
                                    div()
                                        .text_size(px(13.))
                                        .font_weight(FontWeight::MEDIUM)
                                        .text_color(if can_save { skin.text } else { skin.muted })
                                        .child(if already_saved {
                                            "Saved to library"
                                        } else {
                                            "Save this draft"
                                        }),
                                )
                                .when(!already_saved, |details| {
                                    details.child(
                                        div().text_size(px(11.)).text_color(skin.muted).child(
                                            if can_save {
                                                "Reuse it in another review"
                                            } else {
                                                "Write a comment first"
                                            },
                                        ),
                                    )
                                }),
                        )
                        .on_hover(cx.listener(|a, hovered: &bool, _, c| {
                            if *hovered && a.comment_slash_index != 0 {
                                a.comment_slash_index = 0;
                                c.notify();
                            }
                        }))
                        .when(can_save, |row| {
                            row.on_click(cx.listener(|a, _, _, c| a.save_comment_reply(c)))
                        }),
                );
                if self.settings.saved_replies.is_empty() {
                    card = card.child(
                        div()
                            .h_flex()
                            .items_center()
                            .gap_3()
                            .p_3()
                            .rounded_md()
                            .bg(skin.surface)
                            .child(
                                gpui_kit::component::Icon::default()
                                    .path(AppIcon::MessageSquare.path())
                                    .size(px(16.))
                                    .text_color(skin.muted),
                            )
                            .child(
                                div()
                                    .v_flex()
                                    .gap_1()
                                    .child(
                                        div()
                                            .text_size(px(12.))
                                            .font_weight(FontWeight::MEDIUM)
                                            .text_color(skin.text)
                                            .child("No saved replies yet"),
                                    )
                                    .child(
                                        div()
                                            .text_size(px(11.))
                                            .text_color(skin.muted)
                                            .child("Save a draft to build your library."),
                                    ),
                            ),
                    );
                }
                for (ix, reply) in self.settings.saved_replies.iter().enumerate() {
                    let body = reply.body.clone();
                    let excerpt = reply
                        .body
                        .lines()
                        .skip(1)
                        .find(|line| !line.trim().is_empty())
                        .unwrap_or("Click to insert")
                        .chars()
                        .take(72)
                        .collect::<String>();
                    card = card.child(
                        div()
                            .id(("saved-reply-row", ix))
                            .h_flex()
                            .items_center()
                            .gap_2()
                            .rounded_md()
                            .border_1()
                            .border_color(skin.border)
                            .bg(skin.surface)
                            .p_1()
                            .when(self.comment_slash_index == ix + 1, |row| {
                                row.bg(skin.selection)
                            })
                            .hover(|row| row.bg(skin.selection).border_color(skin.accent))
                            .active(|row| row.bg(skin.accent.opacity(0.3)))
                            .child(
                                div()
                                    .id(("saved-reply", ix))
                                    .v_flex()
                                    .flex_1()
                                    .min_w_0()
                                    .gap_1()
                                    .px_2()
                                    .py_1()
                                    .rounded_sm()
                                    .cursor_pointer()
                                    .hover(|row| row.bg(skin.selection))
                                    .active(|row| row.bg(skin.accent.opacity(0.3)))
                                    .child(
                                        div()
                                            .text_size(px(12.))
                                            .font_weight(FontWeight::MEDIUM)
                                            .text_color(skin.text)
                                            .text_ellipsis()
                                            .child(reply.title.clone()),
                                    )
                                    .child(
                                        div()
                                            .text_size(px(10.))
                                            .text_color(skin.muted)
                                            .text_ellipsis()
                                            .child(excerpt),
                                    )
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
                                gpui_kit::base::Button::new(("remove-reply", ix))
                                    .w(px(28.))
                                    .h(px(28.))
                                    .rounded_sm()
                                    .cursor_pointer()
                                    .text_size(px(18.))
                                    .text_color(skin.muted)
                                    .hover(|button| {
                                        button
                                            .bg(skin.negative.opacity(0.16))
                                            .text_color(skin.negative)
                                    })
                                    .active(|button| button.bg(skin.negative.opacity(0.28)))
                                    .accessibility_label(format!(
                                        "Remove saved reply: {}",
                                        reply.title
                                    ))
                                    .child("×")
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

#[cfg(all(test, feature = "syntax"))]
mod tests {
    use super::*;
    use core::prelude::v1::test;

    #[test]
    fn preview_code_styles_stay_within_code_block() {
        let source = "fn main() {\n    let answer = 42;\n}\n";
        let block = gpui_kit::base::text::CodeBlock::from_code(source, Some("rust"));
        let styles = preview_code_styles(&block, Skin::new(true));
        assert!(!styles.is_empty());
        assert!(
            styles
                .iter()
                .all(|(range, _)| range.start < range.end && range.end <= source.len())
        );
        assert!(
            styles
                .iter()
                .any(|(range, _)| range.start >= source.find("let").unwrap())
        );
    }
}
