//! Rich diff at block level for Markdown prose. Splits the rows of a patch into whole
//! paragraph blocks a renderer draws using the states unchanged, added, removed, and changed.

use crate::patch::{FileChange, Hunk, PatchRow, RowKind};
use std::ops::Range;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RichKind {
    Same,
    Added,
    Removed,
    Changed,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RichBlock {
    pub kind: RichKind,
    /// Markdown source on the old side; its lines join with '\n' (Added blocks have None).
    pub old: Option<String>,
    /// Markdown source on the new side (Removed blocks have None).
    pub new: Option<String>,
    /// The block's first line number on a side, only when that side is present.
    pub old_line: Option<u32>,
    pub new_line: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RichItem {
    Block(RichBlock),
    /// Lines the patch leaves out, between hunks.
    Gap {
        old_line: u32,
        new_line: u32,
        count: u32,
    },
}

/// Boundaries over `lines` that ignore bytes; every range is a span of line indexes.
pub fn blocks(lines: &[&str]) -> Vec<Range<usize>> {
    let mut out = vec![];
    let mut i = 0;
    while i < lines.len() {
        let trimmed = lines[i].trim();
        if trimmed.is_empty() {
            i += 1;
            continue;
        }
        if let Some((ch, len)) = opener(trimmed) {
            // Fenced code block: extend to the matching end fence and keep that fence too.
            let mut j = i + 1;
            while j < lines.len() && !closer(lines[j].trim(), ch, len) {
                j += 1;
            }
            if j < lines.len() {
                out.push(i..(j + 1));
                i = j + 1;
            } else {
                // A fence that never closes reaches the end.
                out.push(i..lines.len());
                i = lines.len();
            }
        } else if is_heading(lines[i]) {
            out.push(i..(i + 1));
            i += 1;
        } else {
            // Longest run of lines that are neither blank nor special.
            let start = i;
            while i < lines.len()
                && !lines[i].trim().is_empty()
                && !is_heading(lines[i])
                && opener(lines[i].trim()).is_none()
            {
                i += 1;
            }
            out.push(start..i);
        }
    }
    out
}

/// An opening fence starts the trimmed line with 3+ backticks or tildes.
fn opener(trimmed: &str) -> Option<(char, usize)> {
    let ch = trimmed.chars().next()?;
    if ch != '`' && ch != '~' {
        return None;
    }
    let len = trimmed.chars().take_while(|&c| c == ch).count();
    (len >= 3).then_some((ch, len))
}

/// A closer fence repeats the same char at least as many times as the opener.
fn closer(trimmed: &str, ch: char, open_len: usize) -> bool {
    trimmed.starts_with(ch) && trimmed.chars().take_while(|&c| c == ch).count() >= open_len.max(3)
}

/// ATX heading: `#` repeated 1-6 times, then a space, with at most 3 leading spaces allowed.
fn is_heading(line: &str) -> bool {
    let spaces = line.chars().take_while(|&c| c == ' ').count();
    if spaces > 3 {
        return false;
    }
    let rest = &line[spaces..];
    let hashes = rest.chars().take_while(|&c| c == '#').count();
    if !(1..=6).contains(&hashes) {
        return false;
    }
    rest[hashes..].starts_with(' ')
}

/// One hunk side: kept rows plus their line numbers, grouped as blocks.
struct Side {
    lines: Vec<String>,
    nums: Vec<u32>,
    blocks: Vec<Range<usize>>,
}

impl Side {
    fn text(&self, idx: usize) -> String {
        let r = &self.blocks[idx];
        self.lines[r.clone()].join("\n")
    }
    fn norm(&self, idx: usize) -> String {
        let r = &self.blocks[idx];
        self.lines[r.clone()]
            .iter()
            .map(|s| s.trim_end())
            .collect::<Vec<_>>()
            .join("\n")
    }
}

fn build_side(
    hunk: &Hunk,
    keep: impl Fn(RowKind) -> bool,
    num: impl Fn(&PatchRow) -> Option<u32>,
) -> Side {
    let mut lines = vec![];
    let mut nums = vec![];
    for row in &hunk.rows {
        if keep(row.kind) {
            lines.push(row.text.clone());
            // A kept row carries only its own side's line number.
            nums.push(num(row).unwrap_or(0));
        }
    }
    let refs: Vec<&str> = lines.iter().map(String::as_str).collect();
    let blocks = blocks(&refs);
    Side {
        lines,
        nums,
        blocks,
    }
}

/// LCS over normalized block text. None once the pairing grows too large.
fn lcs(a: &Side, b: &Side) -> Option<Vec<(usize, usize)>> {
    if a.blocks.is_empty() || b.blocks.is_empty() {
        return Some(vec![]);
    }
    if a.blocks.len() * b.blocks.len() > 250_000 {
        return None;
    }
    let na: Vec<String> = (0..a.blocks.len()).map(|i| a.norm(i)).collect();
    let nb: Vec<String> = (0..b.blocks.len()).map(|i| b.norm(i)).collect();
    let cols = nb.len() + 1;
    let rows = na.len() + 1;
    let mut dp = vec![0u16; rows * cols];
    for i in (0..na.len()).rev() {
        for j in (0..nb.len()).rev() {
            dp[i * cols + j] = if na[i] == nb[j] {
                1 + dp[(i + 1) * cols + j + 1]
            } else {
                dp[(i + 1) * cols + j].max(dp[i * cols + j + 1])
            };
        }
    }
    let mut out = vec![];
    let (mut i, mut j) = (0, 0);
    while i < na.len() && j < nb.len() {
        if na[i] == nb[j] {
            out.push((i, j));
            i += 1;
            j += 1;
        } else if i + 1 < na.len() && dp[(i + 1) * cols + j] >= dp[i * cols + j + 1] {
            i += 1;
        } else {
            j += 1;
        }
    }
    Some(out)
}

fn block(kind: RichKind, old: Option<(usize, &Side)>, new: Option<(usize, &Side)>) -> RichBlock {
    RichBlock {
        kind,
        old: old.map(|(i, s)| s.text(i)),
        new: new.map(|(i, s)| s.text(i)),
        old_line: old.map(|(i, s)| s.nums[s.blocks[i].start]),
        new_line: new.map(|(i, s)| s.nums[s.blocks[i].start]),
    }
}

fn emit_hunk(hunk: &Hunk, out: &mut Vec<RichItem>) {
    let old = build_side(hunk, |k| k != RowKind::Added, |r| r.old_line);
    let new = build_side(hunk, |k| k != RowKind::Removed, |r| r.new_line);
    let Some(matches) = lcs(&old, &new) else {
        // Pairing too large: each old block becomes Removed, each new one Added.
        for i in 0..old.blocks.len() {
            out.push(RichItem::Block(block(
                RichKind::Removed,
                Some((i, &old)),
                None,
            )));
        }
        for j in 0..new.blocks.len() {
            out.push(RichItem::Block(block(
                RichKind::Added,
                None,
                Some((j, &new)),
            )));
        }
        return;
    };

    let (mut i, mut j, mut mi) = (0, 0, 0);
    while i < old.blocks.len() || j < new.blocks.len() {
        if mi < matches.len() && i == matches[mi].0 && j == matches[mi].1 {
            let (oi, nj) = matches[mi];
            out.push(RichItem::Block(block(
                RichKind::Same,
                Some((oi, &old)),
                Some((nj, &new)),
            )));
            i = oi + 1;
            j = nj + 1;
            mi += 1;
        } else {
            // Emit every row from the current position up to the next match.
            let old_end = matches.get(mi).map_or(old.blocks.len(), |(oi, _)| *oi);
            let new_end = matches.get(mi).map_or(new.blocks.len(), |(_, nj)| *nj);
            let k = old_end - i;
            let m = new_end - j;
            let paired = k.min(m);
            for t in 0..paired {
                out.push(RichItem::Block(block(
                    RichKind::Changed,
                    Some((i + t, &old)),
                    Some((j + t, &new)),
                )));
            }
            for t in paired..k {
                out.push(RichItem::Block(block(
                    RichKind::Removed,
                    Some((i + t, &old)),
                    None,
                )));
            }
            for t in paired..m {
                out.push(RichItem::Block(block(
                    RichKind::Added,
                    None,
                    Some((j + t, &new)),
                )));
            }
            i = old_end;
            j = new_end;
        }
    }
}

/// Experimental: mark the words that change inside a Changed block, using markup the
/// renderer knows (`removed`/`added` are the opening and closing delimiters).
/// Fenced code blocks pass through untouched; whitespace-only edits get no marks.
pub fn mark_words(old: &str, new: &str, removed: [&str; 2], added: [&str; 2]) -> (String, String) {
    if old.trim_start().starts_with("```") || old.trim_start().starts_with("~~~") {
        return (old.to_owned(), new.to_owned());
    }
    // A paragraph spans many source rows; 3,000 tokens holds the LCS table below 20 MB.
    let (a, b) = crate::inline::word_diff_with(old, new, 64 * 1024, 3_000);
    (wrap(old, &a, removed), wrap(new, &b, added))
}
fn wrap(text: &str, ranges: &[Range<usize>], marker: [&str; 2]) -> String {
    // Words separated by just a space or a punctuation mark read better when merged into one span.
    let mut merged: Vec<Range<usize>> = Vec::with_capacity(ranges.len());
    for r in ranges {
        if let Some(last) = merged.last_mut()
            && r.start >= last.end
            && r.start - last.end <= 2
            && !text[last.end..r.start].chars().any(char::is_alphanumeric)
        {
            last.end = r.end;
            continue;
        }
        merged.push(r.clone());
    }
    let mut out = String::with_capacity(text.len() + merged.len() * 8);
    let mut at = 0;
    for r in &merged {
        let piece = &text[r.clone()];
        let lead = piece.len() - piece.trim_start().len();
        let trail = piece.len() - piece.trim_end().len();
        if lead + trail >= piece.len() {
            continue;
        }
        let (start, end) = (r.start + lead, r.end - trail);
        out.push_str(&text[at..start]);
        out.push_str(marker[0]);
        out.push_str(&text[start..end]);
        out.push_str(marker[1]);
        at = end;
    }
    out.push_str(&text[at..]);
    out
}

pub fn rich_diff(file: &FileChange) -> Vec<RichItem> {
    let mut out = vec![];
    let (mut prev_old_end, mut prev_new_end) = (0u32, 0u32);
    for hunk in &file.hunks {
        if hunk.new_start > prev_new_end + 1 {
            out.push(RichItem::Gap {
                old_line: prev_old_end + 1,
                new_line: prev_new_end + 1,
                count: hunk.new_start - prev_new_end - 1,
            });
        }
        emit_hunk(hunk, &mut out);
        prev_old_end = hunk
            .rows
            .iter()
            .filter_map(|r| r.old_line)
            .max()
            .unwrap_or(prev_old_end);
        prev_new_end = hunk
            .rows
            .iter()
            .filter_map(|r| r.new_line)
            .max()
            .unwrap_or(prev_new_end);
    }
    out
}

#[cfg(test)]
mod tests {
    #[test]
    fn mark_words_wraps_only_the_changed_words() {
        let (old, new) = super::mark_words(
            "Reading, copying, and commenting must hold.",
            "Careful reading, exact copying must hold.",
            ["~~", "~~"],
            ["<mark>", "</mark>"],
        );
        assert!(old.contains("~~"), "{old}");
        assert!(!old.contains("must hold.~~"), "{old}");
        assert!(
            new.contains("<mark>Careful reading</mark>") || new.contains("<mark>Careful"),
            "{new}"
        );
        assert!(new.ends_with("must hold."), "{new}");
    }
    #[test]
    fn mark_words_handles_a_long_paragraph_with_a_local_edit() {
        let filler = "This paragraph is one source line. ".repeat(600);
        let old = format!("Intro. Reading, copying, and commenting must hold. {filler}\n");
        let new = format!("Intro. Careful reading, exact copying must hold. {filler}");
        let (o, n) = super::mark_words(&old, &new, ["~~", "~~"], ["==", "=="]);
        assert!(o.starts_with("Intro. ~~"), "{}", &o[..60]);
        assert!(n.starts_with("Intro. =="), "{}", &n[..60]);
        assert_eq!(o.matches("~~").count() % 2, 0);
        assert!(o.ends_with("line. \n") && n.ends_with(&filler));
        assert!(
            o.starts_with("Intro. ~~Reading, copying, and commenting~~ must"),
            "{}",
            &o[..120]
        );
    }
    #[test]
    fn mark_words_marks_both_ends_of_a_long_paragraph_without_striking_the_middle() {
        let filler = "This paragraph is one source line. ".repeat(600);
        let old = format!("Reading, copying, and commenting must hold. {filler}BEFORE-END");
        let new = format!("Careful reading, exact copying must hold. {filler}AFTER-END");
        let (o, n) = super::mark_words(&old, &new, ["~~", "~~"], ["<mark>", "</mark>"]);
        assert!(
            o.starts_with("~~Reading, copying, and commenting~~ must"),
            "{}",
            &o[..80]
        );
        assert!(o.ends_with("~~BEFORE~~-END"), "{}", &o[o.len() - 40..]);
        assert!(
            n.ends_with("<mark>AFTER</mark>-END"),
            "{}",
            &n[n.len() - 40..]
        );
        assert!(
            o.contains("hold. This paragraph is one source line. This"),
            "middle must stay unmarked"
        );
        assert_eq!(o.matches("~~").count(), 4, "{}", &o[..200]);
    }
    #[test]
    fn mark_words_leaves_code_fences_and_space_only_changes_alone() {
        let fence = "```rs\nlet a = 1;\n```";
        assert_eq!(
            super::mark_words(fence, fence, ["~~", "~~"], ["<", ">"]).0,
            fence
        );
        let (old, new) = super::mark_words("a  b", "a b", ["~~", "~~"], ["<", ">"]);
        assert_eq!(old, "a  b");
        assert_eq!(new, "a b");
    }

    use super::*;
    use crate::patch::{ParseLimits, parse_patch};

    fn file(text: &str) -> FileChange {
        let report =
            parse_patch(text.as_bytes(), ParseLimits::default()).unwrap_or_else(|e| panic!("{e}"));
        report.files[0].clone()
    }

    #[test]
    fn two_paragraphs_separated_by_blank_line_are_two_blocks() {
        assert_eq!(blocks(&["para one", "", "para two"]), vec![0..1, 2..3]);
    }

    #[test]
    fn heading_directly_above_paragraph_is_a_separate_block() {
        assert_eq!(blocks(&["# Title", "body"]), vec![0..1, 1..2]);
    }

    #[test]
    fn fenced_block_keeping_blank_line_is_one_block() {
        assert_eq!(blocks(&["```", "code", "", "more", "```"]), vec![0..5]);
    }

    #[test]
    fn unclosed_fence_runs_to_the_end() {
        assert_eq!(blocks(&["```", "code", ""]), vec![0..3]);
    }

    #[test]
    fn rewrite_between_two_contexts_is_same_changed_same() {
        let f = file(
            "diff --git a/f.md b/f.md
index x..y 100644
--- a/f.md
+++ b/f.md
@@ -1,6 +1,6 @@
 First para
 
-Old third para
+New third para
 
 Fourth para
 Fifth para
",
        );
        let items = rich_diff(&f);
        assert_eq!(items.len(), 3);
        let RichItem::Block(a) = &items[0] else {
            panic!("expected Same block");
        };
        assert_eq!(a.kind, RichKind::Same);
        assert_eq!(a.old.as_deref(), Some("First para"));
        assert_eq!(a.new.as_deref(), Some("First para"));
        assert_eq!((a.old_line, a.new_line), (Some(1), Some(1)));
        let RichItem::Block(b) = &items[1] else {
            panic!("expected Changed block");
        };
        assert_eq!(b.kind, RichKind::Changed);
        assert_eq!(b.old.as_deref(), Some("Old third para"));
        assert_eq!(b.new.as_deref(), Some("New third para"));
        assert_eq!((b.old_line, b.new_line), (Some(3), Some(3)));
        let RichItem::Block(c) = &items[2] else {
            panic!("expected Same block");
        };
        assert_eq!(c.kind, RichKind::Same);
        assert_eq!(c.old.as_deref(), Some("Fourth para\nFifth para"));
        assert_eq!(c.new.as_deref(), Some("Fourth para\nFifth para"));
        assert_eq!((c.old_line, c.new_line), (Some(5), Some(5)));
    }

    #[test]
    fn added_paragraph_is_exactly_one_added() {
        let f = file(
            "diff --git a/f.md b/f.md
index x..y 100644
--- a/f.md
+++ b/f.md
@@ -1,3 +1,5 @@
 First para
 
+New para
+
 Second para
",
        );
        let items = rich_diff(&f);
        let added: Vec<_> = items
            .iter()
            .filter(|i| matches!(i, RichItem::Block(b) if b.kind == RichKind::Added))
            .collect();
        assert_eq!(added.len(), 1);
        assert_eq!(
            added[0],
            &RichItem::Block(RichBlock {
                kind: RichKind::Added,
                old: None,
                new: Some("New para".into()),
                old_line: None,
                new_line: Some(3),
            })
        );
    }

    #[test]
    fn ten_unchanged_lines_between_hunks_produce_a_gap() {
        let f = file(
            "diff --git a/f.md b/f.md
index x..y 100644
--- a/f.md
+++ b/f.md
@@ -1,3 +1,3 @@
 # Doc
 
-old
+new
@@ -14,1 +14,1 @@
-something
+something else
",
        );
        let items = rich_diff(&f);
        let gaps: Vec<_> = items
            .iter()
            .filter_map(|i| match i {
                RichItem::Gap {
                    old_line,
                    new_line,
                    count,
                } => Some((*old_line, *new_line, *count)),
                _ => None,
            })
            .collect();
        assert_eq!(gaps, vec![(4, 4, 10)]);
        // the gap lies between the blocks of the two hunks
        let gap_idx = items
            .iter()
            .position(|i| matches!(i, RichItem::Gap { .. }))
            .unwrap();
        assert!(gap_idx > 0 && gap_idx + 1 < items.len());
    }

    #[test]
    fn trailing_whitespace_only_difference_is_same() {
        let f = file(
            "diff --git a/f.md b/f.md\n\
             index x..y 100644\n\
             --- a/f.md\n\
             +++ b/f.md\n\
             @@ -1,1 +1,1 @@\n\
             -a    \n\
             +a\n",
        );
        let items = rich_diff(&f);
        assert_eq!(
            items,
            vec![RichItem::Block(RichBlock {
                kind: RichKind::Same,
                old: Some("a    ".into()),
                new: Some("a".into()),
                old_line: Some(1),
                new_line: Some(1),
            })]
        );
    }
}
