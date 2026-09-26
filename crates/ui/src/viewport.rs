//! Native viewport with variable row heights anchored to source locations.
//! Row numbers can change; a source point and viewport offset survive remeasurement.
//! GPUI measures and paints text. Prefix height estimates serve the scrollbar only.
mod layout;
mod navigation;
mod paint;
use crate::{
    native_text::{NativeLine, label_width, paint_label},
    theme::Skin,
};
use diffz_core::{
    anchor::ViewportAnchor,
    domain::*,
    height_index::HeightIndex,
    patch::RowKind,
    presentation::{self, Cell, DisplayRow},
    syntax::Span,
};
use gpui_kit::*;
use std::{collections::HashMap, ops::Range, sync::Arc};

const PAD: f32 = 1.0;
pub(crate) const BAR: f32 = 12.0;

/// Top offset and height of a vertical scrollbar thumb inside a track of `view_height`,
/// for `content_height` of content scrolled `scrolled` pixels down. Shared by the source
/// view and the rich view so both bars read the same.
pub(crate) fn vertical_thumb(view_height: f32, content_height: f32, scrolled: f32) -> (f32, f32) {
    let total = content_height.max(view_height);
    let thumb = (view_height * view_height / total.max(1.0)).clamp(24.0, view_height.max(24.0));
    let top = if total > view_height {
        (scrolled / (total - view_height)).clamp(0.0, 1.0) * (view_height - thumb).max(0.0)
    } else {
        0.0
    };
    (top, thumb)
}
#[derive(Clone)]
pub struct MeasuredCell {
    pub cell: Cell,
    pub side: Side,
    pub x: f32,
    pub width: f32,
    pub gutter: f32,
    pub native: std::result::Result<Arc<NativeLine>, String>,
}
#[derive(Clone)]
pub struct MeasuredRow {
    pub cells: Vec<MeasuredCell>,
    pub label: Option<String>,
    pub height: f32,
    pub unified: bool,
    pub old_number: Option<u32>,
    /// Height of the context-expansion band beneath a hunk header; zero means it is empty.
    pub band: f32,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ContextRequest {
    /// Request up to ten hidden lines immediately above the hunk.
    Above,
    /// Request up to ten hidden lines immediately below the hunk.
    Below,
    /// Request all hidden lines between this hunk and its predecessor or file start.
    All,
}
impl ContextRequest {
    pub fn above(self) -> bool {
        !matches!(self, Self::Below)
    }
}
/// Clickable context-band text for a hunk, located in viewport coordinates.
#[derive(Clone)]
pub struct ContextControl {
    pub row: usize,
    pub request: ContextRequest,
    pub label: String,
    pub x: Range<f32>,
}
const CONTEXT_BAND: f32 = 24.0;
const CONTEXT_STEP: u32 = 10;
const CONTEXT_BUDGET: u32 = 1000;
const CONTEXT_FONT: f32 = 11.0;
#[derive(Clone)]
pub struct PositionedRow {
    pub index: usize,
    pub y: f32,
    pub row: Arc<MeasuredRow>,
}
#[derive(Clone)]
pub struct Frame {
    pub horizontal_track: Bounds<Pixels>,
    pub horizontal_thumb: Bounds<Pixels>,
    pub bounds: Bounds<Pixels>,
    pub rows: Vec<PositionedRow>,
    pub horizontal: f32,
    pub thumb: Bounds<Pixels>,
    pub context_controls: Vec<ContextControl>,
}
#[derive(Clone, Copy, Default)]
struct Cursor {
    row: usize,
    offset: f32,
}
impl Cursor {
    fn needs_bottom_fill(self, end_y: f32, height: f32) -> bool {
        end_y < height && (self.row > 0 || self.offset > 0.)
    }
}
pub type Decorations = HashMap<(Side, u32), Vec<Span>>;
/// Horizontal padding that keeps a revealed match away from the text boundary.
const REVEAL_MARGIN: f32 = 24.0;
/// Find the smallest change to `current` that places `x1..x2` in the visible-width
/// window. If the range is wider than the window, preserve its starting edge.
fn reveal_horizontal(current: f32, x1: f32, x2: f32, visible: f32) -> f32 {
    let mut h = current.max(0.0);
    if x2 + REVEAL_MARGIN > h + visible {
        h = x2 + REVEAL_MARGIN - visible;
    }
    if x1 - REVEAL_MARGIN < h {
        h = x1 - REVEAL_MARGIN;
    }
    h.max(0.0)
}
pub struct Viewport {
    pub snapshot: Arc<Snapshot>,
    pub file: FileId,
    pub rows: Arc<Vec<DisplayRow>>,
    pub split: bool,
    pub wrap: bool,
    pub font_size: f32,
    pub family: String,
    pub selection: Option<SourceSelection>,
    pub active_search: Option<SourceSelection>,
    pub decorations: Arc<Decorations>,
    pub anchor: Option<ViewportAnchor>,
    pub last: Option<Frame>,
    pub expanded: HashMap<usize, (u32, u32)>,
    pub comment_lines: std::collections::HashSet<(Side, u32)>,
    pub draft_lines: std::collections::HashSet<(Side, u32)>,
    pub scrollbars_visible: bool,
    pub horizontal: f32,
    pub max_horizontal: f32,
    cursor: Cursor,
    hunk_target: Option<usize>,
    width: f32,
    height: f32,
    dirty: bool,
    pending_anchor: bool,
    pending_scroll: f32,
    cache: HashMap<usize, Arc<MeasuredRow>>,
    cache_bytes: usize,
    pub hovered: Option<SourcePoint>,
    pub annotations: HashMap<(Side, u32), diffz_core::annotation::Severity>,
    pub hover_cursor: CursorStyle,
    /// Edge being pulled to turn the file (`0` for none) and how far, 0 to 1.
    pub pull: (i8, f32),
    height_index: HeightIndex,
    digits: usize,
    /// Display-row positions of hunk headers, kept in sync when rows change.
    hunk_rows: Vec<usize>,
    /// Byte range on the anchored line to reveal horizontally; `restore_anchor` clears it.
    focus: Option<Range<usize>>,
}
fn hunk_rows(rows: &[DisplayRow]) -> Vec<usize> {
    rows.iter()
        .enumerate()
        .filter_map(|(i, row)| matches!(row, DisplayRow::Hunk { .. }).then_some(i))
        .collect()
}
impl Viewport {
    pub fn new(
        snapshot: Arc<Snapshot>,
        file: FileId,
        split: bool,
        wrap: bool,
        font_size: f32,
        family: String,
        anchor: Option<ViewportAnchor>,
    ) -> Self {
        let rows = Arc::new(
            snapshot
                .file(&file)
                .map_or_else(Vec::new, |f| presentation::rows(f, split)),
        );
        let digits = snapshot.file(&file).map_or(3, |f| {
            f.hunks
                .iter()
                .flat_map(|h| h.rows.iter().flat_map(|r| [r.old_line, r.new_line]))
                .flatten()
                .max()
                .unwrap_or(1)
                .to_string()
                .len()
                .max(3)
        });
        let path = snapshot
            .file(&file)
            .map(|f| f.display_path())
            .unwrap_or_default();
        let comment_lines = snapshot
            .comments
            .iter()
            .filter(|c| c.path == path)
            .filter_map(|c| Some((c.side?, c.line?)))
            .collect();
        Self {
            expanded: HashMap::new(),
            comment_lines,
            draft_lines: Default::default(),
            digits,
            hunk_rows: hunk_rows(&rows),
            height_index: HeightIndex::new(rows.len(), 28.0),
            snapshot,
            file,
            rows,
            split,
            wrap,
            font_size,
            family,
            selection: None,
            active_search: None,
            decorations: Arc::new(HashMap::new()),
            anchor,
            last: None,
            horizontal: 0.0,
            scrollbars_visible: false,
            max_horizontal: 0.0,
            cursor: Cursor::default(),
            hunk_target: None,
            width: 0.0,
            height: 0.0,
            dirty: true,
            pending_anchor: false,
            focus: None,
            pending_scroll: 0.0,
            cache: HashMap::new(),
            cache_bytes: 0,
            hovered: None,
            annotations: HashMap::new(),
            hover_cursor: CursorStyle::Arrow,
            pull: (0, 0.),
        }
    }
    pub fn draft_selection(&self) -> Option<SourceSelection> {
        self.selection.clone()
    }
    pub fn select_at(&mut self, position: Point<Pixels>) -> Option<SourcePoint> {
        self.hunk_target = None;
        let point = self.hit(position);
        self.selection = point.as_ref().map(|point| SourceSelection {
            start: point.clone(),
            end: point.clone(),
        });
        point
    }
    pub fn configure(&mut self, split: bool, wrap: bool, font_size: f32) {
        let font_size = font_size.clamp(10.0, 30.0);
        if self.split == split && self.wrap == wrap && (self.font_size - font_size).abs() < 0.01 {
            return;
        }
        if split != self.split {
            self.expanded.clear();
            self.hunk_target = None;
            self.rows = Arc::new(
                self.snapshot
                    .file(&self.file)
                    .map_or_else(Vec::new, |f| presentation::rows(f, split)),
            );
            self.hunk_rows = hunk_rows(&self.rows);
        }
        if wrap && !self.wrap {
            self.horizontal = 0.0;
        }
        self.split = split;
        self.wrap = wrap;
        self.font_size = font_size;
        self.dirty = true;
        self.last = None;
    }
    pub fn scroll(&mut self, dx: f32, dy: f32) {
        if dy != 0. {
            self.hunk_target = None;
        }
        if dx.is_finite() {
            self.horizontal = (self.horizontal + dx).clamp(0.0, self.max_horizontal.max(0.0));
        }
        if dy.is_finite() {
            self.pending_scroll = (self.pending_scroll + dy).clamp(-1_000_000.0, 1_000_000.0);
        }
    }
    pub fn jump_fraction(&mut self, fraction: f32) {
        self.hunk_target = None;
        let (row, offset) = self
            .height_index
            .locate(fraction.clamp(0.0, 1.0) * (self.height_index.total() - self.height).max(0.0));
        self.cursor = Cursor { row, offset };
        self.anchor = None;
        self.pending_scroll = 0.0;
    }
    pub fn reveal(&mut self, point: SourcePoint) {
        self.hunk_target = None;
        self.focus = None;
        if point.snapshot != self.snapshot.id || point.file != self.file {
            return;
        }
        self.anchor = Some(ViewportAnchor {
            point,
            viewport_y: self.font_size * 3.0,
            horizontal: self.horizontal,
        });
        self.pending_anchor = true;
        self.pending_scroll = 0.0;
    }
    /// Reveal `start` and scroll horizontally until its byte range fits.
    pub fn reveal_range(&mut self, start: SourcePoint, end_byte: usize) {
        let range = start.byte_column..end_byte.max(start.byte_column);
        self.reveal(start);
        if self.anchor.is_some() {
            self.focus = Some(range);
        }
    }
    /// Start a newly selected file at its first hunk, ignoring any saved anchor.
    pub fn jump_first_hunk(&mut self) {
        self.hunk_target = self.hunk_rows.first().copied();
        self.cursor = self
            .hunk_target
            .map_or_else(Cursor::default, |row| Cursor { row, offset: 0. });
        self.anchor = None;
        self.pending_anchor = false;
        self.pending_scroll = 0.;
        self.focus = None;
    }
    pub fn next_hunk(&mut self, forward: bool) -> Option<(usize, usize)> {
        let current = self.hunk_target.unwrap_or(self.cursor.row);
        let index = if forward {
            self.hunk_rows.partition_point(|&row| row <= current)
        } else {
            self.hunk_rows
                .partition_point(|&row| row < current)
                .checked_sub(1)?
        };
        let target = *self.hunk_rows.get(index)?;
        self.hunk_target = Some(target);
        self.cursor = Cursor {
            row: target,
            offset: 0.,
        };
        self.anchor = None;
        self.pending_scroll = 0.;
        Some((index + 1, self.hunk_rows.len()))
    }
    pub fn hit(&self, position: Point<Pixels>) -> Option<SourcePoint> {
        let frame = self.last.as_ref()?;
        if !frame.bounds.contains(&position)
            || self.hit_scrollbar(position).is_some()
            || self.hit_horizontal_scrollbar(position).is_some()
        {
            return None;
        }
        let x = f32::from(position.x - frame.bounds.left());
        let y = f32::from(position.y - frame.bounds.top());
        for p in &frame.rows {
            if y < p.y || y >= p.y + p.row.height {
                continue;
            }
            for cell in &p.row.cells {
                if x < cell.x || x >= cell.x + cell.width {
                    continue;
                }
                let line = cell.native.as_ref().ok()?;
                let cy = y - p.y - PAD;
                if cy < 0.0 || cy >= line.height {
                    return None;
                }
                let byte = if x < cell.x + cell.gutter {
                    line.source_at_fragment((cy / line.line_height) as usize)
                } else {
                    line.hit(x - cell.x - cell.gutter + frame.horizontal, cy)?
                };
                return Some(SourcePoint {
                    snapshot: self.snapshot.id.clone(),
                    file: self.file.clone(),
                    side: cell.side,
                    line: cell.cell.number,
                    byte_column: byte,
                });
            }
        }
        None
    }
    pub fn line_number_hit(
        &self,
        position: Point<Pixels>,
        window: &mut Window,
    ) -> Option<SourcePoint> {
        let mut point = self.hit(position)?;
        let frame = self.last.as_ref()?;
        let x = f32::from(position.x - frame.bounds.left());
        let row = frame.rows.iter().find(|r| {
            position.y >= frame.bounds.top() + px(r.y)
                && position.y < frame.bounds.top() + px(r.y + r.row.height)
        })?;
        let cell = row
            .row
            .cells
            .iter()
            .find(|c| c.side == point.side && c.cell.number == point.line)?;
        if x >= cell.x + cell.gutter - 25. {
            return None;
        }
        if row.row.unified {
            let old_end = cell.x
                + 8.
                + label_width(
                    &"0".repeat(self.digits + 1),
                    self.font_size,
                    &self.family,
                    window,
                );
            if x < old_end {
                point.side = Side::Left;
                point.line = row.row.old_number?;
            } else if cell.side != Side::Right {
                return None;
            }
        }
        point.byte_column = 0;
        Some(point)
    }
    pub fn select_source_line(&mut self, point: SourcePoint) {
        let mut end = point.clone();
        end.byte_column = self
            .snapshot
            .file(&self.file)
            .and_then(|f| f.line(point.side, point.line))
            .map_or(0, |r| r.text.len());
        self.selection = Some(SourceSelection { start: point, end });
        self.hunk_target = None;
    }
    pub fn hit_horizontal_scrollbar(&self, position: Point<Pixels>) -> Option<f32> {
        let f = self.last.as_ref()?;
        if self.scrollbars_visible
            && self.max_horizontal > 0.0
            && f.horizontal_track.contains(&position)
        {
            Some(
                (f32::from(position.x - f.horizontal_track.left())
                    / f32::from(f.horizontal_track.size.width).max(1.0))
                .clamp(0.0, 1.0),
            )
        } else {
            None
        }
    }
    pub fn hit_scrollbar(&self, position: Point<Pixels>) -> Option<f32> {
        let f = self.last.as_ref()?;
        if self.scrollbars_visible
            && self.height_index.total() > self.height
            && f.bounds.contains(&position)
            && position.x >= f.bounds.right() - px(BAR)
        {
            Some(
                (f32::from(position.y - f.bounds.top()) / f32::from(f.bounds.size.height).max(1.0))
                    .clamp(0.0, 1.0),
            )
        } else {
            None
        }
    }
    fn selection_range(
        &self,
        cell: &MeasuredCell,
        unified: bool,
        old_number: Option<u32>,
    ) -> Option<Range<usize>> {
        let s = self.selection.as_ref()?;
        if s.start.snapshot != self.snapshot.id || s.start.file != self.file {
            return None;
        }
        let line = if s.start.side == cell.side {
            cell.cell.number
        } else if unified && s.start.side == Side::Left && cell.cell.kind == RowKind::Context {
            old_number?
        } else {
            return None;
        };
        let (a, b) = if (s.start.line, s.start.byte_column) <= (s.end.line, s.end.byte_column) {
            (&s.start, &s.end)
        } else {
            (&s.end, &s.start)
        };
        if line < a.line || line > b.line {
            return None;
        }
        let lo = if line == a.line { a.byte_column } else { 0 };
        let hi = if line == b.line {
            b.byte_column
        } else {
            cell.cell.text.len()
        };
        (lo < hi).then_some(lo..hi)
    }
}

impl MeasuredRow {
    fn estimated_bytes(&self) -> usize {
        std::mem::size_of::<Self>()
            + self
                .cells
                .iter()
                .map(|c| c.cell.text.len() + c.native.as_ref().map_or(0, |n| n.estimated_bytes()))
                .sum::<usize>()
    }
}

#[derive(Clone, Copy)]
pub enum Motion {
    Up,
    Down,
    Left,
    Right,
    Home,
    End,
    First,
    Last,
}

#[cfg(test)]
mod hunk_navigation_tests {
    use super::{Cursor, Viewport};
    use diffz_core::{
        domain::{Side, Snapshot},
        presentation::DisplayRow,
    };
    use std::sync::Arc;
    #[test]
    fn hunk_navigation_advances_even_when_layout_clamps_the_top_row() {
        let snapshot = Arc::new(Snapshot::new(
            "test".into(),
            diffz_core::patch::parse_patch(
                include_bytes!("../../../fixtures/split-asymmetric/change.patch"),
                Default::default(),
            )
            .unwrap(),
            None,
            vec![],
        ));
        let file = snapshot.patch.files[0].id.clone();
        let mut v = Viewport::new(snapshot, file, false, false, 14., "Menlo".into(), None);
        let hunk = || DisplayRow::Hunk {
            label: "@@".into(),
            source_line: 1,
            side: Side::Right,
        };
        v.rows = Arc::new(vec![
            hunk(),
            DisplayRow::Notice("context".into()),
            hunk(),
            DisplayRow::Notice("context".into()),
            hunk(),
        ]);
        v.hunk_rows = super::hunk_rows(&v.rows);
        assert_eq!(v.next_hunk(true), Some((2, 3)));
        v.cursor = Cursor::default(); // A short file remains scrolled to the top.
        assert_eq!(v.next_hunk(true), Some((3, 3)));
        v.cursor = Cursor::default();
        assert_eq!(v.next_hunk(true), None);
        assert_eq!(v.next_hunk(false), Some((2, 3)));
        v.cursor = Cursor::default();
        assert_eq!(v.next_hunk(false), Some((1, 3)));
        assert_eq!(v.next_hunk(false), None);
        v.scroll(0., 20.);
        assert_eq!(v.next_hunk(true), Some((2, 3)));
    }
}

#[cfg(test)]
mod first_hunk_reset_tests {
    use super::{Cursor, Viewport};
    use diffz_core::{
        anchor::ViewportAnchor,
        domain::{Side, Snapshot, SourcePoint},
        patch::parse_patch,
    };
    use std::sync::Arc;

