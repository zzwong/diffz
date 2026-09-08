//! Use native glyph measurements with a Unicode break helper; do not wrap by character count.
//! Apply colors during painting so highlighting changes cannot alter measured geometry.
use diffz_core::layout::{DisplayText, snap_grapheme};
use gpui_kit::*;
use std::{
    collections::{BTreeMap, HashSet},
    ops::Range,
    sync::Arc,
};
use unicode_segmentation::UnicodeSegmentation;

mod bidi;

#[derive(Clone)]
pub struct Fragment {
    pub display: Range<usize>,
    pub layout: Arc<LineLayout>,
    pub y: f32,
    visual: Option<Vec<bidi::VisualCluster>>,
}
#[derive(Clone)]
pub struct NativeLine {
    pub source: String,
    pub display: DisplayText,
    pub fragments: Vec<Fragment>,
    pub line_height: f32,
    pub height: f32,
    pub width: f32,
}
#[derive(Debug, Clone)]
pub struct ShapeError(pub String);
pub(crate) fn run(len: usize, family: &str, color: Hsla) -> TextRun {
    TextRun {
        len,
        font: font(family.to_owned()),
        color,
        background_color: None,
        underline: None,
        strikethrough: None,
    }
}
/// Choose breaks from measured native clusters. Tests can provide samples, while
/// the native probe confirms the real font and rendering backend.
pub fn measured_breaks(
    clusters: &[(usize, f32)],
    preferred: &HashSet<usize>,
    width: Option<f32>,
) -> Vec<Range<usize>> {
    let end = clusters.last().map_or(0, |p| p.0);
    let Some(width) = width else {
        return std::iter::once(0..end).collect();
    };
    if clusters.len() < 2 {
        return std::iter::once(0..end).collect();
    }
    let width = width.max(1.0);
    let mut start = 0;
    let mut parts = vec![];
    while start + 1 < clusters.len() {
        let mut j = start + 1;
        let mut last_fit = start;
        let mut last_preferred = None;
        while j < clusters.len() && clusters[j].1 - clusters[start].1 <= width {
            last_fit = j;
            if preferred.contains(&clusters[j].0) {
                last_preferred = Some(j)
            }
            j += 1;
        }
        // Keep one unbreakable shaping cluster visible when the pane is narrower than it.
        let end_ix = if j == clusters.len() {
            clusters.len() - 1
        } else {
            last_preferred.unwrap_or(last_fit.max(start + 1))
        };
        parts.push(clusters[start].0..clusters[end_ix].0);
        start = end_ix;
    }
    if parts.is_empty() {
        parts.push(0..end)
    }
    parts
}
/// Split an already shaped LTR line without changing native glyphs or clusters.
fn ltr_fragments(
    text: &str,
    layout: Arc<LineLayout>,
    width: Option<f32>,
    line_height: f32,
) -> std::result::Result<Vec<Fragment>, ShapeError> {
    // A single ordered fragment already has the native byte indices and origins
    // we need. Keep its shared glyph storage instead of building break maps and
    // cloning every glyph. Unusual native order still takes the checked path.
    if width.is_none_or(|w| f32::from(layout.width) <= w.max(1.0)) && layout.len == text.len() {
        let mut previous = (0, 0.0);
        let ordered = layout.runs.iter().flat_map(|run| &run.glyphs).all(|g| {
            let current = (g.index, f32::from(g.position.x));
            let ordered =
                current.0 < text.len() && current.0 >= previous.0 && current.1 >= previous.1;
            previous = current;
            ordered
        });
        if ordered
            && previous.1 <= f32::from(layout.width)
            && (!text.is_empty() || layout.width == px(0.0))
        {
            return Ok(vec![Fragment {
                display: 0..text.len(),
                layout,
                y: 0.0,
                visual: None,
            }]);
        }
    }
    let mut positions: BTreeMap<usize, f32> = BTreeMap::new();
    positions.insert(0, 0.0);
    for r in &layout.runs {
        for g in &r.glyphs {
            positions.entry(g.index).or_insert(f32::from(g.position.x));
        }
    }
    positions.insert(text.len(), f32::from(layout.width));
    let mut valid: HashSet<usize> = text.grapheme_indices(true).map(|(i, _)| i).collect();
    valid.insert(text.len());
    let clusters: Vec<_> = positions
        .into_iter()
        .filter(|(i, _)| valid.contains(i))
        .collect();
    if clusters.windows(2).any(|w| w[1].1 + 0.01 < w[0].1) {
        return Err(ShapeError(
            "native glyph order needs bidi support; this line was neither reordered nor clipped"
                .into(),
        ));
    }
    let preferred: HashSet<_> = unicode_linebreak::linebreaks(text)
        .map(|(i, _)| i)
        .collect();
    let ranges = measured_breaks(&clusters, &preferred, width);
    let x_at = |i: usize| {
        clusters
            .binary_search_by_key(&i, |p| p.0)
            .ok()
            .map_or(f32::from(layout.width), |n| clusters[n].1)
    };
    let mut layouts: Vec<LineLayout> = ranges
        .iter()
        .map(|range| LineLayout {
            font_size: layout.font_size,
            width: px((x_at(range.end) - x_at(range.start)).max(0.0)),
            ascent: layout.ascent,
            descent: layout.descent,
            runs: vec![],
            len: range.len(),
        })
        .collect();
    // Retain glyph IDs, fallback runs, and cluster locations from native shaping. Fragments are not reshaped.
    for run in &layout.runs {
        let mut buckets: Vec<Vec<ShapedGlyph>> = (0..ranges.len()).map(|_| vec![]).collect();
        for glyph in &run.glyphs {
            let ix = ranges.partition_point(|r| r.end <= glyph.index);
            if ix >= ranges.len() {
                continue;
            }
            let mut g = glyph.clone();
            g.index -= ranges[ix].start;
            g.position.x -= px(x_at(ranges[ix].start));
            buckets[ix].push(g);
        }
        for (i, glyphs) in buckets.into_iter().enumerate() {
            if !glyphs.is_empty() {
                layouts[i].runs.push(ShapedRun {
                    font_id: run.font_id,
                    glyphs,
                });
            }
        }
    }
    let parts = ranges
        .into_iter()
        .zip(layouts)
        .enumerate()
        .map(|(i, (display, layout))| Fragment {
            display,
            layout: Arc::new(layout),
            y: i as f32 * line_height,
            visual: None,
        })
        .collect();
    Ok(parts)
}
impl NativeLine {
    pub fn estimated_bytes(&self) -> usize {
        self.source.capacity()
            + self.display.estimated_bytes()
            + self.fragments.capacity() * std::mem::size_of::<Fragment>()
            + self
                .fragments
                .iter()
                .map(|f| {
                    f.visual.as_ref().map_or(0, |v| {
                        v.capacity() * std::mem::size_of::<bidi::VisualCluster>()
                    }) + std::mem::size_of::<LineLayout>()
                        + f.layout
                            .runs
                            .iter()
                            .map(|r| r.glyphs.capacity() * std::mem::size_of::<ShapedGlyph>())
                            .sum::<usize>()
                })
                .sum::<usize>()
    }

