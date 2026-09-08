//! Paragraph direction is resolved before wrapping. Each visual line then gets
//! UAX #9 L1/L2 reordering, while the native shaper handles joining and mirroring.
use super::{Fragment, run};
use gpui_kit::{FontId, LineLayout, ShapedGlyph, ShapedRun, Window, px, rgb};
use std::{
    collections::{BTreeMap, HashSet, VecDeque},
    ops::Range,
    sync::Arc,
};
use unicode_bidi::{BidiInfo, Level, ParagraphBidiInfo, ParagraphInfo};
use unicode_segmentation::UnicodeSegmentation;

#[derive(Clone, Debug)]
pub(super) struct VisualCluster {
    pub bytes: Range<usize>,
    pub left: f32,
    pub right: f32,
    pub rtl: bool,
}

impl VisualCluster {
    pub fn caret(&self, x: f32) -> usize {
        let left_half = x < (self.left + self.right) * 0.5;
        if left_half != self.rtl {
            self.bytes.start
        } else {
            self.bytes.end
        }
    }
}

/// GPUI exposes glyph origins, rather than advances/caret stops. Keep native
/// shaping clusters indivisible and use adjacent visual origins as cell edges.
/// In particular, never subtract x positions in *logical* byte order for RTL.
pub(super) fn clusters(text: &str, layout: &LineLayout, levels: &[Level]) -> Vec<VisualCluster> {
    let mut boundaries: Vec<_> = text.grapheme_indices(true).map(|(i, _)| i).collect();
    boundaries.push(text.len());
    let mut origins = BTreeMap::<usize, f32>::new();
    for glyph in layout.runs.iter().flat_map(|r| &r.glyphs) {
        if glyph.index >= text.len() {
            continue;
        }
        let index = boundaries[boundaries
            .partition_point(|i| *i <= glyph.index)
            .saturating_sub(1)];
        origins
            .entry(index)
            .and_modify(|x| *x = x.min(f32::from(glyph.position.x)))
            .or_insert(f32::from(glyph.position.x));
    }
    let logical: Vec<_> = origins.into_iter().collect();
    let mut cells: Vec<_> = logical
        .iter()
        .enumerate()
        .map(|(i, &(start, left))| VisualCluster {
            bytes: start
                ..(start
                    + text[start..logical.get(i + 1).map_or(text.len(), |p| p.0)]
                        .trim_end_matches(control)
                        .len()),
            left,
            right: 0.0,
            rtl: levels.get(start).is_some_and(Level::is_rtl),
        })
        .collect();
    cells.sort_by(|a, b| {
        a.left
            .total_cmp(&b.left)
            .then(a.bytes.start.cmp(&b.bytes.start))
    });
    for i in 0..cells.len() {
        cells[i].right = cells
            .get(i + 1)
            .map_or(f32::from(layout.width), |c| c.left)
            .max(cells[i].left);
    }
    cells
}

fn control(c: char) -> bool {
    matches!(c, '\u{061c}' | '\u{200e}' | '\u{200f}' | '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}')
}

/// Supply an already resolved directional run to a native API with no direction
/// argument. Synthetic controls are stripped from the returned indices; source
/// controls already influenced the paragraph levels and must not resolve twice.
fn directional_text(text: &str, range: Range<usize>, rtl: bool) -> (String, Vec<(usize, usize)>) {
    let mut shaped = if rtl {
        "\u{200f}\u{202e}"
    } else {
        "\u{200e}\u{202d}"
    }
    .to_string();
    let mut map = vec![];
    for (i, c) in text[range.clone()].char_indices() {
        if !control(c) {
            map.push((shaped.len(), range.start + i));
            shaped.push(c);
        }
    }
    shaped.push('\u{202c}');
    (shaped, map)
}

/// GPUI's decoration cursor only advances by source index. Painting in logical
/// order with unchanged native positions preserves both RTL ink and span colors.
fn logical_paint_order(layout: &mut LineLayout) {
    let mut glyphs: Vec<(FontId, ShapedGlyph)> = std::mem::take(&mut layout.runs)
        .into_iter()
        .flat_map(|r| r.glyphs.into_iter().map(move |g| (r.font_id, g)))
        .collect();
    glyphs.sort_by_key(|(_, g)| g.index);
    for (font_id, glyph) in glyphs {
        if let Some(last) = layout.runs.last_mut().filter(|r| r.font_id == font_id) {
            last.glyphs.push(glyph);
        } else {
            layout.runs.push(ShapedRun {
                font_id,
                glyphs: vec![glyph],
            });
        }
    }
}

