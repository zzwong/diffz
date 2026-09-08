//! Source mapping independent of the text engine. Native code provides the measured shaped geometry.
use std::ops::Range;
use thiserror::Error;
use unicode_segmentation::GraphemeCursor;
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
    // Start at the requested position. Unicode rules inspect preceding context
    // where needed, without rescanning an ordinary line for every pointer hit.
    let byte = text.floor_char_boundary(byte);
    let mut cursor = GraphemeCursor::new(byte, text.len(), true);
    if cursor
        .is_boundary(text, 0)
        .expect("the full source provides all grapheme context")
    {
        byte
    } else {
        cursor
            .prev_boundary(text, 0)
            .expect("the full source provides all grapheme context")
            .unwrap_or(0)
    }
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

#[cfg(test)]
mod grapheme_tests {
    use super::snap_grapheme;
    use unicode_segmentation::UnicodeSegmentation;

    #[test]
    fn snapping_matches_extended_graphemes_at_every_byte_offset() {
        let samples = [
            "",
            "abc\r\n\0",
            "e\u{301}",
            "\u{600}a",
            "👩🏽‍🚀",
            "👨‍👩‍👧‍👦",
            "🇺🇸🇨🇦🇬",
            "한",
            "क्षि",
            "क्‍ष",
            "\u{301}\u{308}",
            "\u{200d}",
            "\u{202e}אב\u{202c}",
            "🏴\u{e0067}\u{e0062}\u{e007f}",
        ];
        // Adjacent samples also exercise grapheme rules across their join.
        for left in samples {
            for right in samples {
                let text = format!("{left}{right}");
                let mut boundaries = text
                    .grapheme_indices(true)
                    .map(|(i, _)| i)
                    .collect::<Vec<_>>();
                boundaries.push(text.len());
                for byte in (0..=text.len() + 1).chain([usize::MAX]) {
                    let expected = boundaries[boundaries.partition_point(|i| *i <= byte) - 1];
                    assert_eq!(snap_grapheme(&text, byte), expected, "{text:?} byte={byte}");
                }
            }
        }
    }
    // Isolated snapping CPU cost, not native shaping or end-to-end pointer latency.
    // Run with: cargo test -p diffz-core --release --lib benchmark_grapheme_snapping -- --ignored --nocapture
    #[test]
    #[ignore]
    fn benchmark_grapheme_snapping() {
        use std::{hint::black_box, time::Instant};
        for (text, iterations) in [
            ("a".repeat(80), 10000),
            ("a".repeat(10000), 2000),
            ("a".repeat(288017), 1000),
            ("e\u{301}👩🏽‍🚀".repeat(1000), 2000),
        ] {
            let byte = text.len() - 2;
            let started = Instant::now();
            for _ in 0..iterations {
                black_box(snap_grapheme(black_box(&text), black_box(byte)));
            }
            println!(
                "grapheme_benchmark bytes={} iterations={iterations} ns_per_hit={:.0}",
                text.len(),
                started.elapsed().as_nanos() as f64 / iterations as f64
            );
        }
    }
}
