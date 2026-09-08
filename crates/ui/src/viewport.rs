//! Native viewport with variable row heights anchored to source locations.
//! Row numbers can change; a source point and viewport offset survive remeasurement.
//! GPUI measures and paints text. Prefix height estimates serve the scrollbar only.
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
const BAR: f32 = 12.0;
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
    pub full_bounds: Bounds<Pixels>,
    pub horizontal_track: Bounds<Pixels>,
    pub horizontal_thumb: Bounds<Pixels>,
    pub bounds: Bounds<Pixels>,
    pub rows: Vec<PositionedRow>,
    pub horizontal: f32,
    pub thumb: Bounds<Pixels>,
    pub anchor: Option<ViewportAnchor>,
    pub error_count: usize,
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
    pub hover_cursor: CursorStyle,
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
            hover_cursor: CursorStyle::Arrow,
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
    fn measure(&mut self, index: usize, window: &mut Window) -> Arc<MeasuredRow> {
        if let Some(row) = self.cache.get(&index) {
            return row.clone();
        }
        let Some(source) = self.rows.get(index).cloned() else {
            return Arc::new(MeasuredRow {
                cells: vec![],
                label: None,
                height: 28.0,
                unified: true,
                old_number: None,
                band: 0.0,
            });
        };
        let lh = (self.font_size * 1.4).ceil();
        let measured = match source {
            DisplayRow::Hunk { label, .. } => {
                let band = if self.context_labels(index).is_empty() {
                    0.0
                } else {
                    CONTEXT_BAND
                };
                MeasuredRow {
                    cells: vec![],
                    label: Some(label),
                    height: lh + 4.0 + band,
                    unified: true,
                    old_number: None,
                    band,
                }
            }
            DisplayRow::Notice(label) => MeasuredRow {
                cells: vec![],
                label: Some(label),
                height: lh + 4.0,
                unified: true,
                old_number: None,
                band: 0.0,
            },
            DisplayRow::Line {
                left,
                right,
                unified,
                old_number,
            } => {
                let gutter = label_width(
                    &"9".repeat(self.digits),
                    self.font_size,
                    &self.family,
                    window,
                ) * if unified { 2.0 } else { 1.0 }
                    + 44.0;
                let col = if unified {
                    self.width
                } else {
                    (self.width - 1.0) / 2.0
                };
                let mut cells = vec![];
                let mut height = lh;
                for (side, cell) in [(Side::Left, left), (Side::Right, right)] {
                    if let Some(cell) = cell {
                        let native = NativeLine::shape(
                            &cell.text,
                            (col - gutter - 12.0).max(1.0),
                            self.font_size,
                            lh,
                            self.wrap,
                            &self.family,
                            window,
                        )
                        .map(Arc::new)
                        .map_err(|e| e.0);
                        if let Ok(n) = &native {
                            height = height.max(n.height);
                            self.max_horizontal = self
                                .max_horizontal
                                .max((n.width - (col - gutter - 12.0)).max(0.0));
                        }
                        let x = if side == Side::Right && !unified {
                            col + 1.0
                        } else {
                            0.0
                        };
                        cells.push(MeasuredCell {
                            cell,
                            side,
                            x,
                            width: col,
                            gutter,
                            native,
                        });
                    }
                }
                MeasuredRow {
                    cells,
                    label: None,
                    height: height + PAD * 2.0,
                    unified,
                    old_number,
                    band: 0.0,
                }
            }
        };
        let row = Arc::new(measured);
        let _ = self.height_index.update(index, row.height);
        let bytes = row.estimated_bytes();
        const CACHE_BUDGET: usize = 8 * 1024 * 1024;
        while !self.cache.is_empty()
            && (self.cache_bytes.saturating_add(bytes) > CACHE_BUDGET || self.cache.len() >= 128)
        {
            let farthest = *self
                .cache
                .keys()
                .max_by_key(|i| i.abs_diff(self.cursor.row))
                .unwrap();
            if let Some(old) = self.cache.remove(&farthest) {
                self.cache_bytes = self.cache_bytes.saturating_sub(old.estimated_bytes());
            }
        }
        if bytes <= CACHE_BUDGET {
            self.cache_bytes += bytes;
            self.cache.insert(index, row.clone());
        }
        row
    }
    fn normalize(&mut self, window: &mut Window) {
        if self.rows.is_empty() {
            self.cursor = Cursor::default();
            return;
        }
        self.cursor.row = self.cursor.row.min(self.rows.len() - 1);
        while self.cursor.offset < 0.0 && self.cursor.row > 0 {
            self.cursor.row -= 1;
            self.cursor.offset += self.measure(self.cursor.row, window).height;
        }
        self.cursor.offset = self.cursor.offset.max(0.0);
        let mut steps = 0;
        loop {
            let h = self.measure(self.cursor.row, window).height;
            if self.cursor.offset < h {
                break;
            }
            if self.cursor.row + 1 >= self.rows.len() {
                self.cursor.offset = (h - 1.0).max(0.0);
                break;
            }
            self.cursor.offset -= h;
            self.cursor.row += 1;
            steps += 1;
            if steps >= 5000 {
                self.pending_scroll += self.cursor.offset;
                self.cursor.offset = 0.0;
                break;
            }
        }
    }
    fn restore_anchor(&mut self, window: &mut Window) {
        let Some(anchor) = self.anchor.clone() else {
            return;
        };
        if anchor.point.snapshot != self.snapshot.id || anchor.point.file != self.file {
            return;
        }
        let (mut side, mut number) = (anchor.point.side, anchor.point.line);
        // Unified context paints one cell. LEFT coordinates map to the row's RIGHT
        // coordinates, while saved drafts and selections keep their original values.
        if !self.split
            && side == Side::Left
            && let Some(row) = self
                .snapshot
                .file(&self.file)
                .and_then(|f| f.line(side, number))
            && row.kind == RowKind::Context
            && let Some(new) = row.new_line
        {
            side = Side::Right;
            number = new;
        }
        if let Some(index) = DisplayRow::find_source(&self.rows, side, number) {
            let row = self.measure(index, window);
            if let Some(cell) = row.cells.iter().find(|c| c.side == side)
                && let Ok(line) = &cell.native
            {
                let f = line.fragment_for_source(anchor.point.byte_column);
                self.cursor = Cursor {
                    row: index,
                    offset: PAD + f as f32 * line.line_height - anchor.viewport_y,
                };
                self.horizontal = anchor.horizontal.max(0.0);
                if let Some(focus) = self.focus.take()
                    && let Some(rect) = line.rectangles(focus).into_iter().next()
                {
                    let x1 = f32::from(rect.left());
                    let x2 = f32::from(rect.right());
                    self.horizontal =
                        reveal_horizontal(self.horizontal, x1, x2, cell.width - cell.gutter - 4.0)
                            .min(self.max_horizontal.max(0.0));
                }
            }
        }
    }
    fn prepare_geometry(&mut self, width: f32) -> bool {
        if self.dirty || (width - self.width).abs() > 0.1 {
            self.width = width;
            self.cache.clear();
            self.cache_bytes = 0;
            self.height_index = HeightIndex::new(self.rows.len(), self.font_size * 1.4 + PAD * 2.0);
            self.max_horizontal = 0.0;
            return true;
        }
        false
    }
    pub fn frame(&mut self, bounds: Bounds<Pixels>, window: &mut Window) -> Frame {
        let full_bounds = bounds;
        let width = f32::from(bounds.size.width).max(1.0);
        self.height = f32::from(bounds.size.height);
        let geometry_changed = self.prepare_geometry(width);
        if geometry_changed || self.pending_anchor {
            self.restore_anchor(window);
            self.pending_anchor = false;
            self.dirty = false;
        }
        self.cursor.offset += std::mem::take(&mut self.pending_scroll);
        self.normalize(window);
        let mut rows = vec![];
        // Fill the lower edge in a second pass while ordinary remeasurement keeps the top anchor.
        for pass in 0..2 {
            rows.clear();
            let mut y = -self.cursor.offset;
            let mut i = self.cursor.row;
            while y < self.height && i < self.rows.len() {
                let row = self.measure(i, window);
                let h = row.height;
                rows.push(PositionedRow { index: i, y, row });
                y += h;
                i += 1;
            }
            if pass == 0 && i == self.rows.len() && self.cursor.needs_bottom_fill(y, self.height) {
                self.cursor.offset -= self.height - y;
                self.normalize(window);
                continue;
            }
            break;
        }
        let mut anchor = None;
        let mut errors = 0;
        'outer: for p in &rows {
            for cell in &p.row.cells {
                let Ok(line) = &cell.native else {
                    errors += 1;
                    continue;
                };
                let from = (-p.y - PAD).max(0.0);
                if from >= line.height {
                    continue;
                }
                let ix = (from / line.line_height).floor() as usize;
                anchor = Some(ViewportAnchor {
                    point: SourcePoint {
                        snapshot: self.snapshot.id.clone(),
                        file: self.file.clone(),
                        side: cell.side,
                        line: cell.cell.number,
                        byte_column: line.source_at_fragment(ix),
                    },
                    viewport_y: p.y + PAD + ix as f32 * line.line_height,
                    horizontal: self.horizontal,
                });
                break 'outer;
            }
        }
        self.anchor = anchor.clone();
        let mut context_controls = vec![];
        for p in rows.iter().filter(|p| p.row.band > 0.0) {
            let mut x = 16.0;
            for (request, label) in self.context_labels(p.index) {
                let w = label_width(&label, CONTEXT_FONT, &self.family, window);
                context_controls.push(ContextControl {
                    row: p.index,
                    request,
                    label,
                    x: x..x + w,
                });
                x += w + 28.0;
            }
        }
        let total = self.height_index.total().max(self.height);
        let thumb_h =
            (self.height * self.height / total.max(1.0)).clamp(24.0, self.height.max(24.0));
        let absolute = self.height_index.prefix(self.cursor.row) + self.cursor.offset;
        let thumb_y = if total > self.height {
            (absolute / (total - self.height)).clamp(0.0, 1.0) * (self.height - thumb_h).max(0.0)
        } else {
            0.0
        };
        let horizontal_track = Bounds::new(
            point(bounds.left(), bounds.bottom() - px(BAR)),
            size(px(width), px(BAR)),
        );
        let hwidth =
            (width * width / (width + self.max_horizontal).max(1.0)).clamp(20.0, width.max(20.0));
        let hx = if self.max_horizontal > 0.0 {
            self.horizontal / self.max_horizontal * (width - hwidth).max(0.0)
        } else {
            0.0
        };
        let horizontal_thumb = Bounds::new(
            point(bounds.left() + px(hx), bounds.bottom() - px(BAR - 2.0)),
            size(px(hwidth), px(BAR - 4.0)),
        );
        let frame = Frame {
            full_bounds,
            horizontal_track,
            horizontal_thumb,
            bounds,
            rows,
            horizontal: self.horizontal,
            thumb: Bounds::new(
                point(bounds.right() - px(BAR - 2.0), bounds.top() + px(thumb_y)),
                size(px(BAR - 4.0), px(thumb_h)),
            ),
            anchor,
            error_count: errors,
            context_controls,
        };
        self.last = Some(frame.clone());
        frame
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
    pub fn paint(&self, frame: &Frame, skin: Skin, window: &mut Window, cx: &mut App) {
        window.paint_quad(fill(frame.bounds, skin.base));
        for p in &frame.rows {
            let y = frame.bounds.top() + px(p.y);
            let row_bounds = Bounds::new(
                point(frame.bounds.left(), y),
                size(px(self.width), px(p.row.height)),
            );
            if let Some(label) = &p.row.label {
                window.paint_quad(fill(
                    row_bounds,
                    if self.hunk_target == Some(p.index) {
                        skin.selection
                    } else {
                        skin.surface
                    },
                ));
                window.with_content_mask(
                    Some(ContentMask {
                        bounds: row_bounds.intersect(&frame.bounds),
                    }),
                    |window| {
                        paint_label(
                            label,
                            point(
                                row_bounds.left() + px(10.0),
                                y + px(
                                    (p.row.height - p.row.band - (self.font_size - 1.0) * 1.4) / 2.
                                ),
                            ),
                            skin.muted,
                            self.font_size - 1.0,
                            &self.family,
                            window,
                            cx,
                        )
                    },
                );
                if p.row.band > 0.0 {
                    let band = Bounds::new(
                        point(frame.bounds.left(), y + px(p.row.height - p.row.band)),
                        size(px(self.width), px(p.row.band)),
                    );
                    window.paint_quad(fill(band, skin.base));
                    window.paint_quad(fill(band, skin.accent.opacity(0.07)));
                    window.paint_quad(fill(
                        Bounds::new(band.origin, size(px(self.width), px(1.0))),
                        skin.border,
                    ));
                    let text_y = band.top() + px((p.row.band - CONTEXT_FONT * 1.4) / 2.);
                    for c in frame.context_controls.iter().filter(|c| c.row == p.index) {
                        let origin = point(band.left() + px(c.x.start), text_y);
                        paint_label(
                            &c.label,
                            origin,
                            skin.accent,
                            CONTEXT_FONT,
                            &self.family,
                            window,
                            cx,
                        );
                    }
                }
                continue;
            }
            for cell in &p.row.cells {
                let x = frame.bounds.left() + px(cell.x);
                let cb = Bounds::new(point(x, y), size(px(cell.width), px(p.row.height)));
                let bg = match cell.cell.kind {
                    RowKind::Added => skin.added,
                    RowKind::Removed => skin.removed,
                    _ => skin.base,
                };
                window.paint_quad(fill(cb, bg));
                if self.selection.as_ref().is_some_and(|s| {
                    s.start == s.end && s.end.side == cell.side && s.end.line == cell.cell.number
                }) {
                    window.paint_quad(fill(cb, skin.accent.opacity(0.12)));
                }

                let number = if p.row.unified {
                    format!(
                        "{:>d$} {:>d$}",
                        p.row.old_number.map(|n| n.to_string()).unwrap_or_default(),
                        if cell.side == Side::Right {
                            cell.cell.number.to_string()
                        } else {
                            String::new()
                        },
                        d = self.digits
                    )
                } else {
                    cell.cell.number.to_string()
                };
                let hover = self
                    .hovered
                    .as_ref()
                    .is_some_and(|p| p.side == cell.side && p.line == cell.cell.number);
                let marker = if hover {
                    "+"
                } else {
                    match cell.cell.kind {
                        RowKind::Added => "+",
                        RowKind::Removed => "−",
                        RowKind::Context => " ",
                    }
                };
                window.with_content_mask(
                    Some(ContentMask {
                        bounds: cb.intersect(&frame.bounds),
                    }),
                    |window| {
                        paint_label(
                            &number,
                            point(x + px(8.0), y + px(PAD)),
                            skin.muted,
                            self.font_size,
                            &self.family,
                            window,
                            cx,
                        );
                        paint_label(
                            marker,
                            point(x + px(cell.gutter - 20.0), y + px(PAD)),
                            match cell.cell.kind {
                                RowKind::Added => skin.positive,
                                RowKind::Removed => skin.negative,
                                _ => skin.muted,
                            },
                            self.font_size,
                            &self.family,
                            window,
                            cx,
                        );
                    },
                );
                let has_comment = self.draft_lines.contains(&(cell.side, cell.cell.number))
                    || self.comment_lines.contains(&(cell.side, cell.cell.number))
                    || (p.row.unified
                        && p.row.old_number.is_some_and(|n| {
                            self.comment_lines.contains(&(Side::Left, n))
                                || self.draft_lines.contains(&(Side::Left, n))
                        }));
                let original_line = self
                    .snapshot
                    .file(&self.file)
                    .and_then(|f| f.line(cell.side, cell.cell.number))
                    .is_some();
                if (hover || has_comment) && original_line {
                    let fragment_y = cell.native.as_ref().map_or(0., |n| {
                        n.fragment_for_source(self.hovered.as_ref().map_or(0, |p| p.byte_column))
                            as f32
                            * n.line_height
                    });
                    let extent = 20.0_f32.min(self.font_size * 1.4);
                    let py = y + px(PAD + fragment_y + (self.font_size * 1.4 - extent) / 2.);
                    let button = Bounds::new(
                        point(x + px(cell.gutter - 24.), py),
                        size(px(20.), px(extent)),
                    );
                    window.paint_quad(fill(button, skin.accent).corner_radii(px(4.)));
                    let center = button.center();
                    if has_comment {
                        let bubble = Bounds::new(
                            point(center.x - px(5.), center.y - px(4.)),
                            size(px(10.), px(7.)),
                        );
                        window.paint_quad(fill(bubble, skin.base).corner_radii(px(2.)));
                        let inside = Bounds::new(
                            point(center.x - px(3.5), center.y - px(2.5)),
                            size(px(7.), px(4.)),
                        );
                        window.paint_quad(fill(inside, skin.accent).corner_radii(px(1.)));
                        window.paint_quad(
                            fill(
                                Bounds::new(
                                    point(center.x - px(4.), center.y + px(2.)),
                                    size(px(2.), px(3.)),
                                ),
                                skin.base,
                            )
                            .corner_radii(px(0.5)),
                        );
                    } else {
                        for bar in [
                            Bounds::new(
                                point(center.x - px(4.), center.y - px(0.75)),
                                size(px(8.), px(1.5)),
                            ),
                            Bounds::new(
                                point(center.x - px(0.75), center.y - px(4.)),
                                size(px(1.5), px(8.)),
                            ),
                        ] {
                            window.paint_quad(fill(bar, skin.base));
                        }
                    }
                }
                let text_bounds = Bounds::new(
                    point(x + px(cell.gutter), y + px(PAD)),
                    size(
                        px((cell.width - cell.gutter - 4.0).max(1.0)),
                        px(p.row.height - PAD * 2.0),
                    ),
                )
                .intersect(&frame.bounds);
                let origin = point(x + px(cell.gutter - frame.horizontal), y + px(PAD));
                window.with_content_mask(
                    Some(ContentMask {
                        bounds: text_bounds,
                    }),
                    |window| {
                        let Ok(line) = &cell.native else {
                            paint_label(
                                cell.native
                                    .as_ref()
                                    .err()
                                    .map(String::as_str)
                                    .unwrap_or("layout unavailable"),
                                origin,
                                skin.warning,
                                self.font_size,
                                &self.family,
                                window,
                                cx,
                            );
                            return;
                        };
                        let word_bg = if cell.cell.kind == RowKind::Removed {
                            skin.removed_word
                        } else {
                            skin.added_word
                        };
                        for range in &cell.cell.intra {
                            for rect in line.rectangles(range.clone()) {
                                window.paint_quad(fill(
                                    Bounds::new(origin + rect.origin, rect.size),
                                    word_bg,
                                ));
                            }
                        }
                        if let Some(range) =
                            self.selection_range(cell, p.row.unified, p.row.old_number)
                        {
                            for rect in line.rectangles(range) {
                                window.paint_quad(fill(
                                    Bounds::new(origin + rect.origin, rect.size),
                                    if self
                                        .active_search
                                        .as_ref()
                                        .zip(self.selection.as_ref())
                                        .is_some_and(|(a, b)| a.start == b.start && a.end == b.end)
                                    {
                                        skin.warning.opacity(0.55)
                                    } else {
                                        skin.selection
                                    },
                                ));
                            }
                        }
                        let spans: Vec<_> = self
                            .decorations
                            .get(&(cell.side, cell.cell.number))
                            .into_iter()
                            .flatten()
                            .map(|s| (s.bytes.clone(), skin.token(s.token)))
                            .collect();
                        line.paint(origin, text_bounds, skin.text, &spans, window, cx);
                    },
                );
                // Continuation symbols belong to the gutter and stay out of copied source.
                if let Ok(line) = &cell.native {
                    let first = (((-p.y - PAD) / line.line_height).floor().max(1.0)) as usize;
                    for i in first..line.fragments.len() {
                        let cy = y + px(PAD + i as f32 * line.line_height);
                        if cy > frame.bounds.bottom() {
                            break;
                        }
                        paint_label(
                            "↪",
                            point(x + px(cell.gutter - 20.0), cy),
                            skin.muted,
                            self.font_size - 2.0,
                            &self.family,
                            window,
                            cx,
                        );
                    }
                    if cell.cell.ending != LineEnding::Lf {
                        let text = cell.cell.ending.label();
                        let at = y + px(p.row.height - self.font_size - 4.0);
                        if at >= frame.bounds.top() && at <= frame.bounds.bottom() {
                            paint_label(
                                text,
                                point(x + px(6.0), at),
                                skin.muted,
                                9.0,
                                &self.family,
                                window,
                                cx,
                            );
                        }
                    }
                }
            }
            if self.split {
                window.paint_quad(fill(
                    Bounds::new(
                        point(frame.bounds.left() + px((self.width - 1.0) / 2.0), y),
                        size(px(1.0), px(p.row.height)),
                    ),
                    skin.border,
                ));
            }
        }
        if self.scrollbars_visible && self.height_index.total() > self.height {
            window.paint_quad(fill(frame.thumb, skin.muted.opacity(0.5)));
        }
        if self.scrollbars_visible && self.max_horizontal > 0.0 {
            window.paint_quad(fill(frame.horizontal_thumb, skin.muted.opacity(0.5)));
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
impl Viewport {
    pub fn move_selection(&mut self, motion: Motion, extend: bool) {
        use unicode_segmentation::UnicodeSegmentation;
        let current = self
            .selection
            .as_ref()
            .map(|s| s.end.clone())
            .or_else(|| self.anchor.as_ref().map(|a| a.point.clone()));
        let side = current.as_ref().map_or(Side::Right, |p| p.side);
        let cells: Vec<_> = self
            .rows
            .iter()
            .filter_map(|row| match row {
                DisplayRow::Line { left, right, .. } => {
                    if side == Side::Left {
                        left.as_ref()
                    } else {
                        right.as_ref()
                    }
                }
                _ => None,
            })
            .collect();
        if cells.is_empty() {
            return;
        }
        let mut at = current
            .as_ref()
            .and_then(|p| cells.iter().position(|c| c.number == p.line))
            .unwrap_or(0);
        let mut column = current
            .as_ref()
            .map_or(0, |p| p.byte_column)
            .min(cells[at].text.len());
        match motion {
            Motion::Up => at = at.saturating_sub(1),
            Motion::Down => at = (at + 1).min(cells.len() - 1),
            Motion::Home => column = 0,
            Motion::End => column = cells[at].text.len(),
            Motion::First => {
                at = 0;
                column = 0;
            }
            Motion::Last => {
                at = cells.len() - 1;
                column = cells[at].text.len();
            }
            Motion::Left => {
                if column == 0 && at > 0 {
                    at -= 1;
                    column = cells[at].text.len();
                } else {
                    column = cells[at]
                        .text
                        .grapheme_indices(true)
                        .map(|(i, _)| i)
                        .take_while(|i| *i < column)
                        .last()
                        .unwrap_or(0);
                }
            }
            Motion::Right => {
                if column >= cells[at].text.len() && at + 1 < cells.len() {
                    at += 1;
                    column = 0;
                } else {
                    column = cells[at]
                        .text
                        .grapheme_indices(true)
                        .map(|(i, _)| i)
                        .find(|i| *i > column)
                        .unwrap_or(cells[at].text.len());
                }
            }
        }
        column = diffz_core::layout::snap_grapheme(&cells[at].text, column);
        let end = SourcePoint {
            snapshot: self.snapshot.id.clone(),
            file: self.file.clone(),
            side,
            line: cells[at].number,
            byte_column: column,
        };
        let start = if extend {
            self.selection
                .as_ref()
                .map(|s| s.start.clone())
                .or(current)
                .unwrap_or_else(|| end.clone())
        } else {
            end.clone()
        };
        self.selection = Some(SourceSelection {
            start,
            end: end.clone(),
        });
        let visible = self.last.as_ref().is_some_and(|frame| {
            frame.rows.iter().any(|p| {
                p.row.cells.iter().any(|c| {
                    c.side == end.side
                        && c.cell.number == end.line
                        && c.native.as_ref().is_ok_and(|n| {
                            let y = p.y
                                + PAD
                                + n.fragment_for_source(end.byte_column) as f32 * n.line_height;
                            y >= 0. && y + n.line_height < f32::from(frame.bounds.size.height)
                        })
                })
            })
        });
        if !visible {
            self.reveal(end);
        }
    }
    pub fn boundary(&self, delta: f32) -> i8 {
        let Some(frame) = &self.last else {
            return 0;
        };
        if delta < 0.
            && frame
                .rows
                .first()
                .is_some_and(|r| r.index == 0 && r.y >= -0.5)
        {
            -1
        } else if delta > 0.
            && frame.rows.last().is_some_and(|r| {
                r.index + 1 == self.rows.len()
                    && r.y + r.row.height <= f32::from(frame.bounds.size.height) + 0.5
            })
        {
            1
        } else {
            0
        }
    }
    pub fn jump_edge(&mut self, end: bool) {
        self.hunk_target = None;
        self.anchor = None;
        self.pending_scroll = 0.;
        self.cursor = if end {
            Cursor {
                row: self.rows.len().saturating_sub(1),
                offset: 1_000_000.,
            }
        } else {
            Cursor::default()
        };
    }
    pub fn source_screen_point(&self, point: &SourcePoint) -> Option<Point<Pixels>> {
        if point.file != self.file {
            return None;
        }
        let frame = self.last.as_ref()?;
        for row in &frame.rows {
            for cell in &row.row.cells {
                if cell.side == point.side && cell.cell.number == point.line {
                    let line = cell.native.as_ref().ok()?;
                    let y = row.y
                        + PAD
                        + line.fragment_for_source(point.byte_column) as f32 * line.line_height
                        + line.line_height;
                    return Some(
                        frame.bounds.origin + gpui_kit::point(px(cell.x + cell.gutter), px(y)),
                    );
                }
            }
        }
        None
    }
    fn context_control(&self, position: Point<Pixels>) -> Option<(usize, ContextRequest)> {
        let f = self.last.as_ref()?;
        if !f.bounds.contains(&position) {
            return None;
        }
        let x = f32::from(position.x - f.bounds.left());
        let y = f32::from(position.y - f.bounds.top());
        let row = f.rows.iter().find(|r| {
            r.row.band > 0.0 && y >= r.y + r.row.height - r.row.band && y < r.y + r.row.height
        })?;
        f.context_controls
            .iter()
            .find(|c| c.row == row.index && x >= c.x.start - 8.0 && x < c.x.end + 8.0)
            .map(|c| (c.row, c.request))
    }
    pub fn context_hover(&self, position: Point<Pixels>) -> bool {
        self.context_control(position).is_some()
    }
    pub fn context_hit(&mut self, position: Point<Pixels>) -> Option<ContextRequest> {
        let (row, request) = self.context_control(position)?;
        self.hunk_target = Some(row);
        Some(request)
    }
    /// List the controls beneath hunk header `row` in display order.
    fn context_labels(&self, row: usize) -> Vec<(ContextRequest, String)> {
        [
            ContextRequest::Above,
            ContextRequest::All,
            ContextRequest::Below,
        ]
        .into_iter()
        .filter_map(|request| {
            let (_, _, _, count) = self.plan_for_row(row, request)?;
            let label = match request {
                ContextRequest::Above => format!("↑ Show {count} above"),
                ContextRequest::All => format!("⇈ Show all {count} above"),
                ContextRequest::Below => format!("↓ Show {count} below"),
            };
            Some((request, label))
        })
        .collect()
    }
    /// Return the target hunk index, old start, new start, and line count.
    pub fn context_plan(&self, request: ContextRequest) -> Option<(usize, u32, u32, u32)> {
        self.plan_for_row(self.hunk_target.unwrap_or(self.cursor.row), request)
    }
    fn plan_for_row(&self, row: usize, request: ContextRequest) -> Option<(usize, u32, u32, u32)> {
        let used = self.expanded.values().map(|(a, b)| a + b).sum::<u32>();
        if used >= CONTEXT_BUDGET {
            return None;
        }
        let h = self
            .hunk_rows
            .partition_point(|&header| header <= row)
            .saturating_sub(1);
        let file = self.snapshot.file(&self.file)?;
        let current = file.hunks.get(h)?;
        if current.old_count == 0 || current.new_count == 0 {
            return None;
        }
        let (before, after) = self.expanded.get(&h).copied().unwrap_or_default();
        if request.above() {
            let old = current.old_start.checked_sub(before)?;
            let new = current.new_start.checked_sub(before)?;
            let previous_end = h
                .checked_sub(1)
                .and_then(|i| {
                    file.hunks.get(i).map(|p| {
                        p.old_start + p.old_count + self.expanded.get(&i).map_or(0, |x| x.1)
                    })
                })
                .unwrap_or(1);
            let gap = old.saturating_sub(previous_end).min(new.saturating_sub(1));
            let count = match request {
                // Offer "Show all" only when one step would leave hidden content behind.
                ContextRequest::All if gap <= CONTEXT_STEP || gap > CONTEXT_BUDGET - used => {
                    return None;
                }
                ContextRequest::All => gap,
                _ => gap.min(CONTEXT_STEP),
            };
            (count > 0).then_some((h, old - count, new - count, count))
        } else {
            let old = current.old_start + current.old_count + after;
            let new = current.new_start + current.new_count + after;
            let count = file.hunks.get(h + 1).map_or(CONTEXT_STEP, |next| {
                next.old_start
                    .saturating_sub(self.expanded.get(&(h + 1)).map_or(0, |x| x.0))
                    .saturating_sub(old)
                    .min(CONTEXT_STEP)
            });
            (count > 0).then_some((h, old, new, count))
        }
    }
    pub fn insert_context(
        &mut self,
        hunk: usize,
        above: bool,
        old: u32,
        new: u32,
        lines: Vec<String>,
    ) {
        let Some(&header) = self.hunk_rows.get(hunk) else {
            return;
        };
        let index = if above {
            header + 1
        } else {
            self.hunk_rows
                .get(hunk + 1)
                .copied()
                .unwrap_or(self.rows.len())
        };
        let count = lines.len() as u32;
        let rows = lines.into_iter().enumerate().map(|(i, text)| {
            let cell = |number| Cell {
                number,
                text: text.as_str().into(),
                ending: LineEnding::Lf,
                kind: RowKind::Context,
                intra: vec![],
            };
            DisplayRow::Line {
                left: self.split.then(|| cell(old + i as u32)),
                right: Some(cell(new + i as u32)),
                unified: !self.split,
                old_number: Some(old + i as u32),
            }
        });
        Arc::make_mut(&mut self.rows).splice(index..index, rows);
        for header in &mut self.hunk_rows[hunk + 1..] {
            *header += count as usize;
        }
        let entry = self.expanded.entry(hunk).or_default();
        if above {
            entry.0 += count;
        } else {
            entry.1 += count;
        }
        self.dirty = true;
        self.hunk_target = None;
    }
    pub fn comment_hit(&self, position: Point<Pixels>) -> Option<SourcePoint> {
        let point = self.hit(position)?;
        self.snapshot
            .file(&self.file)?
            .line(point.side, point.line)?;
        let frame = self.last.as_ref()?;
        let x = f32::from(position.x - frame.bounds.left());
        frame
            .rows
            .iter()
            .flat_map(|r| &r.row.cells)
            .find(|c| {
                c.side == point.side
                    && c.cell.number == point.line
                    && x >= c.x + c.gutter - 25.
                    && x < c.x + c.gutter
            })
            .map(|_| point)
    }
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
        // The second hunk offers the fourteen concealed lines from 21 through 34.
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