    fn viewport_with_hunks() -> Viewport {
        let snapshot = Arc::new(
            Snapshot::new(
                "test".into(),
                parse_patch(
                    b"diff --git a/a.rs b/a.rs\n--- a/a.rs\n+++ b/a.rs\n@@ -1,1 +1,1 @@\n-old\n+new\n@@ -5,1 +5,1 @@\n-old2\n+new2\n",
                    Default::default(),
                )
                .unwrap(),
                None,
                vec![],
            ),
        );
        let file = snapshot.patch.files[0].id.clone();
        Viewport::new(snapshot, file, false, false, 14., "Menlo".into(), None)
    }

    #[test]
    fn reset_jumps_to_first_hunk_and_discards_stale_anchor_state() {
        let mut v = viewport_with_hunks();
        let first = v.hunk_rows[0];
        let second = v.hunk_rows[1];
        v.cursor = Cursor {
            row: second,
            offset: 19.,
        };
        v.hunk_target = Some(second);
        v.anchor = Some(ViewportAnchor {
            point: SourcePoint {
                snapshot: v.snapshot.id.clone(),
                file: v.file.clone(),
                side: Side::Right,
                line: 5,
                byte_column: 0,
            },
            viewport_y: 42.,
            horizontal: 77.,
        });
        v.pending_anchor = true;
        v.pending_scroll = 123.;

        v.jump_first_hunk();

        assert_eq!(v.cursor.row, first);
        assert_eq!(v.cursor.offset, 0.);
        assert_eq!(v.hunk_target, Some(first));
        assert!(v.anchor.is_none());
        assert!(!v.pending_anchor);
        assert_eq!(v.pending_scroll, 0.);
    }

