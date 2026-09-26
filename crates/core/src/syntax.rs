//! Optional decoration with fixed bounds. It does not affect source identity, geometry, or completeness.
//! A file without a grammar remains unhighlighted.
use crate::registry::LanguageProvider;
use std::{ops::Range, sync::atomic::AtomicUsize};
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Token {
    Keyword,
    Function,
    Type,
    String,
    Number,
    Comment,
    Property,
    Constant,
    Operator,
    Punctuation,
    Variable,
    Parameter,
    Attribute,
    Namespace,
    Label,
    Embedded,
}
#[derive(Debug, Clone)]
pub struct Span {
    pub bytes: Range<usize>,
    pub token: Token,
}
/// The grammars compiled into diffz. Without the `syntax` feature it claims nothing.
pub struct BuiltinGrammars;

impl LanguageProvider for BuiltinGrammars {
    fn claim(&self, path: &str) -> Option<(&str, u8)> {
        #[cfg(feature = "syntax")]
        {
            enabled::language_for_path(path).map(|l| (l.name, 0))
        }
        #[cfg(not(feature = "syntax"))]
        {
            let _ = path;
            None
        }
    }
    fn highlight(&self, path: &str, source: &str, cancel: &AtomicUsize) -> Option<Vec<Vec<Span>>> {
        #[cfg(feature = "syntax")]
        {
            enabled::run(path, source, cancel)
        }
        #[cfg(not(feature = "syntax"))]
        {
            let _ = (path, source, cancel);
            None
        }
    }
}
#[cfg(feature = "syntax")]
mod enabled {
    use super::{Span, Token};
    use std::sync::{OnceLock, atomic::AtomicUsize};
    use tree_sitter_highlight::{HighlightConfiguration, HighlightEvent, Highlighter};

    const RECOGNIZED: &[(&str, Token)] = &[
        ("keyword", Token::Keyword),
        ("function", Token::Function),
        ("function.method", Token::Function),
        ("function.macro", Token::Function),
        ("method", Token::Function),
        ("constructor", Token::Function),
        ("type", Token::Type),
        ("type.builtin", Token::Type),
        ("string", Token::String),
        ("string.special", Token::String),
        ("character", Token::String),
        ("escape", Token::String),
        ("number", Token::Number),
        ("float", Token::Number),
        ("comment", Token::Comment),
        ("property", Token::Property),
        ("field", Token::Property),
        ("constant", Token::Constant),
        ("constant.builtin", Token::Constant),
        ("boolean", Token::Constant),
        ("operator", Token::Operator),
        ("punctuation", Token::Punctuation),
        ("punctuation.bracket", Token::Punctuation),
        ("punctuation.delimiter", Token::Punctuation),
        ("punctuation.special", Token::Punctuation),
        ("variable", Token::Variable),
        ("variable.builtin", Token::Variable),
        ("variable.parameter", Token::Parameter),
        ("parameter", Token::Parameter),
        ("attribute", Token::Attribute),
        ("tag", Token::Attribute),
        ("tag.delimiter", Token::Attribute),
        ("namespace", Token::Namespace),
        ("module", Token::Namespace),
        ("label", Token::Label),
        ("embedded", Token::Embedded),
    ];

