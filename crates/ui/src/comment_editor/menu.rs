use super::*;

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
                                a.composer.comment_slash = Some(SlashMenu::Commands);
                                a.composer.comment_slash_index = 0;
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
                    let selected = ix == self.composer.comment_slash_index;
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
                                if *hovered && a.composer.comment_slash_index != ix {
                                    a.composer.comment_slash_index = ix;
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
                let columns = self.composer.table_columns;
                let rows = self.composer.table_rows;
                let valid = self.valid_table_dimensions(cx).is_some();
                let mut grid = div().v_flex().gap_1();
                for grid_row in 1..=5 {
                    let mut row = div().h_flex().gap_1();
                    for grid_column in 1..=5 {
                        let (preview_columns, preview_rows) = self
                            .composer
                            .table_hover_dimensions
                            .unwrap_or((columns, rows));
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
                                    if *hovered
                                        && a.composer.table_hover_dimensions != Some(current)
                                    {
                                        a.composer.table_hover_dimensions = Some(current);
                                        c.notify();
                                    } else if !*hovered
                                        && a.composer.table_hover_dimensions == Some(current)
                                    {
                                        a.composer.table_hover_dimensions = None;
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
                                &self.composer.table_columns_input,
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
                                &self.composer.table_rows_input,
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
                                .when(!self.composer.insert_table_hovered, |button| {
                                    button.outline()
                                })
                                .label(format!("Insert {columns} × {rows}"))
                                .disabled(!valid)
                                .on_hover(cx.listener(|a, hovered: &bool, _, c| {
                                    if a.composer.insert_table_hovered != *hovered {
                                        a.composer.insert_table_hovered = *hovered;
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
                            .when(self.composer.comment_slash_index == ix, |row| {
                                row.bg(skin.selection)
                            })
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
                                if *hovered && a.composer.comment_slash_index != ix {
                                    a.composer.comment_slash_index = ix;
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
                        .bg(if can_save && self.composer.comment_slash_index == 0 {
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
                            if *hovered && a.composer.comment_slash_index != 0 {
                                a.composer.comment_slash_index = 0;
                                c.notify();
                            }
                        }))
                        .when(can_save, |row| {
                            row.on_click(cx.listener(|a, _, w, c| a.save_comment_reply(w, c)))
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
                            .when(self.composer.comment_slash_index == ix + 1, |row| {
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
                                        if *hovered && a.composer.comment_slash_index != ix + 1 {
                                            a.composer.comment_slash_index = ix + 1;
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
                                        a.composer.comment_slash_index = 0;
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

#[cfg(test)]
mod pointer_tests {
    use super::*;
    use core::prelude::v1::test;
    use std::{cell::Cell, rc::Rc};

    struct StepButtonHarness {
        hovered: Rc<Cell<bool>>,
        clicked: Rc<Cell<bool>>,
    }

    impl Render for StepButtonHarness {
        fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
            let hover = self.hovered.clone();
            let click = self.clicked.clone();
            div().size(px(50.)).child(
                table_step_button(
                    gpui_kit::base::Button::new("test-column-step"),
                    "column",
                    "Add",
                    "+",
                    Skin::new(true),
                )
                .on_hover(move |hovered: &bool, _, _| hover.set(*hovered))
                .on_click(move |_, _, _| click.set(true)),
            )
        }
    }

    #[gpui_kit::gpui::test]
    fn table_step_button_receives_pointer_hover_and_click(cx: &mut TestAppContext) {
        let hovered = Rc::new(Cell::new(false));
        let clicked = Rc::new(Cell::new(false));
        let (view, cx) = cx.add_window_view({
            let hovered = hovered.clone();
            let clicked = clicked.clone();
            move |_, _| StepButtonHarness { hovered, clicked }
        });
        cx.update(|window, cx| {
            window.draw(cx).clear(cx);
            window.simulate_mouse_move(point(px(10.), px(10.)), cx);
            window.dispatch_event(
                MouseDownEvent {
                    position: point(px(10.), px(10.)),
                    button: MouseButton::Left,
                    modifiers: Default::default(),
                    click_count: 1,
                    first_mouse: false,
                }
                .to_platform_input(),
                cx,
            );
            window.dispatch_event(
                MouseUpEvent {
                    position: point(px(10.), px(10.)),
                    button: MouseButton::Left,
                    modifiers: Default::default(),
                    click_count: 1,
                }
                .to_platform_input(),
                cx,
            );
        });
        let _ = view;
        assert!(hovered.get());
        assert!(clicked.get());
    }
}
