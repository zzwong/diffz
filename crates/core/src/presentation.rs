//! Presentation rows support lookup only; durable source coordinates live elsewhere.
use crate::{domain::*, patch::*};
#[derive(Debug, Clone)]
pub struct Cell {
    pub number: u32,
    pub text: std::sync::Arc<str>,
    pub ending: LineEnding,
    pub kind: RowKind,
    pub intra: Vec<std::ops::Range<usize>>,
}
impl Cell {
    fn from(r: &PatchRow, side: Side) -> Option<Self> {
        Some(Self {
            number: r.number(side)?,
            text: r.text.clone(),
            ending: r.ending,
            kind: r.kind,
            intra: vec![],
        })
    }
}
#[derive(Debug, Clone)]
pub enum DisplayRow {
    Hunk {
        label: String,
        source_line: u32,
        side: Side,
    },
    Line {
        left: Option<Cell>,
        right: Option<Cell>,
        unified: bool,
        old_number: Option<u32>,
    },
    Notice(String),
}
impl DisplayRow {
    pub fn cell(&self, side: Side) -> Option<&Cell> {
        match self {
            Self::Line { left, right, .. } => match side {
                Side::Left => left.as_ref(),
                Side::Right => right.as_ref(),
            },
            _ => None,
        }
    }
    pub fn find_source(rows: &[Self], side: Side, line: u32) -> Option<usize> {
        rows.iter()
            .position(|r| r.cell(side).is_some_and(|c| c.number == line))
    }
}
pub fn rows(file: &FileChange, split: bool) -> Vec<DisplayRow> {
    let mut out = vec![];
    if file.content != ContentKind::Text {
        out.push(DisplayRow::Notice(format!(
            "{:?} change — executable and binary content are not opened",
            file.content
        )));
    }
    if file.old_mode != file.new_mode {
        let label = match file.kind {
            ChangeKind::Added => {
                if file.new_mode.as_deref() == Some("100755") {
                    "Added executable file".into()
                } else {
                    "Added file".into()
                }
            }
            ChangeKind::Deleted => "Deleted file".into(),
            _ => match (file.old_mode.as_deref(), file.new_mode.as_deref()) {
                (Some("100644"), Some("100755")) => "Executable permission added".into(),
                (Some("100755"), Some("100644")) => "Executable permission removed".into(),
                (old, new) => format!(
                    "File mode: {} → {}",
                    old.unwrap_or("not recorded"),
                    new.unwrap_or("not recorded")
                ),
            },
        };
        out.push(DisplayRow::Notice(label));
    }
    if file.hunks.is_empty() {
        out.push(DisplayRow::Notice(format!(
            "{:?}: metadata-only change",
            file.kind
        )));
        return out;
    }
    for h in &file.hunks {
        out.push(DisplayRow::Hunk {
            label: format!(
                "@@ -{},{} +{},{} @@ {}",
                h.old_start, h.old_count, h.new_start, h.new_count, h.section
            ),
            source_line: if h.new_count > 0 {
                h.new_start
            } else {
                h.old_start
            },
            side: if h.new_count > 0 {
                Side::Right
            } else {
                Side::Left
            },
        });
        if !split {
            for r in &h.rows {
                out.push(DisplayRow::Line {
                    left: if r.kind == RowKind::Removed {
                        Cell::from(r, Side::Left)
                    } else {
                        None
                    },
                    right: if r.kind != RowKind::Removed {
                        Cell::from(r, Side::Right)
                    } else {
                        None
                    },
                    unified: true,
                    old_number: r.old_line,
                });
            }
        } else {
            let mut i = 0;
            while i < h.rows.len() {
                let r = &h.rows[i];
                if r.kind == RowKind::Removed {
                    let start = i;
                    while i < h.rows.len() && h.rows[i].kind == RowKind::Removed {
                        i += 1
                    }
                    let mid = i;
                    while i < h.rows.len() && h.rows[i].kind == RowKind::Added {
                        i += 1
                    }
                    let end = i;
                    for p in 0..(mid - start).max(end - mid) {
                        out.push(DisplayRow::Line {
                            left: if start + p < mid {
                                Cell::from(&h.rows[start + p], Side::Left)
                            } else {
                                None
                            },
                            right: if mid + p < end {
                                Cell::from(&h.rows[mid + p], Side::Right)
                            } else {
                                None
                            },
                            unified: false,
                            old_number: None,
                        });
                    }
                } else {
                    out.push(DisplayRow::Line {
                        left: Cell::from(r, Side::Left),
                        right: Cell::from(r, Side::Right),
                        unified: false,
                        old_number: None,
                    });
                    i += 1;
                }
            }
        }
    }
    // Optional word decoration stays bounded; line identity and hunk eligibility are unaffected.
    let mut words = std::collections::HashMap::new();
    for h in &file.hunks {
        let mut i = 0;
        while i < h.rows.len() {
            if h.rows[i].kind != RowKind::Removed {
                i += 1;
                continue;
            }
            let start = i;
            while i < h.rows.len() && h.rows[i].kind == RowKind::Removed {
                i += 1
            }
            let mid = i;
            while i < h.rows.len() && h.rows[i].kind == RowKind::Added {
                i += 1
            }
            if mid - start != i - mid || mid - start > 32 {
                continue;
            }
            for pair in 0..mid - start {
                let old = &h.rows[start + pair];
                let new = &h.rows[mid + pair];
                let (left, right) = crate::inline::word_diff(&old.text, &new.text);
                if let Some(n) = old.old_line {
                    words.insert((Side::Left, n), left);
                }
                if let Some(n) = new.new_line {
                    words.insert((Side::Right, n), right);
                }
            }
        }
    }
    for row in &mut out {
        if let DisplayRow::Line { left, right, .. } = row {
            for (side, cell) in [(Side::Left, left), (Side::Right, right)] {
                if let Some(cell) = cell {
                    cell.intra = words.remove(&(side, cell.number)).unwrap_or_default();
                }
            }
        }
    }
    out
}
pub fn default_wrap(path: &str) -> bool {
    let p = path.to_ascii_lowercase();
    [".md", ".markdown", ".mdx"].iter().any(|e| p.ends_with(e))
}
#[derive(Debug, Clone)]
pub struct SearchHit {
    pub file: FileId,
    pub side: Side,
    pub line: u32,
    pub bytes: std::ops::Range<usize>,
}
pub fn find(snapshot: &Snapshot, query: &str, limit: usize) -> Vec<SearchHit> {
    if query.is_empty() {
        return vec![];
    }
    let mut out = vec![];
    for f in &snapshot.patch.files {
        for h in &f.hunks {
            for r in &h.rows {
                let side = if r.kind == RowKind::Removed {
                    Side::Left
                } else {
                    Side::Right
                };
                let Some(line) = r.number(side) else { continue };
                for (i, _) in r.text.match_indices(query) {
                    out.push(SearchHit {
                        file: f.id.clone(),
                        side,
                        line,
                        bytes: i..i + query.len(),
                    });
                    if out.len() >= limit {
                        return out;
                    }
                }
            }
        }
    }
    out
}
