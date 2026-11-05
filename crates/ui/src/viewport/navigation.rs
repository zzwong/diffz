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
                DisplayRow::Line {
                    left,
                    right,
                    unified,
                    old_number,
                } => {
                    let (number, cell) = if side == Side::Left {
                        if let Some(cell) = left {
                            (cell.number, cell)
                        } else if *unified
                            && let Some(cell) = right
                            && cell.kind == RowKind::Context
                        {
                            // Unified context paints one RIGHT cell but belongs to both sides.
                            ((*old_number)?, cell)
                        } else {
                            return None;
                        }
                    } else {
                        let cell = right.as_ref()?;
                        (cell.number, cell)
                    };
                    Some((number, cell.text.as_ref()))
                }
                _ => None,
            })
            .collect();
        if cells.is_empty() {
            return;
        }
        let mut at = current
            .as_ref()
            .and_then(|p| cells.iter().position(|(number, _)| *number == p.line))
            .unwrap_or(0);
        let mut column = current
            .as_ref()
            .map_or(0, |p| p.byte_column)
            .min(cells[at].1.len());
        match motion {
            Motion::Up => at = at.saturating_sub(1),
            Motion::Down => at = (at + 1).min(cells.len() - 1),
            Motion::Home => column = 0,
            Motion::End => column = cells[at].1.len(),
            Motion::First => {
                at = 0;
                column = 0;
            }
            Motion::Last => {
                at = cells.len() - 1;
                column = cells[at].1.len();
            }
            Motion::Left => {
                if column == 0 && at > 0 {
                    at -= 1;
                    column = cells[at].1.len();
                } else {
                    column = cells[at].1
                        .grapheme_indices(true)
                        .map(|(i, _)| i)
                        .take_while(|i| *i < column)
                        .last()
                        .unwrap_or(0);
                }
            }
            Motion::Right => {
                if column >= cells[at].1.len() && at + 1 < cells.len() {
                    at += 1;
                    column = 0;
                } else {
                    column = cells[at].1
                        .grapheme_indices(true)
                        .map(|(i, _)| i)
                        .find(|i| *i > column)
                        .unwrap_or(cells[at].1.len());
                }
            }
        }
        column = diffz_core::layout::snap_grapheme(cells[at].1, column);
        let end = SourcePoint {
            snapshot: self.snapshot.id.clone(),
            file: self.file.clone(),
            side,
            line: cells[at].0,
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
                    let number = if c.side == end.side {
                        Some(c.cell.number)
                    } else if p.row.unified
                        && end.side == Side::Left
                        && c.cell.kind == RowKind::Context
                    {
                        p.row.old_number
                    } else {
                        None
                    };
                    number == Some(end.line)
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

#[cfg(test)]
mod selection_navigation_tests {
    use super::*;

    const PATCH: &[u8] = concat!(
        "diff --git a/a.txt b/a.txt\n--- a/a.txt\n+++ b/a.txt\n",
        "@@ -10,5 +20,6 @@\n before\n-old\n+new\n+extra\n after\n e\u{301}🙂\n tail\n",
    )
    .as_bytes();

    fn viewport(patch: &[u8], split: bool) -> Viewport {
        let snapshot = Arc::new(Snapshot::new(
            "navigation".into(),
            diffz_core::patch::parse_patch(patch, Default::default()).unwrap(),
            None,
            vec![],
        ));
        let file = snapshot.patch.files[0].id.clone();
        Viewport::new(snapshot, file, split, false, 14., "Menlo".into(), None)
    }

    fn select(v: &mut Viewport, side: Side, line: u32, byte_column: usize) {
        let point = SourcePoint {
            snapshot: v.snapshot.id.clone(),
            file: v.file.clone(),
            side,
            line,
            byte_column,
        };
        v.snapshot.validate_point(&point).unwrap();
        v.selection = Some(SourceSelection {
            start: point.clone(),
            end: point,
        });
    }

    fn endpoint(v: &Viewport) -> (Side, u32, usize) {
        let point = &v.selection.as_ref().unwrap().end;
        v.snapshot.validate_point(point).unwrap();
        (point.side, point.line, point.byte_column)
    }

    #[test]
    fn selection_navigation_visits_context_on_each_source_side() {
        for split in [false, true] {
            for (side, first, last) in [(Side::Left, 10, 14), (Side::Right, 20, 25)] {
                let mut v = viewport(PATCH, split);
                select(&mut v, side, first, 0);
                let mut visited = vec![first];
                for _ in first..last {
                    v.move_selection(Motion::Down, false);
                    visited.push(endpoint(&v).1);
                }
                assert_eq!(visited, (first..=last).collect::<Vec<_>>());
                for expected in (first..last).rev() {
                    v.move_selection(Motion::Up, false);
                    assert_eq!(endpoint(&v), (side, expected, 0));
                }
            }
        }
    }

    #[test]
    fn selection_navigation_keeps_old_context_line_for_home_and_end() {
        let mut v = viewport(PATCH, false);
        select(&mut v, Side::Left, 12, 0);
        // This is the selection made after clicking the old-line gutter.
        v.select_source_line(v.selection.as_ref().unwrap().end.clone());
        v.move_selection(Motion::Home, false);
        assert_eq!(endpoint(&v), (Side::Left, 12, 0));
        v.move_selection(Motion::End, false);
        assert_eq!(endpoint(&v), (Side::Left, 12, 5));
        v.move_selection(Motion::First, false);
        assert_eq!(endpoint(&v), (Side::Left, 10, 0));
        v.move_selection(Motion::Last, false);
        assert_eq!(endpoint(&v), (Side::Left, 14, 4));
    }

    #[test]
    fn selection_navigation_extends_from_removed_text_into_old_context() {
        for split in [false, true] {
            let mut v = viewport(PATCH, split);
            select(&mut v, Side::Left, 11, 0);
            v.move_selection(Motion::Down, true);
            assert_eq!(endpoint(&v), (Side::Left, 12, 0));
            v.move_selection(Motion::End, true);
            let selection = v.selection.as_ref().unwrap();
            assert_eq!(selection.start.line, 11);
            assert_eq!(selection.start.byte_column, 0);
            assert_eq!(v.snapshot.copy_selection(selection).unwrap(), "old\nafter");
        }
    }

    #[test]
    fn selection_navigation_preserves_graphemes_and_context_boundaries() {
        for split in [false, true] {
            let mut v = viewport(PATCH, split);
            select(&mut v, Side::Left, 13, 0);
            for (motion, line, column) in [
                (Motion::Right, 13, 3),
                (Motion::Right, 13, 7),
                (Motion::Right, 14, 0),
                (Motion::Left, 13, 7),
                (Motion::Left, 13, 3),
                (Motion::Left, 13, 0),
                (Motion::Left, 12, 5),
            ] {
                v.move_selection(motion, false);
                assert_eq!(endpoint(&v), (Side::Left, line, column));
            }
        }
    }

    #[test]
    fn selection_navigation_old_context_works_without_removed_rows() {
        let patch = b"diff --git a/a.txt b/a.txt\n--- a/a.txt\n+++ b/a.txt\n@@ -1,2 +1,3 @@\n first\n+added\n last\n";
        let mut v = viewport(patch, false);
        select(&mut v, Side::Left, 1, 0);
        v.move_selection(Motion::Down, false);
        assert_eq!(endpoint(&v), (Side::Left, 2, 0));
        v.move_selection(Motion::End, false);
        assert_eq!(endpoint(&v), (Side::Left, 2, 4));
    }
}