    #[test]
    fn reset_falls_back_to_the_top_when_no_hunk_exists() {
        let mut v = viewport_with_hunks();
        v.rows = Arc::new(vec![diffz_core::presentation::DisplayRow::Notice(
            "plain file".into(),
        )]);
        v.hunk_rows.clear();
        v.cursor = Cursor {
            row: 12,
            offset: 9.,
        };
        v.hunk_target = Some(12);
        v.anchor = Some(ViewportAnchor {
            point: SourcePoint {
                snapshot: v.snapshot.id.clone(),
                file: v.file.clone(),
                side: Side::Right,
                line: 1,
                byte_column: 0,
            },
            viewport_y: 42.,
            horizontal: 77.,
        });
        v.pending_anchor = true;
        v.pending_scroll = 123.;

        v.jump_first_hunk();

        assert_eq!(v.cursor.row, 0);
        assert_eq!(v.cursor.offset, 0.);
        assert!(v.hunk_target.is_none());
        assert!(v.anchor.is_none());
        assert!(!v.pending_anchor);
        assert_eq!(v.pending_scroll, 0.);
    }
}

#[cfg(test)]
mod thumb_tests {
    use super::vertical_thumb;
    #[test]
    fn the_thumb_shrinks_with_the_content_and_travels_the_whole_track() {
        assert_eq!(vertical_thumb(600., 1_200., 0.), (0., 300.));
        assert_eq!(vertical_thumb(600., 1_200., 600.), (300., 300.));
        assert_eq!(vertical_thumb(600., 1_200., 300.), (150., 300.));
    }

