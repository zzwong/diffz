use super::{SlashMenu, markdown::slash_query};
use gpui_kit::component::input::{InputState, TextareaState};
use gpui_kit::*;
use std::ops::Range;

pub(crate) struct ComposerState {
    pub draft_input: Entity<TextareaState>,
    pub table_columns_input: Entity<InputState>,
    pub table_rows_input: Entity<InputState>,
    pub table_columns: usize,
    pub table_rows: usize,
    pub table_hover_dimensions: Option<(usize, usize)>,
    pub insert_table_hovered: bool,
    pub comment_preview: bool,
    pub comment_slash: Option<SlashMenu>,
    pub comment_slash_range: Option<Range<usize>>,
    pub comment_slash_query: String,
    pub comment_slash_index: usize,
}

impl ComposerState {
    pub fn new(
        draft_input: Entity<TextareaState>,
        table_columns_input: Entity<InputState>,
        table_rows_input: Entity<InputState>,
    ) -> Self {
        Self {
            draft_input,
            table_columns_input,
            table_rows_input,
            table_columns: 3,
            table_rows: 2,
            table_hover_dimensions: None,
            insert_table_hovered: false,
            comment_preview: false,
            comment_slash: None,
            comment_slash_range: None,
            comment_slash_query: String::new(),
            comment_slash_index: 0,
        }
    }

    pub fn reset(&mut self) {
        self.comment_preview = false;
        self.comment_slash = None;
        self.comment_slash_range = None;
        self.comment_slash_query.clear();
        self.comment_slash_index = 0;
        self.table_hover_dimensions = None;
        self.insert_table_hovered = false;
    }

    pub(super) fn sync_slash(&mut self, text: &str, cursor: usize, selection_empty: bool) -> bool {
        let previous = (
            self.comment_slash,
            self.comment_slash_range.clone(),
            self.comment_slash_query.clone(),
            self.comment_slash_index,
        );
        if let Some((range, query)) = selection_empty.then(|| slash_query(text, cursor)).flatten() {
            if self.comment_slash_range.as_ref() != Some(&range)
                || self.comment_slash_query != query
            {
                self.comment_slash_index = 0;
                self.comment_slash = Some(SlashMenu::Commands);
            }
            self.comment_slash_range = Some(range);
            self.comment_slash_query = query;
        } else if self.comment_slash_range.is_some() {
            self.comment_slash = None;
            self.comment_slash_range = None;
            self.comment_slash_query.clear();
        }
        previous
            != (
                self.comment_slash,
                self.comment_slash_range.clone(),
                self.comment_slash_query.clone(),
                self.comment_slash_index,
            )
    }
}

#[cfg(test)]
mod interaction_tests {
    use super::*;
    use core::prelude::v1::test;
    use gpui_kit::component::input::Textarea;

    struct Harness(ComposerState);

    impl Render for Harness {
        fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
            Textarea::new(&self.0.draft_input)
        }
    }

    #[gpui_kit::gpui::test]
    fn slash_menu_tracks_textarea_selection_and_resets_on_new_draft(cx: &mut TestAppContext) {
        cx.skip_drawing();
        let window = cx.add_window(|window, cx| {
            let draft = cx.new(|cx| TextareaState::new(window, cx));
            let columns = cx.new(|cx| InputState::new(window, cx));
            let rows = cx.new(|cx| InputState::new(window, cx));
            Harness(ComposerState::new(draft, columns, rows))
        });
        window
            .update(cx, |harness, window, cx| {
                let focus = harness.0.draft_input.read(cx).focus_handle(cx);
                focus.focus(window, cx);
                assert!(focus.is_focused(window));
                harness.0.draft_input.update(cx, |input, cx| {
                    input.set_value("/tab", window, cx);
                    input.set_selected_range(4..4, cx);
                });
                let (text, cursor, empty) = {
                    let input = harness.0.draft_input.read(cx);
                    (
                        input.value().to_string(),
                        input.cursor(),
                        input.selected_range().is_empty(),
                    )
                };
                assert!(harness.0.sync_slash(&text, cursor, empty));
                assert_eq!(harness.0.comment_slash, Some(SlashMenu::Commands));
                assert_eq!(harness.0.comment_slash_query, "tab");
                harness
                    .0
                    .draft_input
                    .update(cx, |input, cx| input.set_selected_range(0..4, cx));
                assert!(harness.0.sync_slash("/tab", 4, false));
                assert_eq!(harness.0.comment_slash, None);
                harness.0.comment_slash = Some(SlashMenu::Table);
                harness.0.comment_preview = true;
                harness.0.reset();
                assert_eq!(harness.0.comment_slash, None);
                assert!(!harness.0.comment_preview);
                assert_eq!(harness.0.draft_input.read(cx).value().as_ref(), "/tab");
            })
            .unwrap();
    }
}
