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

#[derive(Clone)]
pub struct Fragment {
    pub display: Range<usize>,
    pub layout: Arc<LineLayout>,
    pub y: f32,
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
fn run(len: usize, family: &str, color: Hsla) -> TextRun {
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
impl NativeLine {
    pub fn estimated_bytes(&self) -> usize {
        self.source.capacity()
            + self.display.estimated_bytes()
            + self.fragments.capacity() * std::mem::size_of::<Fragment>()
            + self
                .fragments
                .iter()
                .map(|f| {
                    std::mem::size_of::<LineLayout>()
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
        let layout = window.text_system().layout_line(
            &display.text,
            px(size),
            &[run(display.text.len(), family, rgb(0xffffff).into())],
            None,
        );
        let mut positions: BTreeMap<usize, f32> = BTreeMap::new();
        positions.insert(0, 0.0);
        for r in &layout.runs {
            for g in &r.glyphs {
                positions.entry(g.index).or_insert(f32::from(g.position.x));
            }
        }
        positions.insert(display.text.len(), f32::from(layout.width));
        let mut valid: HashSet<usize> = display
            .text
            .grapheme_indices(true)
            .map(|(i, _)| i)
            .collect();
        valid.insert(display.text.len());
        let clusters: Vec<_> = positions
            .into_iter()
            .filter(|(i, _)| valid.contains(i))
            .collect();
        if clusters.windows(2).any(|w| w[1].1 + 0.01 < w[0].1) {
            return Err(ShapeError("native glyph order needs bidi support; this line was neither reordered nor clipped".into()));
        }
        let preferred: HashSet<_> = unicode_linebreak::linebreaks(&display.text)
            .map(|(i, _)| i)
            .collect();
        let ranges = measured_breaks(&clusters, &preferred, wrap.then_some(width));
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
        let parts: Vec<_> = ranges
            .into_iter()
            .zip(layouts)
            .enumerate()
            .map(|(i, (display, layout))| Fragment {
                display,
                layout: Arc::new(layout),
                y: i as f32 * line_height,
            })
            .collect();
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
