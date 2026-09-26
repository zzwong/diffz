use super::*;

impl Viewport {
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
    pub(super) fn prepare_geometry(&mut self, width: f32) -> bool {
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
        'outer: for p in &rows {
            for cell in &p.row.cells {
                let Ok(line) = &cell.native else {
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
        self.anchor = anchor;
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
        let absolute = self.height_index.prefix(self.cursor.row) + self.cursor.offset;
        let (thumb_y, thumb_h) = vertical_thumb(self.height, self.height_index.total(), absolute);
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
            horizontal_track,
            horizontal_thumb,
            bounds,
            rows,
            horizontal: self.horizontal,
            thumb: Bounds::new(
                point(bounds.right() - px(BAR - 2.0), bounds.top() + px(thumb_y)),
                size(px(BAR - 4.0), px(thumb_h)),
            ),
            context_controls,
        };
        self.last = Some(frame.clone());
        frame
    }
}
