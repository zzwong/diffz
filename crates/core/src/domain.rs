//! Source identity stays fixed and carries no UI, filesystem, network, clock, or credential state.
use crate::patch::{PatchReport, PatchRow};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{borrow::Cow, collections::BTreeMap, ops::Range};
use thiserror::Error;

macro_rules! identity {
    ($name:ident) => {
        #[derive(
            Debug, Clone, Default, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize,
        )]
        #[serde(transparent)]
        pub struct $name(pub String);
    };
}
identity!(SnapshotId);
identity!(FileId);
identity!(DraftId);
identity!(OperationId);

pub fn digest(parts: &[&[u8]]) -> String {
    let mut h = Sha256::new();
    for p in parts {
        h.update((p.len() as u64).to_le_bytes());
        h.update(p);
    }
    let mut hex = String::with_capacity(64);
    for byte in h.finalize() {
        const DIGITS: &[u8; 16] = b"0123456789abcdef";
        hex.push(DIGITS[(byte >> 4) as usize] as char);
        hex.push(DIGITS[(byte & 15) as usize] as char);
    }
    hex
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Side {
    Left,
    Right,
}
impl Side {
    pub fn api(self) -> &'static str {
        match self {
            Self::Left => "LEFT",
            Self::Right => "RIGHT",
        }
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum LineEnding {
    Lf,
    CrLf,
    None,
}
impl LineEnding {
    pub fn bytes(self) -> &'static [u8] {
        match self {
            Self::Lf => b"\n",
            Self::CrLf => b"\r\n",
            Self::None => b"",
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            Self::Lf => "LF",
            Self::CrLf => "CRLF",
            Self::None => "no final newline",
        }
    }
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceLine {
    pub content: Range<usize>,
    pub ending: LineEnding,
}
#[derive(Debug, Clone, Error, PartialEq, Eq)]
pub enum SourceError {
    #[error("source bytes are not UTF-8; binary/raw content remains undecoded")]
    InvalidUtf8,
    #[error("source is over 32 MiB; every byte remains accounted for")]
    TooLarge,
    #[error("logical line count exceeds 1,000,000")]
    TooManyLines,
    #[error("coordinate lies outside the source or splits a UTF-8 code point")]
    InvalidCoordinate,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(try_from = "Vec<u8>", into = "Vec<u8>")]
pub struct SourceDocument {
    raw: String,
    lines: Vec<SourceLine>,
}
impl TryFrom<Vec<u8>> for SourceDocument {
    type Error = SourceError;
    fn try_from(v: Vec<u8>) -> Result<Self, Self::Error> {
        Self::from_utf8(v)
    }
}
impl From<SourceDocument> for Vec<u8> {
    fn from(d: SourceDocument) -> Self {
        d.raw.into_bytes()
    }
}
impl SourceDocument {
    pub fn from_utf8(bytes: Vec<u8>) -> Result<Self, SourceError> {
        if bytes.len() > 32 * 1024 * 1024 {
            return Err(SourceError::TooLarge);
        }
        let raw = String::from_utf8(bytes).map_err(|_| SourceError::InvalidUtf8)?;
        let mut lines = Vec::new();
        let mut start = 0;
        for (i, b) in raw.bytes().enumerate() {
            if b == b'\n' {
                let cr = i > start && raw.as_bytes()[i - 1] == b'\r';
                lines.push(SourceLine {
                    content: start..if cr { i - 1 } else { i },
                    ending: if cr { LineEnding::CrLf } else { LineEnding::Lf },
                });
                start = i + 1;
                if lines.len() > 1_000_000 {
                    return Err(SourceError::TooManyLines);
                }
            }
        }
        if start < raw.len() {
            lines.push(SourceLine {
                content: start..raw.len(),
                ending: LineEnding::None,
            });
        }
        if lines.len() > 1_000_000 {
            return Err(SourceError::TooManyLines);
        }
        Ok(Self { raw, lines })
    }
    pub fn raw_bytes(&self) -> &[u8] {
        self.raw.as_bytes()
    }
    pub fn lines(&self) -> &[SourceLine] {
        &self.lines
    }
    pub fn line_text(&self, line: u32) -> Option<&str> {
        self.lines
            .get(line.checked_sub(1)? as usize)
            .map(|l| &self.raw[l.content.clone()])
    }
    pub fn offset(&self, line: u32, column: usize) -> Result<usize, SourceError> {
        let l = self
            .lines
            .get(line.checked_sub(1).ok_or(SourceError::InvalidCoordinate)? as usize)
            .ok_or(SourceError::InvalidCoordinate)?;
        if column > l.content.len() || !self.raw.is_char_boundary(l.content.start + column) {
            return Err(SourceError::InvalidCoordinate);
        }
        Ok(l.content.start + column)
    }
}
/// A Git path, kept as raw bytes because Git permits paths that are not UTF-8.
///
/// Serialized as a string when the bytes are UTF-8 and as a byte array otherwise.
/// Deserialization takes either form, so data stored as byte arrays still loads.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct RepoPath(Vec<u8>);
thread_local! {
    static LEGACY_PATH_ENCODING: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}
/// Runs `f` with every [`RepoPath`] serialized as a byte array, as it was before paths became strings.
/// Identity digests hash that encoding so stored snapshot IDs keep verifying.
fn with_legacy_path_encoding<T>(f: impl FnOnce() -> T) -> T {
    struct Restore(bool);
    impl Drop for Restore {
        fn drop(&mut self) {
            LEGACY_PATH_ENCODING.set(self.0);
        }
    }
    let _restore = Restore(LEGACY_PATH_ENCODING.replace(true));
    f()
}
impl Serialize for RepoPath {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        match std::str::from_utf8(&self.0) {
            Ok(text) if !LEGACY_PATH_ENCODING.get() => serializer.serialize_str(text),
            _ => self.0.serialize(serializer),
        }
    }
}
impl<'de> Deserialize<'de> for RepoPath {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct PathVisitor;
        impl<'de> serde::de::Visitor<'de> for PathVisitor {
            type Value = RepoPath;
            fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
                f.write_str("a repository path as a string or an array of bytes")
            }
            fn visit_str<E: serde::de::Error>(self, v: &str) -> Result<RepoPath, E> {
                RepoPath::new(v.as_bytes().to_vec()).map_err(E::custom)
            }
            fn visit_bytes<E: serde::de::Error>(self, v: &[u8]) -> Result<RepoPath, E> {
                RepoPath::new(v.to_vec()).map_err(E::custom)
            }
            fn visit_seq<A: serde::de::SeqAccess<'de>>(
                self,
                mut seq: A,
            ) -> Result<RepoPath, A::Error> {
                let mut bytes = Vec::with_capacity(seq.size_hint().unwrap_or(0).min(4096));
                while let Some(b) = seq.next_element::<u8>()? {
                    bytes.push(b);
                }
                RepoPath::new(bytes).map_err(serde::de::Error::custom)
            }
        }
        deserializer.deserialize_any(PathVisitor)
    }
}
impl TryFrom<Vec<u8>> for RepoPath {
    type Error = String;
    fn try_from(v: Vec<u8>) -> Result<Self, String> {
        Self::new(v)
    }
}
impl From<RepoPath> for Vec<u8> {
    fn from(v: RepoPath) -> Self {
        v.0
    }
}
impl RepoPath {
    pub fn new(bytes: Vec<u8>) -> Result<Self, String> {
        if bytes.is_empty()
            || bytes.contains(&0)
            || bytes.starts_with(b"/")
            || bytes
                .split(|b| *b == b'/')
                .any(|p| p.is_empty() || p == b"." || p == b"..")
        {
            return Err("invalid relative repository path".into());
        }
        Ok(Self(bytes))
    }
    pub fn bytes(&self) -> &[u8] {
        &self.0
    }
    pub fn utf8(&self) -> Result<&str, String> {
        std::str::from_utf8(&self.0).map_err(|_| "GitHub requires a UTF-8 path".into())
    }
    pub fn display(&self) -> String {
        match std::str::from_utf8(&self.0) {
            Ok(s) => s
                .chars()
                .flat_map(|c| {
                    if c.is_control() {
                        c.escape_default().collect::<Vec<_>>()
                    } else {
                        vec![c]
                    }
                })
                .collect(),
            Err(_) => self
                .0
                .iter()
                .map(|b| {
                    if b.is_ascii_graphic() || *b == b' ' {
                        (*b as char).to_string()
                    } else {
                        format!("\\x{b:02x}")
                    }
                })
                .collect(),
        }
    }
    /// Quoted paths use Git's C syntax. Spaces stay literal in unquoted paths.
    pub fn from_git_token(v: &[u8]) -> Result<Self, String> {
        if !v.starts_with(b"\"") {
            return Self::new(v.to_vec());
        }
        if v.len() < 2 || v.last() != Some(&b'"') {
            return Err("unterminated quoted path".into());
        }
        let mut out = Vec::new();
        let mut i = 1;
        while i < v.len() - 1 {
            let b = v[i];
            i += 1;
            if b == b'"' {
                return Err("unescaped quote in path".into());
            }
            if b != b'\\' {
                out.push(b);
                continue;
            }
            if i >= v.len() - 1 {
                return Err("trailing path escape".into());
            }
            let c = v[i];
            i += 1;
            match c {
                b'0'..=b'7' => {
                    let mut n = (c - b'0') as u16;
                    let mut used = 1;
                    while used < 3 && i < v.len() - 1 && (b'0'..=b'7').contains(&v[i]) {
                        n = n * 8 + (v[i] - b'0') as u16;
                        i += 1;
                        used += 1
                    }
                    if n > 255 {
                        return Err("octal path escape is larger than one byte".into());
                    }
                    out.push(n as u8)
                }
                b'a' => out.push(7),
                b'b' => out.push(8),
                b't' => out.push(9),
                b'n' => out.push(10),
                b'v' => out.push(11),
                b'f' => out.push(12),
                b'r' => out.push(13),
                b'\\' | b'"' => out.push(c),
                _ => return Err("invalid Git path escape".into()),
            }
        }
        Self::new(out)
    }
    pub fn without_side(self) -> Result<Self, String> {
        if self.0.starts_with(b"a/") || self.0.starts_with(b"b/") {
            Self::new(self.0[2..].to_vec())
        } else {
            Ok(self)
        }
    }
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SourcePoint {
    pub snapshot: SnapshotId,
    pub file: FileId,
    pub side: Side,
    pub line: u32,
    pub byte_column: usize,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SourceSelection {
    pub start: SourcePoint,
    pub end: SourcePoint,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RepositoryKey {
    pub host: String,
    pub id: u64,
    pub owner: String,
    pub name: String,
}
/// Names the review provider that owns a remote target. Stored snapshots and outbox
/// entries hold it as a plain string, so a provider that is no longer registered still
/// deserializes; only its reads and publication become unavailable.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ProviderId(Cow<'static, str>);
impl ProviderId {
    pub const GITHUB: ProviderId = ProviderId(Cow::Borrowed("GitHub"));
    pub const GITLAB: ProviderId = ProviderId(Cow::Borrowed("GitLab"));
    pub fn new(id: impl Into<String>) -> Self {
        Self(Cow::Owned(id.into()))
    }
    pub fn as_str(&self) -> &str {
        &self.0
    }
    pub fn is_github(&self) -> bool {
        *self == Self::GITHUB
    }
    /// Domain separation for snapshot identities. GitHub keeps the original tag, so
    /// identities computed before providers were pluggable stay valid.
    fn identity_tag(&self) -> Vec<u8> {
        if self.is_github() {
            b"snapshot-v1".to_vec()
        } else {
            format!("snapshot-{}-v1", self.0.to_ascii_lowercase()).into_bytes()
        }
    }
}
impl Default for ProviderId {
    fn default() -> Self {
        Self::GITHUB
    }
}
impl std::fmt::Display for ProviderId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RemoteTarget {
    #[serde(default, skip_serializing_if = "ProviderId::is_github")]
    pub provider: ProviderId,
    pub repository: RepositoryKey,
    pub account: String,
    pub pr: u64,
    pub target_tip: String,
    pub comparison_base: String,
    pub head: String,
    pub open: bool,
    #[serde(default)]
    pub draft: bool,
    pub pending_review: bool,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ThreadComment {
    pub id: u64,
    pub root_id: u64,
    pub path: String,
    pub side: Option<Side>,
    pub line: Option<u32>,
    pub start_line: Option<u32>,
    pub body: String,
    pub author: String,
    pub commit_id: String,
    /// Creation timestamp from the provider, RFC 3339. Snapshots made before this field existed leave it empty.
    #[serde(default)]
    pub created_at: Option<String>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Snapshot {
    pub id: SnapshotId,
    pub title: String,
    pub origin: String,
    pub patch: PatchReport,
    pub remote: Option<RemoteTarget>,
    pub comments: Vec<ThreadComment>,
    #[serde(default)]
    pub overview: crate::review_details::Overview,
    /// Provider patch/file listing may be incomplete; full blobs are not promised.
    pub warnings: Vec<String>,
}
impl Snapshot {
    pub fn new(
        title: String,
        patch: PatchReport,
        remote: Option<RemoteTarget>,
        comments: Vec<ThreadComment>,
    ) -> Self {
        Self::with_origin(title, patch, remote, comments, "patch-bytes".into())
    }
    pub fn with_origin(
        title: String,
        patch: PatchReport,
        remote: Option<RemoteTarget>,
        comments: Vec<ThreadComment>,
        origin: String,
    ) -> Self {
        let id = Self::identity(&patch, remote.as_ref(), &origin);
        Self {
            id,
            title,
            origin,
            patch,
            remote,
            comments,
            overview: Default::default(),
            warnings: vec![],
        }
    }
    fn identity(patch: &PatchReport, remote: Option<&RemoteTarget>, origin: &str) -> SnapshotId {
        // Identity input leaves out title changes, pending status, comment data, and account display.
        // Paths keep their original byte-array encoding here so existing snapshot IDs stay valid.
        let bytes = with_legacy_path_encoding(|| {
            serde_json::to_vec(&(
                origin,
                patch,
                remote.map(|r| {
                    (
                        &r.repository,
                        r.pr,
                        &r.target_tip,
                        &r.comparison_base,
                        &r.head,
                        &r.account,
                    )
                }),
            ))
        })
        .expect("serializing only string/integer source data");
        let tag = remote.map_or_else(
            || ProviderId::GITHUB.identity_tag(),
            |r| r.provider.identity_tag(),
        );
        SnapshotId(digest(&[&tag, &bytes]))
    }
    pub fn verify_identity(&self) -> bool {
        self.id == Self::identity(&self.patch, self.remote.as_ref(), &self.origin)
    }
    pub fn file(&self, id: &FileId) -> Option<&crate::patch::FileChange> {
        self.patch.files.iter().find(|f| &f.id == id)
    }
    pub fn validate_point(&self, p: &SourcePoint) -> Result<&PatchRow, String> {
        if self.id != p.snapshot {
            return Err("source point comes from a different snapshot".into());
        }
        let r = self
            .file(&p.file)
            .and_then(|f| f.line(p.side, p.line))
            .ok_or("source line is not loaded")?;
        if !r.text.is_char_boundary(p.byte_column) {
            return Err("invalid UTF-8 source column".into());
        }
        Ok(r)
    }
    pub fn copy_selection(&self, s: &SourceSelection) -> Result<String, String> {
        if s.start.snapshot != s.end.snapshot
            || s.start.file != s.end.file
            || s.start.side != s.end.side
        {
            return Err("selection spans multiple files, snapshots, or sides".into());
        }
        self.validate_point(&s.start)?;
        self.validate_point(&s.end)?;
        let (a, b) = if (s.start.line, s.start.byte_column) <= (s.end.line, s.end.byte_column) {
            (&s.start, &s.end)
        } else {
            (&s.end, &s.start)
        };
        let file = self.file(&a.file).ok_or("missing file")?;
        let mut out = String::new();
        for n in a.line..=b.line {
            let row = file
                .line(a.side, n)
                .ok_or("selection includes unloaded context; choose a smaller range")?;
            let lo = if n == a.line { a.byte_column } else { 0 };
            let hi = if n == b.line {
                b.byte_column
            } else {
                row.text.len()
            };
            out.push_str(row.text.get(lo..hi).ok_or("invalid source selection")?);
            if n != b.line {
                out.push_str(std::str::from_utf8(row.ending.bytes()).expect("ASCII line ending"));
            }
        }
        Ok(out)
    }
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Draft {
    pub id: DraftId,
    pub snapshot: SnapshotId,
    pub file: FileId,
    pub side: Side,
    pub start_line: u32,
    pub line: u32,
    /// A draft whose comment covers the whole file, not one source line.
    /// The side is always [`Side::Right`], and start_line is zero and line is zero.
    /// Validation checks only that the path exists in the snapshot,
    /// and skips the bounds check for line and source point.
    #[serde(default)]
    pub file_level: bool,
    pub body: String,
    pub version: u64,
    pub saved_version: u64,
    pub published: bool,
}
impl Draft {
    pub fn is_saved(&self) -> bool {
        self.version == self.saved_version
    }
    pub fn is_file_level(&self) -> bool {
        self.file_level
    }
}
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Preferences {
    pub viewed: BTreeMap<String, bool>,
}
/// Review-wide options for wrapping, text size, layout, and appearance.
/// The state directory stores these settings once rather than with each snapshot.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub split: bool,
    /// A value of `None` lets the file type decide (`presentation::default_wrap`).
    pub wrap: Option<bool>,
    pub font_size: f32,
    pub dark: bool,
    /// A theme selector: a name inside the theme dirs, a path, or the string `"current"`. `None` means built-in.
    pub theme: Option<String>,
    /// Show Markdown prose in paired rendered blocks, not raw source lines.
    pub rich: bool,
    /// Experimental: flag words that differ inside each changed rich-diff block.
    pub rich_inline: bool,
}
impl Default for Settings {
    fn default() -> Self {
        Self {
            split: false,
            wrap: None,
            font_size: 14.0,
            dark: true,
            theme: None,
            rich: true,
            rich_inline: false,
        }
    }
}

#[cfg(test)]
mod digest_tests {
    #[test]
    fn stable_identity_encoding() {
        assert_eq!(
            super::digest(&[b"file-v1", b"", b"docs/guide.md"]),
            "20cc8afbde14bc9cc262bf7b9303cbe9180fd2997f6f7b5c43b19741d3a2de84"
        );
    }
}
