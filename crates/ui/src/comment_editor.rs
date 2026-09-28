mod markdown;
mod menu;
mod state;
use self::markdown::{Edit, Format, block_context, code_block, format, table};
pub(crate) use self::state::ComposerState;
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
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

impl Workbench {
    pub(crate) fn reset_comment_editor(&mut self) {
        self.composer.reset();
    }

    pub(crate) fn sync_comment_slash(&mut self, cx: &mut Context<Self>) {
        let (text, cursor, selection_empty) = {
            let input = self.composer.draft_input.read(cx);
            (
                input.value().to_string(),
                input.cursor(),
                input.selected_range().is_empty(),
            )
        };
        if self.composer.sync_slash(&text, cursor, selection_empty) {
            cx.notify();
        }
    }

    fn edit_comment(
        &mut self,
        edit: Edit,
        replace_range: Option<Range<usize>>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let input = self.composer.draft_input.clone();
        input.update(cx, |input, cx| {
            let range = replace_range.unwrap_or_else(|| input.selected_range());
            input.set_selected_range(range.clone(), cx);
            input.replace(edit.text, window, cx);
            let start = range.start;
            input.set_selected_range(start + edit.selection.start..start + edit.selection.end, cx);
        });
        input.read(cx).focus_handle(cx).focus(window, cx);
        self.composer.comment_slash = None;
        self.composer.comment_slash_range = None;
        self.composer.comment_slash_query.clear();
        self.composer.comment_slash_index = 0;
        cx.notify();
    }

    fn format_comment(&mut self, action: Format, window: &mut Window, cx: &mut Context<Self>) {
        let edit = {
            let input = self.composer.draft_input.read(cx);
            let text = input.value();
            let range = input.selected_range();
            let selected = text.get(range.clone()).unwrap_or("");
            let edit = format(action, selected);
            if matches!(
                action,
                Format::Heading
                    | Format::Quote
                    | Format::CodeBlock
                    | Format::Bulleted
                    | Format::Numbered
                    | Format::Checklist
            ) {
                block_context(edit, &text[..range.start], &text[range.end..])
            } else {
                edit
            }
        };
        self.edit_comment(edit, None, window, cx);
    }

    fn active_slash_range(&self, cx: &App) -> Option<Range<usize>> {
        self.composer.comment_slash_range.clone().filter(|range| {
            let input = self.composer.draft_input.read(cx);
            input.cursor() == range.end
                && input
                    .value()
                    .get(range.clone())
                    .is_some_and(|text| text.starts_with('/'))
        })
    }