    pub fn shape(
        source: &str,
        width: f32,
        size: f32,
        line_height: f32,
        wrap: bool,
        family: &str,
        window: &mut Window,
    ) -> std::result::Result<Self, ShapeError> {
        if !width.is_finite() || !size.is_finite() || size <= 0.0 {
            return Err(ShapeError("invalid native geometry".into()));
        }
        if source.len() > 1024 * 1024 {
            return Err(ShapeError(
                "source line is over the 1 MiB display limit; export it separately".into(),
            ));
        }
        let display = DisplayText::new(source, 4);
        if let Some(parts) = bidi::shape_if_needed(
            &display.text,
            width,
            size,
            line_height,
            wrap,
            family,
            window,
        ) {
            let max_width = parts
                .iter()
                .map(|p| f32::from(p.layout.width))
                .fold(0.0_f32, f32::max);
            return Ok(Self {
                source: source.into(),
                display,
                height: parts.len() as f32 * line_height,
                line_height,
                fragments: parts,
                width: max_width,
            });
        }
        let layout = window.text_system().layout_line(
            &display.text,
            px(size),
            &[run(display.text.len(), family, rgb(0xffffff).into())],
            None,
        );
        let parts = ltr_fragments(&display.text, layout, wrap.then_some(width), line_height)?;
        let max_width = parts
            .iter()
            .map(|p| f32::from(p.layout.width))
            .fold(0.0_f32, f32::max);
        Ok(Self {
            source: source.into(),
            display,
            height: parts.len() as f32 * line_height,
            line_height,
            fragments: parts,
            width: max_width,
        })
    }
    pub fn fragment_for_source(&self, byte: usize) -> usize {
        let display = self.display.display_byte(byte);
        self.fragments
            .partition_point(|p| p.display.end <= display)
            .min(self.fragments.len().saturating_sub(1))
    }
    pub fn source_at_fragment(&self, ix: usize) -> usize {
        self.fragments
            .get(ix)
            .map_or(0, |p| self.display.source_byte(p.display.start))
    }
    pub fn hit(&self, x: f32, y: f32) -> Option<usize> {
        if y < 0.0 || y >= self.height {
            return None;
        }
        let part = self
            .fragments
            .get((y / self.line_height).floor() as usize)?;
        if let Some(cells) = &part.visual {
            let cell = cells
                .iter()
                .filter(|c| c.right > c.left)
                .find(|c| x < c.right)
                .or_else(|| cells.last());
            let display = part.display.start + cell.map_or(0, |c| c.caret(x));
            return Some(snap_grapheme(
                &self.source,
                self.display.source_byte(display),
            ));
        }
        let display = part.display.start
            + part
                .layout
                .closest_index_for_x(px(x.max(0.0)))
                .min(part.display.len());
        Some(snap_grapheme(
            &self.source,
            self.display.source_byte(display),
        ))
    }
    pub fn rectangles(&self, range: Range<usize>) -> Vec<Bounds<Pixels>> {
        let start = self.display.display_byte(range.start);
        let end = self.display.display_byte(range.end);
        let mut rects = vec![];
        for p in &self.fragments {
            let lo = start.max(p.display.start);
            let hi = end.min(p.display.end);
            if lo >= hi {
                continue;
            }
            if let Some(cells) = &p.visual {
                let mut intervals: Vec<(f32, f32)> = vec![];
                for cell in cells {
                    if cell.bytes.start < hi - p.display.start
                        && cell.bytes.end > lo - p.display.start
                    {
                        if let Some(last) = intervals
                            .last_mut()
                            .filter(|last| cell.left <= last.1 + 0.01)
                        {
                            last.1 = last.1.max(cell.right);
                        } else {
                            intervals.push((cell.left, cell.right));
                        }
                    }
                }
                rects.extend(intervals.into_iter().filter(|(l, r)| r > l).map(|(l, r)| {
                    Bounds::new(point(px(l), px(p.y)), size(px(r - l), px(self.line_height)))
                }));
                continue;
            }
            let x1 = p.layout.x_for_index(lo - p.display.start);
            let x2 = p.layout.x_for_index(hi - p.display.start);
            rects.push(Bounds::new(
                point(x1, px(p.y)),
                size((x2 - x1).max(px(1.0)), px(self.line_height)),
            ));
        }
        rects
    }
    pub fn paint(
        &self,
        origin: Point<Pixels>,
        clip: Bounds<Pixels>,
        base: Hsla,
        spans: &[(Range<usize>, Hsla)],
        window: &mut Window,
        cx: &mut App,
    ) {
        for part in &self.fragments {
            let y = origin.y + px(part.y);
            if y + px(self.line_height) < clip.top() || y > clip.bottom() {
                continue;
            }
            let mut boundaries = vec![0, part.display.len()];
            let mut mapped = vec![];
            for (range, color) in spans {
                let lo = self
                    .display
                    .display_byte(range.start)
                    .max(part.display.start);
                let hi = self.display.display_byte(range.end).min(part.display.end);
                if lo < hi {
                    let r = lo - part.display.start..hi - part.display.start;
                    boundaries.extend([r.start, r.end]);
                    mapped.push((r, *color));
                }
            }
            boundaries.sort_unstable();
            boundaries.dedup();
            let decorations: Vec<DecorationRun> = boundaries
                .windows(2)
                .map(|w| DecorationRun {
                    len: (w[1] - w[0]) as u32,
                    color: mapped
                        .iter()
                        .find(|(r, _)| r.contains(&w[0]))
                        .map_or(base, |(_, c)| *c),
                    background_color: None,
                    underline: None,
                    strikethrough: None,
                })
                .collect();
            // Report paint errors without changing measured height or source positions.
            if let Err(error) = part.layout.paint(
                point(origin.x, y),
                px(self.line_height),
                TextAlign::Left,
                None,
                &decorations,
                window,
                cx,
            ) {
                eprintln!("native text paint failed: {error}");
            }
        }
    }
}
pub fn paint_label(
    text: &str,
    origin: Point<Pixels>,
    color: Hsla,
    font_size: f32,
    family: &str,
    window: &mut Window,
    cx: &mut App,
) {
    let layout = window.text_system().layout_line(
        text,
        px(font_size),
        &[run(text.len(), family, color)],
        None,
    );
    let decorations = [DecorationRun {
        len: text.len() as u32,
        color,
        background_color: None,
        underline: None,
        strikethrough: None,
    }];
    let _ = layout.paint(
        origin,
        px(font_size * 1.4),
        TextAlign::Left,
        None,
        &decorations,
        window,
        cx,
    );
}
pub fn label_width(text: &str, font_size: f32, family: &str, window: &mut Window) -> f32 {
    f32::from(
        window
            .text_system()
            .layout_line(
                text,
                px(font_size),
                &[run(text.len(), family, rgb(0xffffff).into())],
                None,
            )
            .width,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use core::prelude::v1::test;

    // Native shapers return visual positions with logical UTF-8 byte indices.
    fn line(source: &str, glyphs: &[(usize, f32)], width: f32) -> NativeLine {
        let layout = LineLayout {
            font_size: px(14.0),
            width: px(width),
            ascent: px(10.0),
            descent: px(4.0),
            len: source.len(),
            runs: vec![ShapedRun {
                font_id: FontId(0),
                glyphs: glyphs
                    .iter()
                    .map(|&(index, x)| ShapedGlyph {
                        id: GlyphId(0),
                        position: point(px(x), px(0.0)),
                        index,
                        is_emoji: false,
                    })
                    .collect(),
            }],
        };
        NativeLine {
            source: source.into(),
            display: DisplayText::new(source, 4),
            fragments: vec![Fragment {
                display: 0..source.len(),
                visual: Some(bidi::clusters(
                    source,
                    &layout,
                    &unicode_bidi::BidiInfo::new(source, None).levels,
                )),
                layout: Arc::new(layout),
                y: 0.0,
            }],
            line_height: 20.0,
            height: 20.0,
            width,
        }
    }

    #[test]
    fn single_fragments_share_native_glyph_storage() {
        for (source, glyphs, width) in [
            ("abc", vec![(0, 0.0), (1, 10.0), (2, 20.0)], 30.0),
            ("e\u{301}🙂", vec![(0, 0.0), (1, 5.0), (3, 10.0)], 30.0),
            ("", vec![], 0.0),
        ] {
            let native = line(source, &glyphs, width).fragments.remove(0).layout;
            for wrap_width in [None, Some(width), Some(width + 10.0)] {
                let parts = ltr_fragments(source, native.clone(), wrap_width, 20.0).unwrap();
                assert_eq!(parts.len(), 1);
                assert_eq!(parts[0].display, 0..source.len());
                assert_eq!(parts[0].y, 0.0);
                assert!(Arc::ptr_eq(&parts[0].layout, &native), "{source:?}");
            }
        }
    }

    #[test]
    fn wrapping_retains_native_ligatures_graphemes_and_fallback_fonts() {
        let source = "fi e\u{301}🙂z";
        let mut native = line(
            source,
            &[
                (0, 0.0),
                (2, 20.0),
                (3, 30.0),
                (4, 35.0),
                (6, 40.0),
                (10, 60.0),
            ],
            70.0,
        )
        .fragments
        .remove(0)
        .layout;
        let layout = Arc::get_mut(&mut native).unwrap();
        let mut glyphs = std::mem::take(&mut layout.runs[0].glyphs);
        let last = glyphs.pop().unwrap();
        let emoji = glyphs.pop().unwrap();
        layout.runs = vec![
            ShapedRun {
                font_id: FontId(0),
                glyphs,
            },
            ShapedRun {
                font_id: FontId(1),
                glyphs: vec![emoji],
            },
            ShapedRun {
                font_id: FontId(0),
                glyphs: vec![last],
            },
        ];
        let parts = ltr_fragments(source, native, Some(30.0), 20.0).unwrap();
        assert_eq!(
            parts.iter().map(|p| p.display.clone()).collect::<Vec<_>>(),
            vec![0..3, 3..10, 10..11]
        );
        assert_eq!(
            parts
                .iter()
                .map(|p| (f32::from(p.layout.width), p.y))
                .collect::<Vec<_>>(),
            vec![(30.0, 0.0), (30.0, 20.0), (10.0, 40.0)]
        );
        assert_eq!(parts[0].layout.runs[0].glyphs.len(), 2); // The fi ligature stays intact.
        let middle = &parts[1].layout;
        assert_eq!(middle.runs[0].font_id, FontId(0));
        assert_eq!(middle.runs[1].font_id, FontId(1));
        assert_eq!(
            middle.runs[0]
                .glyphs
                .iter()
                .map(|g| (g.index, f32::from(g.position.x)))
                .collect::<Vec<_>>(),
            vec![(0, 0.0), (1, 5.0)]
        );
        assert_eq!(middle.runs[1].glyphs[0].index, 3);
        assert_eq!(f32::from(middle.runs[1].glyphs[0].position.x), 10.0);
    }

    #[test]
    fn unwrapped_layout_still_checks_native_order_and_end_markers() {
        for glyphs in [
            vec![(0, 0.0), (1, 20.0), (2, 10.0)],
            vec![(0, 0.0), (2, 10.0), (1, 20.0)],
        ] {
            let native = line("abc", &glyphs, 30.0).fragments.remove(0).layout;
            assert!(ltr_fragments("abc", native, None, 20.0).is_err());
        }
        let native = line("ab", &[(0, 0.0), (1, 10.0), (2, 20.0)], 20.0)
            .fragments
            .remove(0)
            .layout;
        let parts = ltr_fragments("ab", native, None, 20.0).unwrap();
        assert_eq!(parts[0].layout.runs[0].glyphs.len(), 2);
    }

    // Isolated fragmentation CPU cost, excluding native shaping and rendering.
    // Run with: cargo test -p diffz-ui --release --lib benchmark_ltr_fragments -- --ignored --nocapture
    #[test]
    #[ignore]
    fn benchmark_ltr_fragments() {
        use std::{hint::black_box, time::Instant};
        for (len, iterations) in [(80, 20000), (10000, 300)] {
            let text = "a".repeat(len);
            let glyphs = (0..len).map(|i| (i, i as f32 * 8.0)).collect::<Vec<_>>();
            let native = line(&text, &glyphs, len as f32 * 8.0)
                .fragments
                .remove(0)
                .layout;
            for width in [None, Some(560.0)] {
                let started = Instant::now();
                for _ in 0..iterations {
                    black_box(
                        ltr_fragments(black_box(&text), native.clone(), width, 20.0).unwrap(),
                    );
                }
                println!(
                    "ltr_fragment_benchmark len={len} width={width:?} iterations={iterations} ns_per_line={:.0}",
                    started.elapsed().as_nanos() as f64 / iterations as f64
                );
            }
        }
    }

    #[test]
    fn rtl_hit_testing_uses_visual_edges_with_logical_offsets() {
        let line = line("aאבz", &[(0, 0.0), (3, 10.0), (1, 20.0), (5, 30.0)], 40.0);
        assert_eq!(line.hit(11.0, 5.0), Some(5));
        assert_eq!(line.hit(19.0, 5.0), Some(3));
        assert_eq!(line.hit(21.0, 5.0), Some(3));
        assert_eq!(line.hit(29.0, 5.0), Some(1));
    }

    #[test]
    fn mixed_direction_selection_can_have_disjoint_visual_rectangles() {
        let line = line("aאבz", &[(0, 0.0), (3, 10.0), (1, 20.0), (5, 30.0)], 40.0);
        let rects = line.rectangles(0..3); // Select 'a' and alef, but not bet.
        let intervals: Vec<_> = rects
            .iter()
            .map(|r| (f32::from(r.origin.x), f32::from(r.size.width)))
            .collect();
        assert_eq!(intervals, vec![(0.0, 10.0), (20.0, 10.0)]);
    }

    #[test]
    fn singleton_rtl_cluster_has_reversed_caret_edges() {
        let line = line("א", &[(0, 0.0)], 10.0);
        assert_eq!(line.hit(1.0, 5.0), Some(2));
        assert_eq!(line.hit(9.0, 5.0), Some(0));
    }
    #[test]
    fn invisible_direction_controls_do_not_select_adjacent_ink() {
        let line = line("a\u{202e}b\u{202c}", &[(0, 0.0), (4, 10.0)], 20.0);
        assert!(line.rectangles(1..4).is_empty());
        assert!(line.rectangles(5..8).is_empty());
        assert_eq!(line.rectangles(4..5).len(), 1);
    }
}
