use super::*;

impl Viewport {
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
                let mark = self
                    .annotations
                    .get(&(cell.side, cell.cell.number))
                    .or_else(|| {
                        p.row
                            .unified
                            .then_some(p.row.old_number)
                            .flatten()
                            .and_then(|n| self.annotations.get(&(Side::Left, n)))
                    });
                if let Some(severity) = mark {
                    let bar = Bounds::new(point(x, y), size(px(3.), px(p.row.height)));
                    window.paint_quad(fill(bar, skin.severity(*severity)));
                }
                let release = self.releases.get(&(cell.side, cell.cell.number));
                if let Some((index, dim)) = release.and_then(|r| Some((r.release?, r.dim))) {
                    // A band between the line numbers and the text, in the release's color.
                    let band = Bounds::new(
                        point(x + px(cell.gutter - 4.), y),
                        size(px(3.), px(p.row.height)),
                    );
                    let color = skin.release(index);
                    window.paint_quad(fill(band, if dim { color.opacity(0.3) } else { color }));
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
                // Lines from releases other than the one the view is narrowed to fade back.
                if release.is_some_and(|r| r.dim) {
                    window.paint_quad(fill(cb, skin.base.opacity(0.6)));
                }
                if let Some(release) = release.filter(|_| hover) {
                    let font = self.font_size - 2.0;
                    let width = label_width(&release.label, font, &self.family, window) + 16.;
                    let chip = Bounds::new(
                        point(
                            x + px((cell.width - width - 8.).max(cell.gutter)),
                            y + px(PAD),
                        ),
                        size(px(width), px(font * 1.4 + 2.)),
                    );
                    window.with_content_mask(
                        Some(ContentMask {
                            bounds: cb.intersect(&frame.bounds),
                        }),
                        |window| {
                            window.paint_quad(fill(chip, skin.raised).corner_radii(px(4.)));
                            window.paint_quad(fill(
                                Bounds::new(chip.origin, size(px(3.), chip.size.height)),
                                release.release.map_or(skin.muted, |i| skin.release(i)),
                            ));
                            paint_label(
                                &release.label,
                                point(chip.left() + px(10.), chip.top() + px(1.)),
                                skin.text,
                                font,
                                &self.family,
                                window,
                                cx,
                            );
                        },
                    );
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
        let (edge, progress) = self.pull;
        if edge != 0 && progress > 0.0 {
            let y = if edge > 0 {
                frame.bounds.bottom() - px(3.0)
            } else {
                frame.bounds.top()
            };
            window.paint_quad(fill(
                Bounds::new(
                    point(frame.bounds.left(), y),
                    size(px(self.width * progress.clamp(0.0, 1.0)), px(3.0)),
                ),
                skin.accent,
            ));
        }
    }
}
