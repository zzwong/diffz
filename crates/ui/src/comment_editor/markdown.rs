use std::ops::Range;

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum Format {
    Heading,
    Bold,
    Italic,
    Quote,
    InlineCode,
    CodeBlock,
    Link,
    Bulleted,
    Numbered,
    Checklist,
}

/// Replacement text and the caret (or selected content) relative to its start.
pub(crate) struct Edit {
    pub text: String,
    pub selection: Range<usize>,
}

impl Edit {
    pub(super) fn caret(text: String, offset: usize) -> Self {
        Self {
            text,
            selection: offset..offset,
        }
    }
}

pub(crate) fn format(action: Format, selected: &str) -> Edit {
    match action {
        Format::Bold => wrap(selected, "**", "**"),
        Format::Italic => wrap(selected, "*", "*"),
        Format::InlineCode => wrap(selected, "`", "`"),
        Format::Link => {
            let text = format!("[{selected}](url)");
            if selected.is_empty() {
                Edit::caret(text, 1)
            } else {
                Edit {
                    text,
                    selection: selected.len() + 3..selected.len() + 6,
                }
            }
        }
        Format::Heading => prefix_lines(selected, "## "),
        Format::Quote => prefix_lines(selected, "> "),
        Format::Bulleted => prefix_lines(selected, "- "),
        Format::Numbered => prefix_lines(selected, "1. "),
        Format::Checklist => prefix_lines(selected, "- [ ] "),
        Format::CodeBlock => code_block("", selected),
    }
}

fn wrap(selected: &str, before: &str, after: &str) -> Edit {
    let text = format!("{before}{selected}{after}");
    Edit {
        text,
        selection: before.len()..before.len() + selected.len(),
    }
}

fn prefix_lines(selected: &str, prefix: &str) -> Edit {
    let text = selected
        .lines()
        .map(|line| format!("{prefix}{line}"))
        .collect::<Vec<_>>()
        .join("\n");
    let text = if text.is_empty() {
        prefix.to_string()
    } else {
        text
    };
    let at = text.len();
    Edit::caret(text, at)
}

pub(crate) fn code_block(language: &str, selected: &str) -> Edit {
    let text = format!("```{language}\n{selected}\n```");
    if selected.is_empty() {
        Edit::caret(text, 4 + language.len())
    } else {
        Edit {
            text,
            selection: 4 + language.len()..4 + language.len() + selected.len(),
        }
    }
}

pub(crate) fn table(columns: usize, rows: usize) -> Edit {
    let columns = columns.clamp(1, 6);
    let rows = rows.clamp(1, 6);
    let header = format!(
        "| {} |",
        (1..=columns)
            .map(|n| format!("Column {n}"))
            .collect::<Vec<_>>()
            .join(" | ")
    );
    let separator = format!("| {} |", vec!["---"; columns].join(" | "));
    let empty = format!("| {} |", vec![" "; columns].join(" | "));
    let text = format!("{header}\n{separator}\n{}", vec![empty; rows].join("\n"));
    let at = header.len() + 1 + separator.len() + 3;
    Edit::caret(text, at)
}

/// Find a slash token immediately before the caret, without matching URLs or paths.
pub(crate) fn slash_query(text: &str, caret: usize) -> Option<(Range<usize>, String)> {
    let before = text.get(..caret)?;
    let start = before.rfind('/')?;
    if start > 0 && !before[..start].ends_with(char::is_whitespace) {
        return None;
    }
    let query = &before[start + 1..];
    if query.contains(char::is_whitespace) || query.contains('\n') {
        return None;
    }
    Some((start..caret, query.to_ascii_lowercase()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use core::prelude::v1::test;

    #[test]
    fn link_places_caret_in_label_and_selects_url_for_existing_text() {
        let empty = format(Format::Link, "");
        assert_eq!(empty.text, "[](url)");
        assert_eq!(empty.selection, 1..1);
        let named = format(Format::Link, "docs");
        assert_eq!(named.text, "[docs](url)");
        assert_eq!(named.selection, 7..10);
    }

    #[test]
    fn table_and_code_place_caret_in_editable_content() {
        let t = table(3, 2);
        assert_eq!(t.text.lines().count(), 4);
        assert_eq!(&t.text[t.selection.start..t.selection.start + 1], " ");
        let c = code_block("rust", "");
        assert_eq!(c.text, "```rust\n\n```");
        assert_eq!(c.selection, 8..8);
    }

    #[test]
    fn slash_only_matches_a_word_at_caret() {
        assert_eq!(slash_query("Try /tab", 8), Some((4..8, "tab".into())));
        assert_eq!(slash_query("https://a", 9), None);
    }
}
