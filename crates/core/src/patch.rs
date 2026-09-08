//! Parser for counted unified patches. Bad or unsupported input cannot become an empty success.
use crate::domain::*;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use thiserror::Error;
#[derive(Debug, Clone, Copy)]
pub struct ParseLimits {
    pub max_bytes: usize,
    pub max_files: usize,
    pub max_rows: usize,
    pub max_line_bytes: usize,
}
impl Default for ParseLimits {
    fn default() -> Self {
        Self {
            max_bytes: 32 * 1024 * 1024,
            max_files: 10_000,
            max_rows: 500_000,
            max_line_bytes: 1024 * 1024,
        }
    }
}
#[derive(Debug, Clone, Error, PartialEq, Eq)]
pub enum PatchError {
    #[error("patch line {line} is malformed: {message}")]
    Malformed { line: usize, message: String },
    #[error("unsupported patch format: {0}")]
    Unsupported(String),
    #[error("patch exceeds safety limit: {0}")]
    Limit(String),
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ChangeKind {
    Added,
    Deleted,
    Modified,
    Renamed,
    Copied,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ContentKind {
    Text,
    Binary,
    Submodule,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum RowKind {
    Context,
    Added,
    Removed,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PatchRow {
    pub old_line: Option<u32>,
    pub new_line: Option<u32>,
    pub text: std::sync::Arc<str>,
    pub ending: LineEnding,
    pub kind: RowKind,
}
impl PatchRow {
    pub fn number(&self, side: Side) -> Option<u32> {
        match side {
            Side::Left => self.old_line,
            Side::Right => self.new_line,
        }
    }
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Hunk {
    pub old_start: u32,
    pub old_count: u32,
    pub new_start: u32,
    pub new_count: u32,
    pub section: String,
    pub rows: Vec<PatchRow>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileChange {
    pub id: FileId,
    pub old_path: Option<RepoPath>,
    pub new_path: Option<RepoPath>,
    pub kind: ChangeKind,
    pub content: ContentKind,
    pub old_mode: Option<String>,
    pub new_mode: Option<String>,
    pub old_oid: Option<String>,
    pub new_oid: Option<String>,
    pub hunks: Vec<Hunk>,
    pub metadata: Vec<String>,
}
impl FileChange {
    pub fn path(&self) -> &RepoPath {
        self.new_path
            .as_ref()
            .or(self.old_path.as_ref())
            .expect("parser requires a path")
    }
    pub fn display_path(&self) -> String {
        self.path().display()
    }
    pub fn line(&self, side: Side, line: u32) -> Option<&PatchRow> {
        self.hunks
            .iter()
            .flat_map(|h| &h.rows)
            .find(|r| r.number(side) == Some(line))
    }
    pub fn additions(&self) -> usize {
        self.hunks
            .iter()
            .flat_map(|h| &h.rows)
            .filter(|r| r.kind == RowKind::Added)
            .count()
    }
    pub fn deletions(&self) -> usize {
        self.hunks
            .iter()
            .flat_map(|h| &h.rows)
            .filter(|r| r.kind == RowKind::Removed)
            .count()
    }
    pub fn eligible(&self, side: Side, start: u32, end: u32) -> bool {
        self.content == ContentKind::Text
            && start > 0
            && start <= end
            && end - start <= 10_000
            && self
                .hunks
                .iter()
                .any(|h| (start..=end).all(|n| h.rows.iter().any(|r| r.number(side) == Some(n))))
    }
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PatchReport {
    pub files: Vec<FileChange>,
}
fn bad(line: usize, msg: impl Into<String>) -> PatchError {
    PatchError::Malformed {
        line: line + 1,
        message: msg.into(),
    }
}
fn utf8(v: &[u8], line: usize) -> Result<&str, PatchError> {
    std::str::from_utf8(v).map_err(|_| {
        bad(
            line,
            "source bytes are invalid UTF-8; this patch cannot render safely",
        )
    })
}
fn path(v: &[u8], line: usize, strip: bool) -> Result<RepoPath, PatchError> {
    let p = RepoPath::from_git_token(v).map_err(|e| bad(line, e))?;
    if strip {
        p.without_side().map_err(|e| bad(line, e))
    } else {
        Ok(p)
    }
}
fn header_paths(v: &[u8], line: usize) -> Result<(Option<RepoPath>, Option<RepoPath>), PatchError> {
    // Marker/rename paths decide; uncertain headers remain unresolved.
    if v.starts_with(b"\"") {
        let mut esc = false;
        let mut end = None;
        for (i, &b) in v.iter().enumerate().skip(1) {
            if esc {
                esc = false;
                continue;
            }
            if b == b'\\' {
                esc = true
            } else if b == b'"' {
                end = Some(i);
                break;
            }
        }
        if let Some(e) = end
            && v.get(e + 1) == Some(&b' ')
        {
            return Ok((
                Some(path(&v[..=e], line, true)?),
                Some(path(&v[e + 2..], line, true)?),
            ));
        }
        return Err(bad(line, "invalid quoted diff header"));
    }
    let positions: Vec<usize> = v
        .windows(3)
        .enumerate()
        .filter_map(|(i, w)| (w == b" b/").then_some(i))
        .collect();
    if positions.len() == 1 {
        let i = positions[0];
        return Ok((
            Some(path(&v[..i], line, true)?),
            Some(path(&v[i + 1..], line, true)?),
        ));
    }
    Ok((None, None))
}
fn pair(s: &str, line: usize) -> Result<(u32, u32), PatchError> {
    let (a, b) = s.split_once(',').unwrap_or((s, "1"));
    let start = a
        .parse::<u32>()
        .map_err(|_| bad(line, "invalid hunk start"))?;
    let count = b
        .parse::<u32>()
        .map_err(|_| bad(line, "invalid hunk count"))?;
    if count > 0 && (start == 0 || start.checked_add(count).is_none()) {
        return Err(bad(line, "invalid or overflowing hunk range"));
    }
    Ok((start, count))
}
fn hunk_header(v: &[u8], line: usize) -> Result<Hunk, PatchError> {
    let s = utf8(v, line)?;
    let rest = s
        .strip_prefix("@@ -")
        .ok_or_else(|| bad(line, "invalid hunk header"))?;
    let (old, rest) = rest
        .split_once(" +")
        .ok_or_else(|| bad(line, "missing new range"))?;
    let (new, section) = rest
        .split_once(" @@")
        .ok_or_else(|| bad(line, "unclosed hunk header"))?;
    let (old_start, old_count) = pair(old, line)?;
    let (new_start, new_count) = pair(new, line)?;
    Ok(Hunk {
        old_start,
        old_count,
        new_start,
        new_count,
        section: section.trim_start().into(),
        rows: vec![],
    })
}
pub fn parse_patch(bytes: &[u8], limits: ParseLimits) -> Result<PatchReport, PatchError> {
    if bytes.len() > limits.max_bytes {
        return Err(PatchError::Limit("bytes".into()));
    }
    let mut lines: Vec<&[u8]> = bytes.split(|b| *b == b'\n').collect();
    if lines.last() == Some(&b"".as_slice()) {
        lines.pop();
    }
    for (i, l) in lines.iter().enumerate() {
        if l.len() > limits.max_line_bytes {
            return Err(PatchError::Limit(format!("line {} length", i + 1)));
        }
        if l.starts_with(b"diff --cc ")
            || l.starts_with(b"diff --combined ")
            || l.starts_with(b"@@@ ")
        {
            return Err(PatchError::Unsupported("combined merge diff".into()));
        }
    }
    let mut i = 0;
    let mut files = vec![];
    let mut total_rows = 0;
    let mut seen = HashSet::new();
    while i < lines.len() {
        if !lines[i].starts_with(b"diff --git ") {
            if files.is_empty() {
                i += 1;
                continue;
            }
            if lines[i].is_empty() || lines[i] == b"-- " {
                i += 1;
                continue;
            }
            return Err(bad(i, "unexpected patch trailer"));
        }
        if files.len() >= limits.max_files {
            return Err(PatchError::Limit("file count".into()));
        }
        let (old_path, new_path) = header_paths(&lines[i][11..], i)?;
        let mut f = FileChange {
            id: FileId::default(),
            old_path,
            new_path,
            kind: ChangeKind::Modified,
            content: ContentKind::Text,
            old_mode: None,
            new_mode: None,
            old_oid: None,
            new_oid: None,
            hunks: vec![],
            metadata: vec![],
        };
        i += 1;
        while i < lines.len() && !lines[i].starts_with(b"diff --git ") {
            let l = lines[i];
            if l.starts_with(b"@@ ") {
                let mut h = hunk_header(l, i)?;
                i += 1;
                let (mut old, mut new) = (0u32, 0u32);
                if let Some(prev) = f.hunks.last() {
                    let prev: &Hunk = prev;
                    if h.old_start < prev.old_start.saturating_add(prev.old_count)
                        || h.new_start < prev.new_start.saturating_add(prev.new_count)
                    {
                        return Err(bad(i, "overlapping/out-of-order hunks"));
                    }
                }
                while old < h.old_count || new < h.new_count {
                    if i >= lines.len() {
                        return Err(bad(i, "truncated counted hunk"));
                    }
                    let l = lines[i];
                    if l == b"\\ No newline at end of file" {
                        let last = h
                            .rows
                            .last_mut()
                            .ok_or_else(|| bad(i, "orphan EOF marker"))?;
                        if last.ending == LineEnding::None {
                            return Err(bad(i, "duplicate EOF marker"));
                        }
                        last.ending = LineEnding::None;
                        i += 1;
                        continue;
                    }
                    let (&prefix, text) = l
                        .split_first()
                        .ok_or_else(|| bad(i, "unprefixed empty hunk row"))?;
                    let kind = match prefix {
                        b' ' => RowKind::Context,
                        b'+' => RowKind::Added,
                        b'-' => RowKind::Removed,
                        _ => return Err(bad(i, "unexpected hunk row; count mismatch")),
                    };
                    let old_line = if kind != RowKind::Added {
                        if old >= h.old_count {
                            return Err(bad(i, "too many old-side lines"));
                        }
                        let n = h
                            .old_start
                            .checked_add(old)
                            .ok_or_else(|| bad(i, "line overflow"))?;
                        old += 1;
                        Some(n)
                    } else {
                        None
                    };
                    let new_line = if kind != RowKind::Removed {
                        if new >= h.new_count {
                            return Err(bad(i, "too many new-side lines"));
                        }
                        let n = h
                            .new_start
                            .checked_add(new)
                            .ok_or_else(|| bad(i, "line overflow"))?;
                        new += 1;
                        Some(n)
                    } else {
                        None
                    };
                    let (text, ending) = if text.ends_with(b"\r") {
                        (&text[..text.len() - 1], LineEnding::CrLf)
                    } else {
                        (text, LineEnding::Lf)
                    };
                    total_rows += 1;
                    if total_rows > limits.max_rows {
                        return Err(PatchError::Limit("source row count".into()));
                    }
                    h.rows.push(PatchRow {
                        old_line,
                        new_line,
                        text: utf8(text, i)?.into(),
                        ending,
                        kind,
                    });
                    i += 1;
                }
                if i < lines.len() && lines[i] == b"\\ No newline at end of file" {
                    h.rows
                        .last_mut()
                        .ok_or_else(|| bad(i, "orphan EOF marker"))?
                        .ending = LineEnding::None;
                    i += 1;
                }
                f.hunks.push(h);
                continue;
            }
            if !f.hunks.is_empty() {
                if l.is_empty() {
                    i += 1;
                    continue;
                }
                if l == b"-- " {
                    i = lines.len();
                    break;
                }
                return Err(bad(i, "unexpected text after counted hunk"));
            }
            if let Some(v) = l.strip_prefix(b"--- ") {
                let v = v.split(|b| *b == b'\t').next().unwrap_or(v);
                f.old_path = if v == b"/dev/null" {
                    None
                } else {
                    Some(path(v, i, true)?)
                };
            } else if let Some(v) = l.strip_prefix(b"+++ ") {
                let v = v.split(|b| *b == b'\t').next().unwrap_or(v);
                f.new_path = if v == b"/dev/null" {
                    None
                } else {
                    Some(path(v, i, true)?)
                };
            } else if let Some(v) = l.strip_prefix(b"rename from ") {
                f.old_path = Some(path(v, i, false)?);
                f.kind = ChangeKind::Renamed;
            } else if let Some(v) = l.strip_prefix(b"rename to ") {
                f.new_path = Some(path(v, i, false)?);
                f.kind = ChangeKind::Renamed;
            } else if let Some(v) = l.strip_prefix(b"copy from ") {
                f.old_path = Some(path(v, i, false)?);
                f.kind = ChangeKind::Copied;
            } else if let Some(v) = l.strip_prefix(b"copy to ") {
                f.new_path = Some(path(v, i, false)?);
                f.kind = ChangeKind::Copied;
            } else if let Some(v) = l.strip_prefix(b"new file mode ") {
                f.new_mode = Some(utf8(v, i)?.into());
                f.kind = ChangeKind::Added;
                f.old_path = None;
            } else if let Some(v) = l.strip_prefix(b"deleted file mode ") {
                f.old_mode = Some(utf8(v, i)?.into());
                f.kind = ChangeKind::Deleted;
                f.new_path = None;
            } else if let Some(v) = l.strip_prefix(b"old mode ") {
                f.old_mode = Some(utf8(v, i)?.into());
            } else if let Some(v) = l.strip_prefix(b"new mode ") {
                f.new_mode = Some(utf8(v, i)?.into());
            } else if l.starts_with(b"Binary files ") || l == b"GIT binary patch" {
                f.content = ContentKind::Binary;
            } else if let Some(v) = l.strip_prefix(b"index ") {
                let s = utf8(v, i)?;
                let mut it = s.split_whitespace();
                if let Some((a, b)) = it.next().and_then(|p| p.split_once("..")) {
                    f.old_oid = Some(a.into());
                    f.new_oid = Some(b.into());
                }
                if let Some(mode) = it.next() {
                    f.old_mode = Some(mode.into());
                    f.new_mode = Some(mode.into());
                }
            } else if !l.is_empty() {
                f.metadata.push(utf8(l, i)?.into());
            }
            i += 1;
        }
        if f.old_path.is_none() && f.new_path.is_none() {
            return Err(bad(i, "file has no unambiguous path"));
        }
        if f.old_mode.as_deref() == Some("160000") || f.new_mode.as_deref() == Some("160000") {
            f.content = ContentKind::Submodule;
        }
        if f.kind == ChangeKind::Added {
            f.old_path = None
        }
        if f.kind == ChangeKind::Deleted {
            f.new_path = None
        }
        let a = f.old_path.as_ref().map_or(b"".as_slice(), RepoPath::bytes);
        let b = f.new_path.as_ref().map_or(b"".as_slice(), RepoPath::bytes);
        f.id = FileId(digest(&[b"file-v1", a, b]));
        if !seen.insert(f.id.clone()) {
            return Err(bad(i, "duplicate file change"));
        }
        files.push(f);
    }
    if files.is_empty() && !bytes.iter().all(u8::is_ascii_whitespace) {
        return Err(PatchError::Unsupported(
            "Git unified file headers are missing".into(),
        ));
    }
    Ok(PatchReport { files })
}