    fn insert_slash_edit(&mut self, edit: Edit, window: &mut Window, cx: &mut Context<Self>) {
        let range = self.active_slash_range(cx);
        let edit = {
            let input = self.composer.draft_input.read(cx);
            let text = input.value();
            let replace = range.clone().unwrap_or_else(|| input.selected_range());
            block_context(edit, &text[..replace.start], &text[replace.end..])
        };
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
                    .contains(&self.composer.comment_slash_query)
                    || cmd
                        .detail()
                        .to_ascii_lowercase()
                        .contains(&self.composer.comment_slash_query)
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
        self.composer.comment_slash_index = 0;
        match cmd {
            SlashCommand::Table => {
                self.composer.table_hover_dimensions = None;
                self.composer.insert_table_hovered = false;
                self.composer.comment_slash = Some(SlashMenu::Table);
            }
            SlashCommand::Language => self.composer.comment_slash = Some(SlashMenu::Language),
            SlashCommand::Replies => self.composer.comment_slash = Some(SlashMenu::Replies),
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
        if self.composer.comment_slash.is_none() {
            return false;
        }
        self.composer.comment_slash = None;
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
            parse(&self.composer.table_columns_input)?,
            parse(&self.composer.table_rows_input)?,
        ))
    }

    fn table_number_focused(&self, window: &Window, cx: &App) -> bool {
        [
            &self.composer.table_columns_input,
            &self.composer.table_rows_input,
        ]
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
        self.composer.table_columns = columns;
        self.composer.table_rows = rows;
        self.composer.table_columns_input.update(cx, |input, cx| {
            input.set_value(columns.to_string(), window, cx)
        });
        self.composer.table_rows_input.update(cx, |input, cx| {
            input.set_value(rows.to_string(), window, cx)
        });
        cx.notify();
    }

    pub(crate) fn move_comment_menu(&mut self, delta: isize, cx: &mut Context<Self>) -> bool {
        let Some(menu) = self.composer.comment_slash else {
            return false;
        };
        let count = self.comment_menu_count(menu);
        if count == 0 {
            return false;
        }
        self.composer.comment_slash_index = (self.composer.comment_slash_index as isize + delta)
            .rem_euclid(count as isize) as usize;
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
        if self.composer.comment_slash == Some(SlashMenu::Table) {
            if self.table_number_focused(window, cx) {
                return false;
            }
            self.set_table_dimensions(
                self.composer.table_columns.min(5),
                (self.composer.table_rows as isize + delta).clamp(1, 5) as usize,
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
        if self.composer.comment_slash == Some(SlashMenu::Table) {
            if self.table_number_focused(window, cx) {
                return false;
            }
            self.set_table_dimensions(
                (self.composer.table_columns as isize + delta).clamp(1, 5) as usize,
                self.composer.table_rows.min(5),
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
        let Some(menu) = self.composer.comment_slash else {
            return false;
        };
        let commands = self.matching_comment_commands();
        let count = self.comment_menu_count(menu);
        if count == 0 {
            return false;
        }
        match menu {
            SlashMenu::Commands => self.choose_slash_command(
                commands[self.composer.comment_slash_index.min(count - 1)],
                window,
                cx,
            ),
            SlashMenu::Table => {
                if let Some((columns, rows)) = self.valid_table_dimensions(cx) {
                    self.insert_slash_edit(table(columns, rows), window, cx);
                }
            }
            SlashMenu::Language => self.insert_slash_edit(
                code_block(
                    LANGUAGES[self.composer.comment_slash_index.min(count - 1)].1,
                    "",
                ),
                window,
                cx,
            ),
            SlashMenu::Replies if self.composer.comment_slash_index == 0 => {
                self.save_comment_reply(window, cx)
            }
            SlashMenu::Replies => {
                let body = self.settings.saved_replies
                    [self.composer.comment_slash_index.min(count - 1) - 1]
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
        let mut body = self.composer.draft_input.read(cx).value().to_string();
        if let Some(range) = self.active_slash_range(cx) {
            body.replace_range(range, "");
        }
        body.trim().to_string()
    }

    fn save_comment_reply(&mut self, window: &mut Window, cx: &mut Context<Self>) {
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
        if let Some(range) = self.active_slash_range(cx) {
            self.edit_comment(Edit::caret(String::new(), 0), Some(range), window, cx);
        }
        self.composer.comment_slash = Some(SlashMenu::Replies);
        self.composer.comment_slash_index = self
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
                        a.composer.comment_slash = if a.composer.comment_slash.is_some() {
                            None
                        } else {
                            Some(SlashMenu::Commands)
                        };
                        a.composer.comment_slash_range = None;
                        a.composer.comment_slash_query.clear();
                        a.composer.comment_slash_index = 0;
                        c.notify();
                    })),
            )
            .child(div().flex_1())
            .child(
                Button::new("comment-preview")
                    .ghost()
                    .small()
                    .when(self.composer.comment_preview, |button| button.secondary())
                    .label(if self.composer.comment_preview {
                        "Write"
                    } else {
                        "Preview"
                    })
                    .on_click(cx.listener(|a, _, w, c| {
                        a.composer.comment_preview = !a.composer.comment_preview;
                        a.composer.comment_slash = None;
                        if !a.composer.comment_preview {
                            a.composer.draft_input.read(c).focus_handle(c).focus(w, c);
                        }
                        c.notify();
                    })),
            );
        div()
            .id("comment-editor")
            .v_flex()
            .gap_2()
            .child(toolbar)
            .child(if self.composer.comment_preview {
                let body = self.composer.draft_input.read(cx).value().to_string();
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
                Textarea::new(&self.composer.draft_input)
                    .h(px(112.))
                    .readonly(readonly)
                    .aria_label("Local review comment in Markdown")
                    .into_any_element()
            })
            .into_any_element()
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
