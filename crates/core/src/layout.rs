//! Source mapping independent of the text engine. Native code provides the measured shaped geometry.
use std::ops::Range;
use thiserror::Error;
use unicode_segmentation::UnicodeSegmentation;
#[derive(Debug, Clone, Error, PartialEq, Eq)]
pub enum MappingError {
    #[error("selection is outside the source")]
    OutOfBounds,
    #[error("selection breaks UTF-8 character boundaries")]
    InvalidBoundary,
    #[error("measured fragments leave part of the source uncovered")]
    Incomplete,
}
pub fn copy_source_range(raw: &[u8], range: Range<usize>) -> Result<String, MappingError> {
    let full = std::str::from_utf8(raw).map_err(|_| MappingError::InvalidBoundary)?;
    if range.start > range.end || range.end > raw.len() {
        return Err(MappingError::OutOfBounds);
    }
    full.get(range)
        .map(str::to_owned)
        .ok_or(MappingError::InvalidBoundary)
}
#[derive(Debug, Clone)]
pub struct VisualFragment {
    pub bytes: Range<usize>,
    pub y: f32,
    pub height: f32,
}
#[derive(Debug, Clone)]
pub struct MeasuredLine {
    pub fragments: Vec<VisualFragment>,
    pub height: f32,
}
impl MeasuredLine {
    pub fn validate(&self, text: &str) -> Result<(), MappingError> {
        let mut end = 0;
        let mut y = 0.;
        if self.fragments.is_empty() {
            return Err(MappingError::Incomplete);
        }
        for f in &self.fragments {
            if f.bytes.start != end
                || text.get(f.bytes.clone()).is_none()
                || !f.height.is_finite()
                || f.height <= 0.
                || !f.y.is_finite()
                || f.y < y
            {
                return Err(MappingError::Incomplete);
            }
            end = f.bytes.end;
            y = f.y + f.height;
        }
        if end != text.len() || !self.height.is_finite() || self.height + 0.01 < y {
            return Err(MappingError::Incomplete);
        }
        Ok(())
    }
    pub fn fragment_for_byte(&self, byte: usize) -> Option<usize> {
        self.fragments
            .iter()
            .position(|f| f.bytes.start <= byte && byte < f.bytes.end)
            .or_else(|| {
                self.fragments
                    .last()
                    .filter(|f| byte == f.bytes.end)
                    .map(|_| self.fragments.len() - 1)
            })
    }
}
pub fn split_row_height(left: f32, right: f32) -> f32 {
    left.max(right)
}
pub fn in_source_height(y: f32, height: f32) -> bool {
    y >= 0. && y < height
}
pub fn snap_grapheme(text: &str, byte: usize) -> usize {
    if byte >= text.len() {
        return text.len();
    }
    text.grapheme_indices(true)
        .take_while(|(i, _)| *i <= byte)
        .last()
        .map_or(0, |(i, _)| i)
}
#[derive(Debug, Clone)]
pub struct DisplayText {
    pub text: String,
    map: Vec<(usize, usize)>,
}
impl DisplayText {
    /// Tabs expand only; each display boundary maps to original byte offsets.
    pub fn new(source: &str, tab_size: usize) -> Self {
        if !source.contains('\t') {
            return Self {
                text: source.into(),
                map: vec![],
            };
        }
        let mut text = String::new();
        let mut map = vec![];
        let mut column = 0;
        let tab_size = tab_size.clamp(1, 16);
        for (i, c) in source.char_indices() {
            if c == '\t' {
                let n = tab_size - column % tab_size;
                for _ in 0..n {
                    map.push((text.len(), i));
                    text.push(' ');
                }
                column += n;
            } else {
                map.push((text.len(), i));
                text.push(c);
                column += 1;
            }
        }
        map.push((text.len(), source.len()));
        Self { text, map }
    }
    pub fn estimated_bytes(&self) -> usize {
        self.text.capacity() + self.map.capacity() * std::mem::size_of::<(usize, usize)>()
    }
    pub fn source_byte(&self, display: usize) -> usize {
        if self.map.is_empty() {
            return display.min(self.text.len());
        }
        self.map
            .get(
                self.map
                    .partition_point(|(d, _)| *d <= display)
                    .saturating_sub(1),
            )
            .map_or(0, |(_, s)| *s)
    }
    pub fn display_byte(&self, source: usize) -> usize {
        if self.map.is_empty() {
            return source.min(self.text.len());
        }
        self.map
            .get(self.map.partition_point(|(_, s)| *s < source))
            .map_or(self.text.len(), |(d, _)| *d)
    }
}
