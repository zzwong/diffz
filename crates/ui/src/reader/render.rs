use super::*;

impl Render for Workbench {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        diffz_core::timing::mark_once("first render");
        let skin = self.skin();
        let titlebar = self.topbar(window, cx);
        let toolbar = self.toolbar(cx);
        let mut source = div().v_flex().flex_1().min_w_0().min_h_0().child(toolbar);
        if self.find_visible {
            source = source.child(
                div()
                    .h_flex()
                    .px_3()
                    .py_2()
                    .gap_2()
                    .child(Input::new(&self.find_input))
                    .child(if self.search_index < self.search_hits.len() {
                        format!("{} of {}", self.search_index + 1, self.search_hits.len())
                    } else {
                        format!("{} matches", self.search_hits.len())
                    })
                    .child(
                        Button::new("prev-match")
                            .cursor_pointer()
                            .label("Previous")
                            .on_click(cx.listener(|a, _, _, c| a.navigate_hit(false, c))),
                    )
                    .child(
                        div()
                            .track_focus(&self.find_next_focus)
                            .rounded_md()
                            .border_1()
                            .border_color(if self.find_next_focus.contains_focused(window, cx) {
                                skin.accent
                            } else {
                                skin.base.opacity(0.)
                            })
                            .child(
                                Button::new("next-match")
                                    .cursor_pointer()
                                    .label("Next")
                                    .on_click(cx.listener(|a, _, _, c| a.navigate_hit(true, c))),
                            ),
                    ),
            );
        }
        if self.offered.is_some() {
            source = source.child(
                div()
                    .h_flex()
                    .gap_3()
                    .p_2()
                    .bg(skin.raised)
                    .child("A newer revision exists. Your reviewed snapshot is still on screen.")
                    .child(
                        Button::new("accept-revision")
                            .cursor_pointer()
                            .label("Open new revision")
                            .on_click(cx.listener(|a, _, _, c| a.accept_offer(c))),
                    ),
            );
        }
        if let Some(a) = &self.active {
            // Unreadable blame is named from the attribution itself, which is never saved.
            let warnings = a.snapshot.warnings.iter().cloned();
            for warning in warnings
                .chain(a.snapshot.overview.unblamed_warning())
                .take(4)
            {
                source = source.child(div().p_2().text_color(skin.warning).child(warning));
            }
        }
        source = source.child(if self.rich_active() {
            self.rich_view(cx)
        } else {
            self.reader(cx)
        });
        let overlay_inspector =
            self.inspector_visible && f32::from(window.viewport_size().width) < 1100.0;
        let mut body = div()
            .h_flex()
            .items_stretch()
            .flex_1()
            .min_h_0()
            .min_w_0()
            .relative();
        if self.files_visible {
            body = body.child(self.files(cx));
        }
        body = body.child(source);
        if self.inspector_visible && !overlay_inspector {
            body = body.child(self.inspector(cx));
        }
        if self.files_visible {
            body = body.child(self.panel_divider(true, cx));
        }
        if self.inspector_visible && !overlay_inspector {
            body = body.child(self.panel_divider(false, cx));
        }
        if overlay_inspector {
            body = body.child(
                div()
                    .absolute()
                    .top_0()
                    .bottom_0()
                    .right_0()
                    .w(px(self.overview_width))
                    .h_flex()
                    .child(self.inspector(cx))
                    .child(self.panel_divider(false, cx)),
            );
        }
        if !self.files_visible {
            // Neither occluding nor clickable: a click or a selection that starts on the
            // window edge still belongs to the diff underneath.
            body = body.child(
                div()
                    .id("files-peek-edge")
                    .absolute()
                    .top_0()
                    .bottom_0()
                    .left_0()
                    .w(px(FILES_PEEK_EDGE_PX))
                    .on_hover(cx.listener(|a, hovered: &bool, window, cx| {
                        a.files_peek_edge(*hovered, window, cx);
                    }))
                    .on_mouse_move(cx.listener(|a, _: &MouseMoveEvent, window, cx| {
                        a.files_peek_edge_moved(window, cx);
                    })),
            );
        }
        if !self.files_visible && self.files_peek.shown() {
            // The hover region keeps the settled bounds; only the panel inside moves.
            let (offset, opacity) = self.files_peek.frame();
            body = body.child(
                div()
                    .id("files-peek")
                    .absolute()
                    .top_0()
                    .bottom_0()
                    .left_0()
                    .w(px(self.files_width))
                    .occlude()
                    .on_hover(cx.listener(|a, hovered: &bool, window, cx| {
                        let close = a.files_peek.hover_panel(*hovered);
                        a.files_peek_hovered(close, window, cx);
                    }))
                    .on_mouse_move(cx.listener(|a, _: &MouseMoveEvent, window, cx| {
                        a.files_peek_panel_moved(window, cx);
                    }))
                    // Occluding the root takes its mouse-exit with it, so the panel
                    // reports the pointer leaving the window itself.
                    .on_mouse_exit(cx.listener(|a, _: &MouseExitEvent, window, cx| {
                        a.files_peek_left(window, cx);
                    }))
                    .child(
                        div()
                            .absolute()
                            .top_0()
                            .bottom_0()
                            .left(px(offset))
                            .w(px(self.files_width))
                            .opacity(opacity)
                            .shadow_lg()
                            .child(self.files(cx)),
                    ),
            );
        }
        if diffz_core::timing::enabled() && self.active.is_some() {
            body = body.child(
                canvas(
                    |_, _, _| (),
                    |_, _, _, _| {
                        static FIRST_PAINT: std::sync::Once = std::sync::Once::new();
                        FIRST_PAINT.call_once(|| diffz_core::timing::mark("first content paint"));
                    },
                )
                .absolute()
                .size(px(1.)),
            );
        }
        let footer = self.footer(window, cx);
        let mut root = div()
            .id("workbench")
            .on_mouse_exit(cx.listener(|a, _: &MouseExitEvent, window, cx| {
                a.files_peek_left(window, cx);
            }))
            .on_mouse_move(cx.listener(|a, e: &MouseMoveEvent, window, cx| {
                a.files_peek.clear_edge_block(f32::from(e.position.x));
                if cancel_resize_on_unpressed_mouse(&mut a.resizing_panel, e.pressed_button) {
                    cx.notify();
                    return;
                }
                if let Some(state) = a.resizing_panel.as_mut() {
                    let x = f32::from(e.position.x);
                    let now = std::time::Instant::now();
                    let elapsed = now.saturating_duration_since(state.last_sample);
                    if elapsed.is_zero() {
                        state.velocity_x = 0.;
                    } else {
                        state.velocity_x = (x - state.last_x) / elapsed.as_secs_f32();
                    }
                    state.last_x = x;
                    state.last_sample = now;
                    let left = state.left;
                    let desired = state.start_width
                        + if left {
                            x - state.start_x
                        } else {
                            state.start_x - x
                        };
                    a.resize_panel(left, desired, window);
                    cx.notify();
                }
            }))
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|a, e: &MouseUpEvent, window, cx| {
                    let Some(state) = a.resizing_panel.take() else {
                        return;
                    };
                    let x = f32::from(e.position.x);
                    let desired = state.start_width
                        + if state.left {
                            x - state.start_x
                        } else {
                            state.start_x - x
                        };
                    let velocity_x = effective_resize_velocity(
                        state.velocity_x,
                        std::time::Instant::now().saturating_duration_since(state.last_sample),
                    );
                    a.resize_panel(state.left, desired, window);
                    if state.left && should_collapse_files(x, desired, velocity_x) {
                        a.files_width = state.start_width;
                        a.files_visible = false;
                        a.reset_files_peek();
                        a.diff_focus.focus(window, cx);
                        a.schedule_view_save(cx);
                    }
                    cx.notify();
                }),
            )
            .track_focus(&self.root_focus)
            .key_context("Workbench")
            .relative()
            .v_flex()
            .size_full()
            .bg(skin.base)
            .text_color(skin.text)
            .font_family(crate::theme::ui_font())
            .text_size(px(13.))
            .capture_action(
                cx.listener(|a, action: &gpui_kit::component::input::Enter, w, c| {
                    if a.panel == Panel::Line && !action.secondary && a.activate_comment_menu(w, c)
                    {
                        c.stop_propagation();
                    } else if a.panel == Panel::Line && action.secondary {
                        a.command(Command::Cancel, w, c);
                        c.stop_propagation();
                    } else {
                        c.propagate();
                    }
                }),
            )
            .capture_action(
                cx.listener(|a, _: &gpui_kit::component::input::Escape, w, c| {
                    if a.panel == Panel::Line && a.dismiss_comment_menu(c) {
                        c.stop_propagation();
                    } else if a.panel != Panel::None {
                        a.command(Command::Cancel, w, c);
                        c.stop_propagation();
                    } else {
                        c.propagate();
                    }
                }),
            )
            .capture_action(
                cx.listener(|a, _: &gpui_kit::component::input::MoveDown, w, c| {
                    if !a.move_comment_menu_vertical(1, w, c) {
                        c.propagate();
                    }
                }),
            )
            .capture_action(
                cx.listener(|a, _: &gpui_kit::component::input::MoveUp, w, c| {
                    if !a.move_comment_menu_vertical(-1, w, c) {
                        c.propagate();
                    }
                }),
            )
            .capture_action(
                cx.listener(|a, _: &gpui_kit::component::input::MoveRight, w, c| {
                    if !a.move_comment_menu_horizontal(1, w, c) {
                        c.propagate();
                    }
                }),
            )
            .capture_action(
                cx.listener(|a, _: &gpui_kit::component::input::MoveLeft, w, c| {
                    if !a.move_comment_menu_horizontal(-1, w, c) {
                        c.propagate();
                    }
                }),
            )
            .capture_key_down(
                cx.listener(|a, event: &KeyDownEvent, w, c| a.handle_keys(event, w, c)),
            )
            .on_action(
                cx.listener(|a, _: &commands::Recent, w, c| a.command(Command::Recent, w, c)),
            )
            .on_action(
                cx.listener(|a, _: &commands::Themes, w, c| a.command(Command::Themes, w, c)),
            )
            .on_action(cx.listener(|a, _: &commands::Open, w, c| a.command(Command::Open, w, c)))
            .on_action(
                cx.listener(|a, _: &commands::PasteOpen, w, c| a.command(Command::PasteOpen, w, c)),
            )
            .on_action(cx.listener(|a, _: &commands::Find, w, c| a.command(Command::Find, w, c)))
            .on_action(cx.listener(|a, _: &commands::Files, w, c| a.command(Command::Files, w, c)))
            .on_action(
                cx.listener(|a, _: &commands::Inspector, w, c| a.command(Command::Inspector, w, c)),
            )
            .on_action(
                cx.listener(|a, _: &commands::Palette, w, c| a.command(Command::Palette, w, c)),
            )
            .on_action(cx.listener(|a, _: &commands::Wrap, w, c| a.command(Command::Wrap, w, c)))
            .on_action(cx.listener(|a, _: &commands::Split, w, c| a.command(Command::Split, w, c)))
            .on_action(cx.listener(|a, _: &commands::Rich, w, c| a.command(Command::Rich, w, c)))
            .on_action(
                cx.listener(|a, _: &commands::RichInline, w, c| {
                    a.command(Command::RichInline, w, c)
                }),
            )
            .on_action(
                cx.listener(|a, _: &commands::CopySource, w, c| a.command(Command::Copy, w, c)),
            )
            .on_action(
                cx.listener(|a, _: &commands::Comment, w, c| a.command(Command::Comment, w, c)),
            )
            .on_action(cx.listener(|a, _: &commands::CommentFile, w, c| {
                a.command(Command::CommentFile, w, c)
            }))
            .on_action(
                cx.listener(|a, _: &commands::NextHunk, w, c| a.command(Command::NextHunk, w, c)),
            )
            .on_action(cx.listener(|a, _: &commands::PreviousHunk, w, c| {
                a.command(Command::PreviousHunk, w, c)
            }))
            .on_action(
                cx.listener(|a, _: &commands::NextFile, w, c| a.command(Command::NextFile, w, c)),
            )
            .on_action(cx.listener(|a, _: &commands::PreviousFile, w, c| {
                a.command(Command::PreviousFile, w, c)
            }))
            .on_action(
                cx.listener(|a, _: &commands::Preview, w, c| a.command(Command::Preview, w, c)),
            )
            .on_action(
                cx.listener(|a, _: &commands::Refresh, w, c| a.command(Command::Refresh, w, c)),
            )
            .on_action(
                cx.listener(|a, _: &commands::ZoomIn, w, c| a.command(Command::ZoomIn, w, c)),
            )
            .on_action(
                cx.listener(|a, _: &commands::ZoomOut, w, c| a.command(Command::ZoomOut, w, c)),
            )
            .on_action(cx.listener(|a, _: &commands::Keys, w, c| a.command(Command::Keys, w, c)))
            .on_action(
                cx.listener(|a, _: &commands::Cancel, w, c| a.command(Command::Cancel, w, c)),
            )
            .child(titlebar)
            .child(body)
            .child(footer);
        if self.panel == Panel::Line {
            root = root.child(self.line_panel(window, cx));
        } else if self.panel != Panel::None {
            root = root.child(self.panel_view(window, cx));
        }
        root
    }
}