fn shape_fragment(
    bidi: &BidiInfo<'_>,
    para: &ParagraphInfo,
    range: Range<usize>,
    size: f32,
    family: &str,
    window: &mut Window,
) -> Fragment {
    let mut layout = LineLayout {
        font_size: px(size),
        len: range.len(),
        ..Default::default()
    };
    // Reuse paragraph resolution, but copy only this line for L1/L2. Calling
    // BidiInfo::visual_runs here would clone the whole paragraph per fragment.
    let line_bidi = ParagraphBidiInfo {
        text: &bidi.text[range.clone()],
        original_classes: bidi.original_classes[range.clone()].to_vec(),
        levels: bidi.levels[range.clone()].to_vec(),
        paragraph_level: para.level,
        is_pure_ltr: false,
    };
    let (levels, runs) = line_bidi.visual_runs(0..range.len());
    for visual_run in runs {
        let rtl = levels[visual_run.start].is_rtl();
        let (text, map) = directional_text(
            bidi.text,
            visual_run.start + range.start..visual_run.end + range.start,
            rtl,
        );
        if map.is_empty() {
            continue;
        }
        let native = window.text_system().layout_line(
            &text,
            px(size),
            &[run(text.len(), family, rgb(0xffffff).into())],
            None,
        );
        let content_end = text.len() - '\u{202c}'.len_utf8();
        for r in &native.runs {
            let mut glyphs = vec![];
            for g in &r.glyphs {
                if g.index < map[0].0 || g.index >= content_end {
                    continue;
                }
                let mut g = g.clone();
                g.index =
                    map[map.partition_point(|p| p.0 <= g.index).saturating_sub(1)].1 - range.start;
                g.position.x += layout.width;
                glyphs.push(g);
            }
            if !glyphs.is_empty() {
                layout.runs.push(ShapedRun {
                    font_id: r.font_id,
                    glyphs,
                });
            }
        }
        layout.width += native.width;
        layout.ascent = layout.ascent.max(native.ascent);
        layout.descent = layout.descent.max(native.descent);
    }
    let visual = clusters(&bidi.text[range.clone()], &layout, &levels);
    logical_paint_order(&mut layout);
    Fragment {
        display: range,
        layout: Arc::new(layout),
        y: 0.0,
        visual: Some(visual),
    }
}

pub(super) fn shape_if_needed(
    text: &str,
    width: f32,
    size: f32,
    line_height: f32,
    wrap: bool,
    family: &str,
    window: &mut Window,
) -> Option<Vec<Fragment>> {
    // ASCII cannot contain RTL characters or directional controls. Avoid the
    // paragraph-sized bidi allocations for ordinary source lines.
    if text.is_ascii() {
        return None;
    }
    let bidi = BidiInfo::new(text, None);
    if !bidi.has_rtl() && !text.chars().any(control) {
        return None;
    }
    let preferred: HashSet<_> = unicode_linebreak::linebreaks(text)
        .map(|(i, _)| i)
        .collect();
    let mut fragments = vec![];
    for para in &bidi.paragraphs {
        let full = shape_fragment(&bidi, para, para.range.clone(), size, family, window);
        if !wrap || f32::from(full.layout.width) <= width.max(1.0) {
            fragments.push(full);
            continue;
        }
        let mut cells = full.visual.as_ref().unwrap().clone();
        cells.sort_by_key(|c| c.bytes.start);
        let mut advance = 0.0;
        let mut boundaries = vec![(0, 0.0)];
        for cell in cells {
            if cell.bytes.start > boundaries.last().unwrap().0 {
                boundaries.push((cell.bytes.start, advance));
            }
            advance += cell.right - cell.left;
        }
        boundaries.push((para.range.len(), advance));
        let preferred: HashSet<_> = preferred
            .iter()
            .filter_map(|i| i.checked_sub(para.range.start))
            .collect();
        let mut ranges: VecDeque<_> =
            super::measured_breaks(&boundaries, &preferred, Some(width)).into();
        while let Some(mut range) = ranges.pop_front() {
            let mut fragment = shape_fragment(
                &bidi,
                para,
                range.start + para.range.start..range.end + para.range.start,
                size,
                family,
                window,
            );
            // Joining and line-end whitespace can change after wrapping. Verify
            // the actual reshaped width, backing off only at native cluster edges.
            while f32::from(fragment.layout.width) > width.max(1.0) + 0.01 {
                let Some(&(end, _)) = boundaries
                    .iter()
                    .rev()
                    .find(|p| p.0 > range.start && p.0 < range.end)
                else {
                    break;
                };
                ranges.push_front(end..range.end);
                range.end = end;
                fragment = shape_fragment(
                    &bidi,
                    para,
                    range.start + para.range.start..range.end + para.range.start,
                    size,
                    family,
                    window,
                );
            }
            fragments.push(fragment);
        }
    }
    for (i, f) in fragments.iter_mut().enumerate() {
        f.y = i as f32 * line_height;
    }
    Some(fragments)
}
