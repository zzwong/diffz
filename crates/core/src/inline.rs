//! Optional word markup: token ranges and word-level diffs with bounds.
use std::ops::Range;
const MAX_BYTES: usize = 4096;
const MAX_TOKENS: usize = 512;
fn tokens(s: &str, max_tokens: usize) -> Option<Vec<Range<usize>>> {
    let mut out: Vec<Range<usize>> = vec![];
    let mut chars = s.char_indices().peekable();
    while let Some(&(start, c)) = chars.peek() {
        let word = c.is_alphanumeric() || c == '_';
        let space = c.is_whitespace();
        let mut end = start + c.len_utf8();
        chars.next();
        while let Some(&(i, next)) = chars.peek() {
            let next_word = next.is_alphanumeric() || next == '_';
            if (word && next_word) || (space && next.is_whitespace()) {
                end = i + next.len_utf8();
                chars.next();
            } else {
                break;
            }
        }
        out.push(start..end);
        if out.len() > max_tokens {
            return None;
        }
    }
    Some(out)
}
fn coalesce(v: Vec<Range<usize>>) -> Vec<Range<usize>> {
    let mut out: Vec<Range<usize>> = vec![];
    for r in v {
        if let Some(p) = out.last_mut()
            && p.end == r.start
        {
            p.end = r.end;
            continue;
        }
        out.push(r)
    }
    out
}
type Ranges = Vec<Range<usize>>;
/// Word marks for a single source row per side. The result is blank once the row is
/// oversized, or so much changed that marking it adds noise, since the row colour flags the change already.
pub fn word_diff(old: &str, new: &str) -> (Ranges, Ranges) {
    if old.len() > MAX_BYTES || new.len() > MAX_BYTES {
        return (vec![], vec![]);
    }
    let (Some(a), Some(b)) = (tokens(old, MAX_TOKENS), tokens(new, MAX_TOKENS)) else {
        return (vec![], vec![]);
    };
    fine(old, new, &a, &b).unwrap_or_default()
}
/// Prose word marks with caller-selected bounds. As opposed to [`word_diff`], this never returns
/// empty marks for differing texts. It strips the shared prefix and suffix, then pairs tokens
/// one by one while `max_tokens` holds, or falls back to sentences first and then
/// refines the words within each sentence pair that changed.
pub fn word_diff_with(
    old: &str,
    new: &str,
    max_bytes: usize,
    max_tokens: usize,
) -> (Ranges, Ranges) {
    if old.len() > max_bytes || new.len() > max_bytes {
        return (vec![], vec![]);
    }
    // Trailing whitespace never gets marked, so drop it before comparing suffixes.
    let (old, new) = (old.trim_end(), new.trim_end());
    let (Some(a), Some(b)) = (tokens(old, usize::MAX), tokens(new, usize::MAX)) else {
        return (vec![], vec![]);
    };
    // Most edits stay local, so strip the shared prefix and suffix before anything else.
    let same = |i: usize, j: usize| old[a[i].clone()] == new[b[j].clone()];
    let mut prefix = 0;
    while prefix < a.len() && prefix < b.len() && same(prefix, prefix) {
        prefix += 1;
    }
    let mut suffix = 0;
    while prefix + suffix < a.len()
        && prefix + suffix < b.len()
        && same(a.len() - 1 - suffix, b.len() - 1 - suffix)
    {
        suffix += 1;
    }
    let a = &a[prefix..a.len() - suffix];
    let b = &b[prefix..b.len() - suffix];
    if a.is_empty() && b.is_empty() {
        return (vec![], vec![]);
    }
    if a.len() <= max_tokens && b.len() <= max_tokens {
        return fine(old, new, a, b).unwrap_or_else(|| (span(a), span(b)));
    }
    coarse(old, new, a, b, max_tokens)
}
fn span(t: &[Range<usize>]) -> Ranges {
    t.first()
        .map(|f| f.start..t[t.len() - 1].end)
        .into_iter()
        .collect()
}
/// LCS at token level. `None` once either side has more than 70% changed.
fn fine(old: &str, new: &str, a: &[Range<usize>], b: &[Range<usize>]) -> Option<(Ranges, Ranges)> {
    let ta: Vec<&str> = a.iter().map(|r| &old[r.clone()]).collect();
    let tb: Vec<&str> = b.iter().map(|r| &new[r.clone()]).collect();
    let matches = lcs(&ta, &tb);
    let (mut ra, mut rb) = (vec![], vec![]);
    let (mut i, mut j) = (0, 0);
    for (mi, mj) in matches.iter().copied().chain([(a.len(), b.len())]) {
        ra.extend(a[i..mi].iter().cloned());
        rb.extend(b[j..mj].iter().cloned());
        i = (mi + 1).min(a.len());
        j = (mj + 1).min(b.len());
    }
    let total = |t: &[Range<usize>]| t.iter().map(Range::len).sum::<usize>();
    let fraction =
        |ranges: &[Range<usize>], len: usize| len > 0 && total(ranges) as f32 / len as f32 > 0.7;
    if fraction(&ra, total(a)) || fraction(&rb, total(b)) {
        None
    } else {
        Some((coalesce(ra), coalesce(rb)))
    }
}
/// For long paragraphs, this pass first pairs sentences, then refines the words of
/// each changed pair, and flags any sentence left without a partner.
fn coarse(
    old: &str,
    new: &str,
    a: &[Range<usize>],
    b: &[Range<usize>],
    max_tokens: usize,
) -> (Ranges, Ranges) {
    let ca = sentences(old, a);
    let cb = sentences(new, b);
    if ca.len() * cb.len() > 4_000_000 {
        return (span(a), span(b));
    }
    let ta: Vec<&str> = ca.iter().map(|c| old[c.bytes.clone()].trim()).collect();
    let tb: Vec<&str> = cb.iter().map(|c| new[c.bytes.clone()].trim()).collect();
    let matches = lcs(&ta, &tb);
    let (mut ra, mut rb) = (vec![], vec![]);
    let (mut i, mut j) = (0, 0);
    for (mi, mj) in matches.iter().copied().chain([(ca.len(), cb.len())]) {
        let paired = (mi - i).min(mj - j);
        for t in 0..paired {
            let (x, y) = (&ca[i + t], &cb[j + t]);
            let (ta, tb) = (&a[x.tokens.clone()], &b[y.tokens.clone()]);
            let refined = (ta.len() <= max_tokens && tb.len() <= max_tokens)
                .then(|| fine(old, new, ta, tb))
                .flatten();
            match refined {
                Some((xa, xb)) => {
                    ra.extend(xa);
                    rb.extend(xb);
                }
                None => {
                    ra.push(x.bytes.clone());
                    rb.push(y.bytes.clone());
                }
            }
        }
        ra.extend(ca[i + paired..mi].iter().map(|c| c.bytes.clone()));
        rb.extend(cb[j + paired..mj].iter().map(|c| c.bytes.clone()));
        i = (mi + 1).min(ca.len());
        j = (mj + 1).min(cb.len());
    }
    (coalesce(ra), coalesce(rb))
}
struct Sentence {
    bytes: Range<usize>,
    tokens: Range<usize>,
}
/// Chunks that approximate sentences over a token list. A chunk stops after `.`, `!`, or `?` when
/// whitespace comes next, or at whitespace holding a newline.
fn sentences(text: &str, toks: &[Range<usize>]) -> Vec<Sentence> {
    let mut out = vec![];
    let mut start = 0;
    for (i, t) in toks.iter().enumerate() {
        let piece = &text[t.clone()];
        let terminal = matches!(piece, "." | "!" | "?")
            && toks
                .get(i + 1)
                .is_some_and(|n| text[n.clone()].chars().all(char::is_whitespace));
        if terminal || piece.contains('\n') || i + 1 == toks.len() {
            out.push(Sentence {
                bytes: toks[start].start..t.end,
                tokens: start..i + 1,
            });
            start = i + 1;
        }
    }
    out
}
/// The matched index pairs, in order, from the longest common subsequence.
fn lcs(a: &[&str], b: &[&str]) -> Vec<(usize, usize)> {
    if a.is_empty() || b.is_empty() {
        return vec![];
    }
    let cols = b.len() + 1;
    let mut dp = vec![0u32; (a.len() + 1) * cols];
    for i in (0..a.len()).rev() {
        for j in (0..b.len()).rev() {
            dp[i * cols + j] = if a[i] == b[j] {
                1 + dp[(i + 1) * cols + j + 1]
            } else {
                dp[(i + 1) * cols + j].max(dp[i * cols + j + 1])
            };
        }
    }
    let (mut i, mut j) = (0, 0);
    let mut out = vec![];
    while i < a.len() && j < b.len() {
        if a[i] == b[j] {
            out.push((i, j));
            i += 1;
            j += 1;
        } else if dp[(i + 1) * cols + j] >= dp[i * cols + j + 1] {
            i += 1;
        } else {
            j += 1;
        }
    }
    out
}
#[cfg(test)]
mod tests {
    use super::*;
    use std::ops::Range;
    #[test]
    fn tokens_cover_without_gaps() {
        let s = "let naïve = 1;";
        let toks = tokens(s, usize::MAX).unwrap();
        let ranges: Vec<Range<usize>> =
            vec![0..3, 3..4, 4..10, 10..11, 11..12, 12..13, 13..14, 14..15];
        assert_eq!(toks, ranges);
        let words = ["let", " ", "naïve", " ", "=", " ", "1", ";"];
        for (r, w) in toks.iter().zip(words) {
            assert_eq!(&s[r.clone()], w);
        }
        let mut prev = 0;
        for r in &toks {
            assert_eq!(r.start, prev);
            prev = r.end;
        }
        assert_eq!(prev, s.len());
    }
    #[test]
    fn whitespace_run_with_newline_is_one_token() {
        let toks = tokens("a\n\n b", usize::MAX).unwrap();
        assert_eq!(toks.len(), 3);
        assert!("a\n\n b"[toks[1].clone()].contains('\n'));
    }
    #[test]
    fn max_tokens_exceeded_returns_none() {
        assert_eq!(tokens("a b c", 2), None);
        assert_eq!(tokens("a b c", 5).unwrap().len(), 5);
    }
    #[test]
    fn empty_input_has_no_tokens() {
        assert_eq!(tokens("", 0), Some(vec![]));
        assert_eq!(tokens("", usize::MAX), Some(vec![]));
    }
    #[test]
    fn mixed_script_ranges_are_char_boundaries() {
        let s = "日本語 text_1";
        let toks = tokens(s, usize::MAX).unwrap();
        assert!(!toks.is_empty());
        for r in &toks {
            assert!(s.get(r.clone()).is_some());
        }
    }
}