    #[test]
    fn content_that_fits_parks_a_full_height_thumb_at_the_top() {
        assert_eq!(vertical_thumb(600., 200., 0.), (0., 600.));
        assert_eq!(vertical_thumb(600., 600., 900.), (0., 600.));
    }

    #[test]
    fn a_very_long_file_keeps_a_grabbable_thumb() {
        let (top, height) = vertical_thumb(600., 1_000_000., 1_000_000.);
        assert_eq!(height, 24.);
        assert_eq!(top, 576.);
    }
}

#[cfg(test)]
mod short_scroll_tests {
    use super::Cursor;
    #[test]
    fn fractional_scroll_on_first_row_is_clamped_when_file_fits() {
        for offset in [0.5, 2., 8., 18.] {
            let cursor = Cursor { row: 0, offset };
            assert!(cursor.needs_bottom_fill(180. - offset, 600.));
        }
        assert!(!Cursor::default().needs_bottom_fill(180., 600.));
        assert!(!Cursor { row: 0, offset: 8. }.needs_bottom_fill(800., 600.));
    }
}

#[cfg(test)]
mod context_tests {
    use super::{ContextRequest, Viewport};
    use diffz_core::{domain::Snapshot, patch::parse_patch};
    use std::sync::Arc;
    fn viewport(patch: &[u8]) -> Viewport {
        let snapshot = Arc::new(Snapshot::new(
            "test".into(),
            parse_patch(patch, Default::default()).unwrap(),
            None,
            vec![],
        ));
        let file = snapshot.patch.files[0].id.clone();
        Viewport::new(snapshot, file, false, false, 14., "Menlo".into(), None)
    }
    // Isolated context-control CPU cost, excluding native layout and rendering.
    // Run with: cargo test -p diffz-ui --release --lib benchmark_context_controls -- --ignored --nocapture
    #[test]
    #[ignore]
    fn benchmark_context_controls() {
        use std::{hint::black_box, time::Instant};
        let mut patch = String::from(
            "diff --git a/a.rs b/a.rs\n--- a/a.rs\n+++ b/a.rs\n@@ -1,100000 +1,100000 @@\n",
        );
        patch.push_str(&" context\n".repeat(100000));
        patch.push_str("@@ -110000,1 +110000,1 @@\n-old\n+new\n");
        let v = viewport(patch.as_bytes());
        let started = Instant::now();
        let iterations = 1000;
        for _ in 0..iterations {
            black_box(v.context_labels(black_box(100001)));
        }
        println!(
            "context_controls_benchmark rows={} iterations={iterations} ns_per_lookup={:.0}",
            v.rows.len(),
            started.elapsed().as_nanos() as f64 / iterations as f64
        );
    }

