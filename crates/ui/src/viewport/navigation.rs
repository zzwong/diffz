use super::*;

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
    pub(super) fn context_labels(&self, row: usize) -> Vec<(ContextRequest, String)> {
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
    pub(super) fn plan_for_row(
        &self,
        row: usize,
        request: ContextRequest,
    ) -> Option<(usize, u32, u32, u32)> {
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