    fn recognized_names() -> &'static [&'static str] {
        static NAMES: OnceLock<Vec<&'static str>> = OnceLock::new();
        NAMES.get_or_init(|| RECOGNIZED.iter().map(|(name, _)| *name).collect())
    }

    fn token_for(index: usize) -> Option<Token> {
        RECOGNIZED.get(index).map(|(_, token)| *token)
    }

    fn build_config(
        language: impl Into<tree_sitter::Language>,
        name: &str,
        highlights: &str,
        injections: &str,
        locals: &str,
    ) -> HighlightConfiguration {
        let mut config =
            HighlightConfiguration::new(language.into(), name, highlights, injections, locals)
                .expect("the highlight query for this grammar must parse");
        config.configure(recognized_names());
        config
    }

    pub struct Lang {
        pub name: &'static str,
        build: fn() -> HighlightConfiguration,
        config: OnceLock<HighlightConfiguration>,
    }

    impl Lang {
        fn config(&self) -> &HighlightConfiguration {
            self.config.get_or_init(self.build)
        }
    }

    macro_rules! grammar {
        ($static:ident, $build:ident, $name:literal, $lang:expr, $hl:expr, $inj:expr, $loc:expr) => {
            static $static: Lang = Lang {
                name: $name,
                build: $build,
                config: OnceLock::new(),
            };
            fn $build() -> HighlightConfiguration {
                build_config($lang, $name, $hl, $inj, $loc)
            }
        };
    }

    grammar!(
        RUST,
        build_rust,
        "rust",
        tree_sitter_rust::LANGUAGE,
        tree_sitter_rust::HIGHLIGHTS_QUERY,
        tree_sitter_rust::INJECTIONS_QUERY,
        ""
    );
    grammar!(
        JAVASCRIPT,
        build_javascript,
        "javascript",
        tree_sitter_javascript::LANGUAGE,
        tree_sitter_javascript::HIGHLIGHT_QUERY,
        tree_sitter_javascript::INJECTIONS_QUERY,
        tree_sitter_javascript::LOCALS_QUERY
    );
    grammar!(
        TYPESCRIPT,
        build_typescript,
        "typescript",
        tree_sitter_typescript::LANGUAGE_TYPESCRIPT,
        tree_sitter_typescript::HIGHLIGHTS_QUERY,
        "",
        tree_sitter_typescript::LOCALS_QUERY
    );
    static TSX: Lang = Lang {
        name: "tsx",
        build: build_tsx,
        config: OnceLock::new(),
    };
    fn build_tsx() -> HighlightConfiguration {
        let highlights = [
            tree_sitter_javascript::HIGHLIGHT_QUERY,
            tree_sitter_javascript::JSX_HIGHLIGHT_QUERY,
            tree_sitter_typescript::HIGHLIGHTS_QUERY,
        ]
        .join("\n");
        build_config(
            tree_sitter_typescript::LANGUAGE_TSX,
            "tsx",
            &highlights,
            "",
            tree_sitter_typescript::LOCALS_QUERY,
        )
    }
    grammar!(
        JSON,
        build_json,
        "json",
        tree_sitter_json::LANGUAGE,
        tree_sitter_json::HIGHLIGHTS_QUERY,
        "",
        ""
    );
    grammar!(
        PYTHON,
        build_python,
        "python",
        tree_sitter_python::LANGUAGE,
        tree_sitter_python::HIGHLIGHTS_QUERY,
        "",
        ""
    );
    grammar!(
        GO,
        build_go,
        "go",
        tree_sitter_go::LANGUAGE,
        tree_sitter_go::HIGHLIGHTS_QUERY,
        "",
        ""
    );
    grammar!(
        C,
        build_c,
        "c",
        tree_sitter_c::LANGUAGE,
        tree_sitter_c::HIGHLIGHT_QUERY,
        "",
        ""
    );
    grammar!(
        CPP,
        build_cpp,
        "cpp",
        tree_sitter_cpp::LANGUAGE,
        tree_sitter_cpp::HIGHLIGHT_QUERY,
        "",
        ""
    );
    grammar!(
        JAVA,
        build_java,
        "java",
        tree_sitter_java::LANGUAGE,
        tree_sitter_java::HIGHLIGHTS_QUERY,
        "",
        ""
    );
    grammar!(
        RUBY,
        build_ruby,
        "ruby",
        tree_sitter_ruby::LANGUAGE,
        tree_sitter_ruby::HIGHLIGHTS_QUERY,
        "",
        tree_sitter_ruby::LOCALS_QUERY
    );
    grammar!(
        BASH,
        build_bash,
        "bash",
        tree_sitter_bash::LANGUAGE,
        tree_sitter_bash::HIGHLIGHT_QUERY,
        "",
        ""
    );
    grammar!(
        TOML,
        build_toml,
        "toml",
        tree_sitter_toml_ng::LANGUAGE,
        tree_sitter_toml_ng::HIGHLIGHTS_QUERY,
        "",
        ""
    );
    grammar!(
        YAML,
        build_yaml,
        "yaml",
        tree_sitter_yaml::LANGUAGE,
        tree_sitter_yaml::HIGHLIGHTS_QUERY,
        "",
        ""
    );
    grammar!(
        HTML,
        build_html,
        "html",
        tree_sitter_html::LANGUAGE,
        tree_sitter_html::HIGHLIGHTS_QUERY,
        tree_sitter_html::INJECTIONS_QUERY,
        ""
    );
    grammar!(
        CSS,
        build_css,
        "css",
        tree_sitter_css::LANGUAGE,
        tree_sitter_css::HIGHLIGHTS_QUERY,
        "",
        ""
    );
    grammar!(
        ELIXIR,
        build_elixir,
        "elixir",
        tree_sitter_elixir::LANGUAGE,
        tree_sitter_elixir::HIGHLIGHTS_QUERY,
        tree_sitter_elixir::INJECTIONS_QUERY,
        ""
    );
    grammar!(
        HEEX,
        build_heex,
        "heex",
        tree_sitter_heex::LANGUAGE,
        tree_sitter_heex::HIGHLIGHTS_QUERY,
        tree_sitter_heex::INJECTIONS_QUERY,
        ""
    );
    grammar!(
        GLEAM,
        build_gleam,
        "gleam",
        tree_sitter_gleam::LANGUAGE,
        tree_sitter_gleam::HIGHLIGHTS_QUERY,
        tree_sitter_gleam::INJECTIONS_QUERY,
        tree_sitter_gleam::LOCALS_QUERY
    );

    pub fn language_for_path(path: &str) -> Option<&'static Lang> {
        let ext = path.rsplit('/').next()?.rsplit_once('.')?.1;
        match ext.to_ascii_lowercase().as_str() {
            "rs" => Some(&RUST),
            "js" | "mjs" | "cjs" | "jsx" => Some(&JAVASCRIPT),
            "ts" | "mts" | "cts" => Some(&TYPESCRIPT),
            "tsx" => Some(&TSX),
            "json" => Some(&JSON),
            "py" => Some(&PYTHON),
            "go" => Some(&GO),
            "c" | "h" => Some(&C),
            "cpp" | "cc" | "cxx" | "hpp" | "hh" | "hxx" => Some(&CPP),
            "java" => Some(&JAVA),
            "rb" => Some(&RUBY),
            "sh" | "bash" | "zsh" => Some(&BASH),
            "toml" => Some(&TOML),
            "yaml" | "yml" => Some(&YAML),
            "html" | "htm" => Some(&HTML),
            "css" => Some(&CSS),
            "ex" | "exs" => Some(&ELIXIR),
            "heex" => Some(&HEEX),
            "gleam" => Some(&GLEAM),
            _ => None,
        }
    }

    pub fn run(path: &str, source: &str, cancel: &AtomicUsize) -> Option<Vec<Vec<Span>>> {
        let config = language_for_path(path)?.config();
        let mut highlighter = Highlighter::new();
        let events = highlighter
            .highlight(config, source.as_bytes(), Some(cancel), |_| None)
            .ok()?;
        let bytes = source.as_bytes();
        let newlines: Vec<usize> = bytes
            .iter()
            .enumerate()
            .filter(|(_, b)| **b == b'\n')
            .map(|(i, _)| i)
            .collect();
        let mut bounds = Vec::with_capacity(newlines.len() + 1);
        let mut line_start = 0;
        for &nl in &newlines {
            let mut line_end = nl;
            if line_end > line_start && bytes[line_end - 1] == b'\r' {
                line_end -= 1;
            }
            bounds.push((line_start, line_end));
            line_start = nl + 1;
        }
        bounds.push((line_start, bytes.len()));
        let mut lines = vec![Vec::new(); bounds.len()];
        let mut stack: Vec<Token> = Vec::new();
        for event in events {
            match event {
                Err(_) => return None,
                Ok(HighlightEvent::HighlightStart(highlight)) => {
                    stack.push(token_for(highlight.0)?);
                }
                Ok(HighlightEvent::HighlightEnd) => {
                    stack.pop();
                }
                Ok(HighlightEvent::Source { start, end }) => {
                    if let Some(&token) = stack.last() {
                        push_span(&mut lines, &bounds, start, end, token);
                    }
                }
            }
        }
        Some(lines)
    }

    fn push_span(
        lines: &mut [Vec<Span>],
        bounds: &[(usize, usize)],
        start: usize,
        end: usize,
        token: Token,
    ) {
        let mut cursor = start;
        let mut line = bounds.partition_point(|&(s, _)| s <= cursor) - 1;
        while cursor < end {
            let (line_start, line_end) = bounds[line];
            let clipped = end.min(line_end);
            if clipped > cursor {
                lines[line].push(Span {
                    bytes: cursor - line_start..clipped - line_start,
                    token,
                });
            }
            if clipped == end {
                break;
            }
            cursor = bounds[line + 1].0;
            line += 1;
        }
    }
}
#[cfg(all(test, feature = "syntax"))]
mod tests {
    use super::*;
    use crate::registry::Registry;
    fn highlight(path: &str, source: &str, cancel: &AtomicUsize) -> Vec<Vec<Span>> {
        Registry::builtin().highlight(path, source, cancel)
    }
    fn language_name(path: &str) -> Option<&'static str> {
        let grammars: &'static BuiltinGrammars = &BuiltinGrammars;
        grammars.claim(path).map(|(name, _)| name)
    }
    fn has(lines: &[Vec<Span>], line: usize, bytes: Range<usize>, token: Token) -> bool {
        lines[line]
            .iter()
            .any(|s| s.bytes == bytes && s.token == token)
    }
    #[test]
    fn rust_tokens_land_on_the_right_lines() {
        let lines = highlight(
            "src/a.rs",
            "fn main() {\n    let x = 1; // hi\n}",
            &AtomicUsize::new(0),
        );
        assert_eq!(lines.len(), 3);
        assert!(has(&lines, 0, 0..2, Token::Keyword));
        assert!(has(&lines, 0, 3..7, Token::Function));
        assert!(has(&lines, 1, 4..7, Token::Keyword));
        assert!(has(&lines, 1, 12..13, Token::Constant));
        assert!(has(&lines, 1, 15..20, Token::Comment));
    }
    #[test]
    fn tsx_captures_javascript_jsx_and_typescript_tokens() {
        let source = "import { useState } from \"react\"; // import/string/comment\n\n".to_owned()
            + "type Props = { title: string };\n"
            + "function render(props: Props) {\n"
            + "    const count: number = 1;\n"
            + "    return <button aria-label={props.title}>{props.title + count}</button>;\n"
            + "}";
        let lines = highlight("component.test.tsx", &source, &AtomicUsize::new(0));

        assert!(has_text(&lines, &source, "import", Token::Keyword));
        assert!(has_text(&lines, &source, "\"react\"", Token::String));
        assert!(has_text(
            &lines,
            &source,
            "// import/string/comment",
            Token::Comment
        ));
        assert!(has_text(&lines, &source, "Props", Token::Type));
        assert!(has_text(&lines, &source, "render", Token::Function));
        assert!(has_text(&lines, &source, "button", Token::Attribute));
        assert!(has_text(&lines, &source, "aria-label", Token::Attribute));
        assert!(has_text(&lines, &source, "props", Token::Parameter));
        assert!(has_text(&lines, &source, "title", Token::Property));
        assert!(has_text(&lines, &source, "+", Token::Operator));
        assert!(has_text(&lines, &source, "(", Token::Punctuation));
    }

    fn has_text(lines: &[Vec<Span>], source: &str, text: &str, token: Token) -> bool {
        source.lines().enumerate().any(|(line, source_line)| {
            lines[line].iter().any(|span| {
                span.token == token && source_line.get(span.bytes.clone()) == Some(text)
            })
        })
    }
    #[test]
    fn every_grammar_compiles_and_highlights_something() {
        let cancel = AtomicUsize::new(0);
        let samples: &[(&str, &str)] = &[
            ("a.rs", "fn f() {}"),
            ("a.js", "function f() { return 1 }"),
            ("a.ts", "function f(): number { return 1 }"),
            ("a.tsx", "const A = () => <div>{1}</div>"),
            ("a.py", "def f():\n    return 1"),
            ("a.go", "func f() int { return 1 }"),
            ("a.c", "int f(void) { return 1; }"),
            ("a.cpp", "class A { int f(); };"),
            ("a.java", "class A { int f() { return 1; } }"),
            ("a.rb", "def f\n  1\nend"),
            ("a.sh", "if true; then echo hi; fi"),
            ("a.json", "{\"a\": 1}"),
            ("a.toml", "[a]\nb = 1"),
            ("a.yaml", "a: 1\nb: two"),
            ("a.html", "<div class=\"x\">hi</div>"),
            ("a.css", "div { color: red }"),
            ("a.ex", "defmodule A do\n  def f, do: 1\nend"),
            ("a.heex", "<div :if={@x}>hi</div>"),
            ("a.gleam", "pub fn add(a: Int) -> Int { a }"),
        ];
        for (path, source) in samples {
            assert!(
                language_name(path).is_some(),
                "{path} has no registered grammar"
            );
            let lines = highlight(path, source, &cancel);
            assert_eq!(lines.len(), source.lines().count().max(1), "{path}");
            assert!(
                lines.iter().any(|l| !l.is_empty()),
                "{path} produced no spans"
            );
        }
    }
    #[test]
    fn unknown_and_dotless_paths_stay_plain() {
        for path in ["a.txt", "Makefile", "dir.with.dots/README", "a.RS.bak"] {
            assert_eq!(language_name(path), None, "{path}");
            let lines = highlight(path, "fn main() {}\nx", &AtomicUsize::new(0));
            assert_eq!(lines.len(), 2);
            assert!(lines.iter().all(Vec::is_empty), "{path}");
        }
    }
    #[test]
    fn extension_match_is_case_insensitive_and_uses_the_basename() {
        assert_eq!(language_name("A/B.PY"), Some("python"));
        assert_eq!(language_name("x.d/notes"), None);
        assert_eq!(language_name("Q2Benchmarking.tsx"), Some("tsx"));
    }
    #[test]
    fn crlf_lines_exclude_the_carriage_return() {
        let lines = highlight("a.rs", "// a\r\n// b\r\n", &AtomicUsize::new(0));
        assert!(has(&lines, 0, 0..4, Token::Comment));
        assert!(has(&lines, 1, 0..4, Token::Comment));
    }
    #[test]
    fn cancellation_returns_plain_lines() {
        let lines = highlight("a.rs", "fn main() {}\n", &AtomicUsize::new(1));
        assert!(lines.iter().all(Vec::is_empty));
    }
}