    #[test]
    fn context_controls_follow_hunk_rows_after_split_and_expansion() {
        let mut v = viewport(b"diff --git a/a.rs b/a.rs\n--- a/a.rs\n+++ b/a.rs\n@@ -20,1 +20,1 @@\n-old\n+new\n@@ -35,1 +35,1 @@\n-old2\n+new2\n");
        for split in [false, true, false] {
            v.configure(split, false, 14.0);
            v.insert_context(0, true, 17, 17, vec!["context".into(); 3]);
            let second = v
                .rows
                .iter()
                .position(|r| {
                    matches!(
                        r,
                        diffz_core::presentation::DisplayRow::Hunk {
                            source_line: 35,
                            ..
                        }
                    )
                })
                .unwrap();
            assert_eq!(
                v.plan_for_row(second, ContextRequest::Above),
                Some((1, 25, 25, 10))
            );
            assert_eq!(
                v.plan_for_row(second + 1, ContextRequest::All),
                Some((1, 21, 21, 14))
            );
            v.hunk_target = Some(second);
            assert_eq!(v.next_hunk(false), Some((1, 2)));
            assert_eq!(v.next_hunk(true), Some((2, 2)));
        }
    }

    #[test]
    fn revealing_a_source_keeps_geometry_cached_until_resize() {
        use super::MeasuredRow;
        use diffz_core::domain::{Side, SourcePoint};
        let mut v = viewport(
            b"diff --git a/a.rs b/a.rs\n--- a/a.rs\n+++ b/a.rs\n@@ -1,1 +1,1 @@\n-old\n+new\n",
        );
        v.width = 800.0;
        v.dirty = false;
        let measured = Arc::new(MeasuredRow {
            cells: vec![],
            label: Some("measured".into()),
            height: 60.0,
            unified: true,
            old_number: None,
            band: 0.0,
        });
        v.cache.insert(0, measured.clone());
        v.cache_bytes = measured.estimated_bytes();
        v.height_index.update(0, 60.0).unwrap();
        v.max_horizontal = 123.0;
        v.reveal(SourcePoint {
            snapshot: v.snapshot.id.clone(),
            file: v.file.clone(),
            side: Side::Right,
            line: 1,
            byte_column: 0,
        });
        assert!(!v.prepare_geometry(800.0));
        assert!(Arc::ptr_eq(v.cache.get(&0).unwrap(), &measured));
        assert_eq!(v.cache_bytes, measured.estimated_bytes());
        assert_eq!(v.height_index.prefix(1), 60.0);
        assert_eq!(v.max_horizontal, 123.0);
        assert!(v.prepare_geometry(801.0));
        assert!(v.cache.is_empty());
        assert_eq!(v.cache_bytes, 0);
        assert_eq!(v.max_horizontal, 0.0);
    }

