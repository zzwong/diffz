//! A ("rich") rendered diff for prose: Markdown blocks appear whole, marked added,
//! removed, changed, or unchanged, rather than one source line at a time.
//! Read-only; commenting and selection live in the source view.
use crate::{
    app::Workbench,
    theme::Skin,
    viewport::{BAR, vertical_thumb},
};
use diffz_core::domain::{FileId, SnapshotId};
use diffz_core::rich::{RichBlock, RichItem, RichKind, rich_diff};
use gpui_kit::component::{StyledExt, text::TextView};
use gpui_kit::{prelude::*, *};
use std::rc::Rc;

/// One file's items, keyed on snapshot, file, and the (word marks, dark) pair; toggling
/// or switching themes rebuilds them, and each item's flag tells HTML from Markdown.
pub struct RichCache {
    pub snapshot: SnapshotId,
    pub file: FileId,
    /// (word marks, dark, split, font size × 10): each input that can alter a row's content or its height.
    pub mode: (bool, bool, bool, u32),
    pub items: Rc<Vec<(RichItem, bool)>>,
    /// Virtualized list state; each frame lays out only the blocks on screen.
    pub list: ListState,
}
impl Workbench {
    /// Set when the open file is prose and the rich setting is enabled.
    pub(crate) fn rich_active(&self) -> bool {
        self.settings.rich
            && self.viewport.as_ref().is_some_and(|v| {
                let v = v.borrow();
                v.snapshot
                    .file(&v.file)
                    .is_some_and(|f| diffz_core::presentation::default_wrap(&f.display_path()))
            })
    }
    pub(crate) fn rich_view(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let skin = self.skin();
        let Some(viewport) = self.viewport.clone() else {
            return div().into_any_element();
        };
        let v = viewport.borrow();
        let Some(file) = v.snapshot.file(&v.file) else {
            return div().into_any_element();
        };
        let inline = self.settings.rich_inline;
        let (split, font) = (v.split, v.font_size);
        let key = (
            v.snapshot.id.clone(),
            v.file.clone(),
            (inline, self.dark, split, (font * 10.) as u32),
        );
        let fresh = self
            .rich_cache
            .as_ref()
            .is_some_and(|c| (&c.snapshot, &c.file, c.mode) == (&key.0, &key.1, key.2));
        if !fresh {
            let added_color = hex(blend(skin.positive, skin.base, 0.4));
            let items: Vec<(RichItem, bool)> = rich_diff(file)
                .into_iter()
                .map(|item| match item {
                    RichItem::Block(mut b)
                        if inline
                            && b.kind == RichKind::Changed
                            && b.old.is_some()
                            && b.new.is_some() =>
                    {
                        let (old, new) = (b.old.take().unwrap(), b.new.take().unwrap());
                        let html = plain_paragraph(&old) && plain_paragraph(&new);
                        let (o, n) = if html {
                            let (o, n) = diffz_core::rich::mark_words(
                                &old,
                                &new,
                                ["\u{1}", "\u{2}"],
                                ["\u{3}", "\u{4}"],
                            );
                            (to_html(&o, &added_color), to_html(&n, &added_color))
                        } else {
                            // Markdown text inside a block can't sit under inline HTML, so blocks with
                            // Markdown syntax keep only the strikethrough.
                            let (o, n) =
                                diffz_core::rich::mark_words(&old, &new, ["~~", "~~"], ["", ""]);
                            (fit_strikes(&o), n)
                        };
                        b.old = Some(o);
                        b.new = Some(n);
                        (RichItem::Block(b), html)
                    }
                    item => (item, false),
                })
                .collect();
            self.rich_cache = Some(RichCache {
                snapshot: key.0,
                file: key.1,
                mode: key.2,
                list: ListState::new(items.len(), ListAlignment::Top, px(600.)),
                items: Rc::new(items),
            });
        }
        drop(v);
        let Some(cache) = self.rich_cache.as_ref() else {
            return div().into_any_element();
        };
        let (items, state) = (cache.items.clone(), cache.list.clone());
        let wheel_state = state.clone();
        let (move_state, up_state) = (state.clone(), state.clone());
        let container = div()
            .id("rich-diff")
            .flex_1()
            .min_h_0()
            .key_context("WorkbenchDiff")
            .track_focus(&self.diff_focus)
            .px_6()
            .text_size(px(font))
            .text_color(skin.text)
            // The list handles its own scrolling first (children bubble ahead of parents); here we
            // track the gesture and choose whether a pull at either end turns to another file.
            .on_scroll_wheel(
                cx.listener(move |this, event: &ScrollWheelEvent, window, cx| {
                    this.wheel(
                        event,
                        crate::scrolling::ScrollTarget::Rich(wheel_state.clone()),
                        window,
                        cx,
                    )
                }),
            )
            .on_mouse_move(cx.listener(move |this, event: &MouseMoveEvent, _, cx| {
                if !this.scrollbar_drag {
                    return;
                }
                if event.pressed_button != Some(MouseButton::Left) {
                    this.scrollbar_drag = false;
                    move_state.scrollbar_drag_ended();
                    return;
                }
                scroll_to_pointer(&move_state, event.position.y);
                cx.notify();
            }))
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(move |this, _, _, cx| {
                    if !this.scrollbar_drag {
                        return;
                    }
                    this.scrollbar_drag = false;
                    up_state.scrollbar_drag_ended();
                    cx.notify();
                }),
            );
        let pull = (
            self.boundary_scroll.pulling(),
            self.boundary_scroll.progress(),
        );
        if items.is_empty() {
            return container
                .p_4()
                .child(
                    div()
                        .text_color(skin.muted)
                        .child("This file has nothing to render."),
                )
                .into_any_element();
        }
        let bar_state = state.clone();
        let thumb = self
            .viewport
            .as_ref()
            .filter(|v| v.borrow().scrollbars_visible)
            .and_then(|_| thumb_geometry(&bar_state));
        container
            .relative()
            .child(pull_bar(pull, &skin))
            .child(
                list(state, move |ix, _, _| {
                    let row = match &items[ix] {
                        (RichItem::Gap { count, .. }, _) => gap(*count, &skin),
                        (RichItem::Block(b), html) => {
                            if split {
                                split_row(ix, b, &skin, *html)
                            } else {
                                stacked_row(ix, b, &skin, *html)
                            }
                        }
                    };
                    row.w_full().py_1p5().into_any_element()
                })
                .size_full(),
            )
            .when_some(thumb, |d, (top, height)| {
                d.child(scrollbar(top, height, bar_state, &skin, cx))
            })
            .into_any_element()
    }
}
/// Thumb top and height for the list's own scroll metrics, or none while the content
/// fits. The list measures item heights lazily, so the travel it reports can shift as
/// rows come into view.
fn thumb_geometry(list: &ListState) -> Option<(f32, f32)> {
    let view = f32::from(list.viewport_bounds().size.height);
    let travel = f32::from(list.max_offset_for_scrollbar().y);
    if view <= 0. || travel <= 0. {
        return None;
    }
    let scrolled = f32::from(-list.scroll_px_offset_for_scrollbar().y);
    Some(vertical_thumb(view, view + travel, scrolled))
}
/// The same auto-hiding thumb the source view paints, over the right edge of the list.
/// The track is not occluding, so a wheel over it still scrolls the list beneath.
fn scrollbar(
    top: f32,
    height: f32,
    list: ListState,
    skin: &Skin,
    cx: &mut Context<Workbench>,
) -> Div {
    div()
        .absolute()
        .top_0()
        .bottom_0()
        .right_0()
        .w(px(BAR))
        .on_mouse_down(
            MouseButton::Left,
            cx.listener(move |this, event: &MouseDownEvent, _, cx| {
                this.scrollbar_drag = true;
                list.scrollbar_drag_started();
                scroll_to_pointer(&list, event.position.y);
                cx.stop_propagation();
                cx.notify();
            }),
        )
        .child(
            div()
                .absolute()
                .top(px(top))
                .left(px(2.))
                .w(px(BAR - 4.))
                .h(px(height))
                .bg(skin.muted.opacity(0.5)),
        )
}
/// Scroll the list to the pointer's share of the track, as the source view does.
fn scroll_to_pointer(list: &ListState, y: Pixels) {
    let bounds = list.viewport_bounds();
    let height = f32::from(bounds.size.height).max(1.0);
    let fraction = (f32::from(y - bounds.top()) / height).clamp(0.0, 1.0);
    let travel = f32::from(list.max_offset_for_scrollbar().y);
    list.set_offset_from_scrollbar(point(px(0.), px(-travel * fraction)));
}
/// A thin bar along the edge being pulled, filling as the pull nears the turn.
fn pull_bar((edge, progress): (i8, f32), skin: &Skin) -> AnyElement {
    if edge == 0 || progress <= 0. {
        return div().into_any_element();
    }
    let bar = div()
        .absolute()
        .left_0()
        .right_0()
        .h(px(3.))
        .child(div().h_full().w(relative(progress)).bg(skin.accent));
    if edge > 0 {
        bar.bottom_0()
    } else {
        bar.top_0()
    }
    .into_any_element()
}
/// Word marks, prototype stage: strikes on removed words, highlights on added ones.
/// A block carrying no Markdown syntax, so rendering it as an HTML paragraph is safe.
fn plain_paragraph(text: &str) -> bool {
    !text.contains(['`', '[', '*', '_', '<', '>', '|', '!'])
        && text.lines().all(|l| {
            let t = l.trim_start();
            !(t.starts_with('#')
                || t.starts_with('-')
                || t.starts_with('+')
                || t.starts_with(|c: char| c.is_ascii_digit()) && t.contains(". "))
        })
}
/// Turn a marked paragraph into HTML, swapping sentinels for strikethrough and mark tags.
fn to_html(marked: &str, added_color: &str) -> String {
    let mut out = String::with_capacity(marked.len() + 64);
    out.push_str("<p>");
    for c in marked.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '\n' => out.push(' '),
            '\u{1}' => out.push_str("<s>"),
            '\u{2}' => out.push_str("</s>"),
            '\u{3}' => {
                out.push_str("<mark style=\"background-color: ");
                out.push_str(added_color);
                out.push_str("\">");
            }
            '\u{4}' => out.push_str("</mark>"),
            c => out.push(c),
        }
    }
    out.push_str("</p>");
    out
}
/// Make the `~~` marks parseable. A span crossing a line is dropped (Markdown
/// strikethrough stops at list items), and each marker moves outward to the
/// closest whitespace so it never lands in a code span or on an emphasis delimiter.
/// Widening spans that meet are merged.
fn fit_strikes(marked: &str) -> String {
    let mut out = String::with_capacity(marked.len());
    // Offset in `out` right before the last `~~` that closed, when one exists.
    let mut last_close: Option<usize> = None;
    let mut rest = marked;
    while let Some(open) = rest.find("~~") {
        out.push_str(&rest[..open]);
        let after = &rest[open + 2..];
        let Some(close) = after.find("~~") else {
            out.push_str(after);
            rest = "";
            break;
        };
        let inner = &after[..close];
        rest = &after[close + 2..];
        if inner.contains('\n') {
            out.push_str(inner);
            continue;
        }
        // Push the opening marker back to whitespace; merge into the previous span
        // if the widening reaches it.
        let mut start = out.len();
        while start > 0 {
            let prev = out[..start].chars().next_back().unwrap_or(' ');
            if prev.is_whitespace() {
                break;
            }
            start -= prev.len_utf8();
        }
        let merged = last_close.is_some_and(|c| start <= c + 2);
        if merged {
            let c = last_close.unwrap_or(0);
            let tail = out[c + 2..].to_owned();
            out.truncate(c);
            out.push_str(&tail);
        } else {
            out.insert_str(start, "~~");
        }
        out.push_str(inner);
        // Push the closing marker ahead to whitespace.
        let extra = rest
            .char_indices()
            .find(|(_, c)| c.is_whitespace() || *c == '~')
            .map_or(rest.len(), |(i, _)| i);
        out.push_str(&rest[..extra]);
        rest = &rest[extra..];
        last_close = Some(out.len());
        out.push_str("~~");
    }
    out.push_str(rest);
    out
}
fn blend(a: Hsla, b: Hsla, t: f32) -> Hsla {
    let (a, b) = (a.to_rgb(), b.to_rgb());
    Rgba {
        r: a.r * t + b.r * (1. - t),
        g: a.g * t + b.g * (1. - t),
        b: a.b * t + b.b * (1. - t),
        a: 1.,
    }
    .into()
}
fn hex(c: Hsla) -> String {
    let c = c.to_rgb();
    let ch = |v: f32| (v.clamp(0., 1.) * 255.).round() as u8;
    format!("#{:02x}{:02x}{:02x}", ch(c.r), ch(c.g), ch(c.b))
}
fn gap(count: u32, skin: &Skin) -> Div {
    let rule = || div().flex_1().h(px(1.)).bg(skin.border);
    div()
        .h_flex()
        .items_center()
        .gap_2()
        .py_1()
        .text_size(px(11.))
        .text_color(skin.muted)
        .child(rule())
        .child(format!(
            "{count} unchanged line{}",
            if count == 1 { "" } else { "s" }
        ))
        .child(rule())
}
/// Unified layout: a changed block puts removed text over added text.
fn stacked_row(i: usize, b: &RichBlock, skin: &Skin, html: bool) -> Div {
    let mut row = div().v_flex().gap_1();
    match b.kind {
        RichKind::Same => {
            if let Some(text) = &b.new {
                row = row.child(cell(("rich-same", i), text, None, html));
            }
        }
        RichKind::Added => {
            if let Some(text) = &b.new {
                row = row.child(cell(("rich-new", i), text, Some(skin.positive), html));
            }
        }
        RichKind::Removed => {
            if let Some(text) = &b.old {
                row = row.child(cell(("rich-old", i), text, Some(skin.negative), html));
            }
        }
        RichKind::Changed => {
            if let Some(text) = &b.old {
                row = row.child(cell(("rich-old", i), text, Some(skin.negative), html));
            }
            if let Some(text) = &b.new {
                row = row.child(cell(("rich-new", i), text, Some(skin.positive), html));
            }
        }
    }
    row
}
/// Split layout: old left, new right, with empty cells holding the alignment.
fn split_row(i: usize, b: &RichBlock, skin: &Skin, html: bool) -> Div {
    let (old_tint, new_tint) = match b.kind {
        RichKind::Same => (None, None),
        RichKind::Added => (None, Some(skin.positive)),
        RichKind::Removed => (Some(skin.negative), None),
        RichKind::Changed => (Some(skin.negative), Some(skin.positive)),
    };
    // Each side takes half the row: under flex the text element reports its
    // unwrapped width, so one long line would crush the other side entirely.
    let side = |id: (&'static str, usize), text: Option<&String>, tint: Option<Hsla>| {
        let half = div().w_1_2().flex_none().min_w_0();
        match text {
            Some(text) => half.child(cell(id, text, tint, html)),
            None => half,
        }
    };
    div()
        .flex()
        .flex_row()
        .items_start()
        .w_full()
        .child(side(("rich-old", i), b.old.as_ref(), old_tint).pr_1p5())
        .child(side(("rich-new", i), b.new.as_ref(), new_tint).pl_1p5())
}
fn cell(id: (&'static str, usize), text: &str, tint: Option<Hsla>, html: bool) -> Div {
    div()
        .w_full()
        .min_w_0()
        .px_2()
        .py_1()
        .when_some(tint, |d, tint| d.bg(tint.opacity(0.12)))
        .child(if html {
            TextView::html(id, text.to_owned()).selectable(false)
        } else {
            TextView::markdown(id, text.to_owned()).selectable(false)
        })
}
#[cfg(test)]
mod tests {
    use super::fit_strikes;
    #[test]
    fn strikes_are_widened_to_whitespace_and_multiline_ones_are_dropped() {
        assert_eq!(fit_strikes("a ~~b~~ c"), "a ~~b~~ c");
        assert_eq!(
            fit_strikes("- ~~one\n- two~~ three ~~x~~"),
            "- one\n- two three ~~x~~"
        );
        assert_eq!(fit_strikes("~~open"), "open");
        assert_eq!(
            fit_strikes("- **`throwOnError~~: true`**: guard with `'json' in err`~~."),
            "- ~~**`throwOnError: true`**: guard with `'json' in err`.~~"
        );
        assert_eq!(fit_strikes("~~foo~~-~~bar~~ end"), "~~foo-bar~~ end");
    }
}