    #[test]
    fn controls_are_line_aware() {
        // An additions-only file hides no context above or below.
        let v = viewport(
            b"diff --git a/a.rs b/a.rs\n--- /dev/null\n+++ b/a.rs\n@@ -0,0 +1,3 @@\n+a\n+b\n+c\n",
        );
        assert!(v.context_labels(0).is_empty());
        // At the file start, the upper gap is empty and the lower gap may be unknown.
        let v = viewport(b"diff --git a/a.rs b/a.rs\n--- a/a.rs\n+++ b/a.rs\n@@ -1,2 +1,2 @@\n-old\n+new\n ctx\n");
        assert_eq!(v.context_plan(ContextRequest::Above), None);
        assert_eq!(v.context_plan(ContextRequest::All), None);
        assert_eq!(v.context_plan(ContextRequest::Below), Some((0, 3, 3, 10)));
        assert_eq!(
            v.context_labels(0),
            vec![(ContextRequest::Below, "↓ Show 10 below".to_owned())]
        );
        // Short gaps display their count and do not need a separate "all" action.
        let v = viewport(
            b"diff --git a/a.rs b/a.rs\n--- a/a.rs\n+++ b/a.rs\n@@ -4,1 +4,1 @@\n-old\n+new\n",
        );
        assert_eq!(v.context_plan(ContextRequest::Above), Some((0, 1, 1, 3)));
        assert_eq!(v.context_plan(ContextRequest::All), None);
    }
    #[test]
    fn show_all_fills_the_gap_above() {
        let mut v = viewport(b"diff --git a/a.rs b/a.rs\n--- a/a.rs\n+++ b/a.rs\n@@ -20,1 +20,1 @@\n-old\n+new\n@@ -35,1 +35,1 @@\n-old2\n+new2\n");
        assert_eq!(v.context_plan(ContextRequest::All), Some((0, 1, 1, 19)));
        let labels: Vec<_> = v.context_labels(0).into_iter().map(|(r, _)| r).collect();
        assert_eq!(
            labels,
            vec![
                ContextRequest::Above,
                ContextRequest::All,
                ContextRequest::Below
            ]
        );
        v.insert_context(
            0,
            true,
            1,
            1,
            (1..20).map(|i| format!("line {i}")).collect(),
        );
        assert_eq!(v.context_plan(ContextRequest::Above), None);
        assert_eq!(v.context_plan(ContextRequest::All), None);
        let second_header = v
            .rows
            .iter()
            .enumerate()
            .filter(|(_, r)| matches!(r, diffz_core::presentation::DisplayRow::Hunk { .. }))
            .nth(1)
            .map(|(i, _)| i)
            .unwrap();
        v.hunk_target = Some(second_header);
        assert_eq!(v.context_plan(ContextRequest::All), Some((1, 21, 21, 14)));
        assert_eq!(v.context_plan(ContextRequest::Above), Some((1, 25, 25, 10)));
    }
    #[test]
    fn expansion_preserves_coordinates_and_stops_at_neighbor() {
        let mut v = viewport(b"diff --git a/a.rs b/a.rs\n--- a/a.rs\n+++ b/a.rs\n@@ -20,1 +20,1 @@\n-old\n+new\n@@ -35,1 +35,1 @@\n-old2\n+new2\n");
        assert_eq!(v.context_plan(ContextRequest::Above), Some((0, 10, 10, 10)));
        v.insert_context(
            0,
            true,
            10,
            10,
            (10..20).map(|i| format!("line {i}")).collect(),
        );
        assert_eq!(v.context_plan(ContextRequest::Above), Some((0, 1, 1, 9)));
        assert_eq!(v.context_plan(ContextRequest::Below), Some((0, 21, 21, 10)));
        v.insert_context(
            0,
            false,
            21,
            21,
            (21..31).map(|i| format!("line {i}")).collect(),
        );
        assert_eq!(v.context_plan(ContextRequest::Below), Some((0, 31, 31, 4)));
        v.insert_context(
            0,
            false,
            31,
            31,
            (31..35).map(|i| format!("line {i}")).collect(),
        );
        assert_eq!(v.context_plan(ContextRequest::Below), None);
        assert!(
            v.snapshot.patch.files[0]
                .line(diffz_core::domain::Side::Right, 10)
                .is_none()
        );
        v.configure(true, false, 14.);
        assert!(v.expanded.is_empty());
    }
}

#[cfg(test)]
mod reveal_horizontal_tests {
    use super::reveal_horizontal;
    #[test]
    fn match_already_visible_keeps_scroll() {
        assert_eq!(reveal_horizontal(0.0, 100.0, 140.0, 800.0), 0.0);
        assert_eq!(reveal_horizontal(300.0, 400.0, 440.0, 800.0), 300.0);
    }
    #[test]
    fn match_past_right_edge_scrolls_right_with_margin() {
        assert_eq!(
            reveal_horizontal(0.0, 2000.0, 2050.0, 800.0),
            2050.0 + 24.0 - 800.0
        );
    }
    #[test]
    fn match_before_left_edge_scrolls_left_with_margin() {
        assert_eq!(reveal_horizontal(1500.0, 100.0, 140.0, 800.0), 76.0);
        assert_eq!(reveal_horizontal(1500.0, 10.0, 40.0, 800.0), 0.0);
    }
    #[test]
    fn match_wider_than_window_shows_its_start() {
        assert_eq!(reveal_horizontal(0.0, 1000.0, 3000.0, 800.0), 976.0);
    }
}
