use diffz_core::{
    extension::{self, CONTRACT},
    registry::Registry,
};
use std::{
    fs,
    path::{Path, PathBuf},
};

fn fixtures() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/extensions")
}

fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("diffz-ext-{name}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    dir
}

fn copy_toy(to: &Path, manifest: impl FnOnce(String) -> String) {
    let from = fixtures().join("toy");
    for sub in ["grammars", "queries/toy", "themes/toy-dark"] {
        fs::create_dir_all(to.join(sub)).unwrap();
        for f in fs::read_dir(from.join(sub)).unwrap().flatten() {
            if f.path().is_file() {
                fs::copy(f.path(), to.join(sub).join(f.file_name())).unwrap();
            }
        }
    }
    let text = fs::read_to_string(from.join("extension.toml")).unwrap();
    fs::write(to.join("extension.toml"), manifest(text)).unwrap();
}

#[test]
fn the_fixture_loads_with_its_language_and_theme() {
    let installed = extension::discover(&[fixtures()]);
    assert!(installed.problems.is_empty(), "{:?}", installed.problems);
    let [ext] = installed.extensions.as_slice() else {
        panic!("{:?}", installed.extensions)
    };
    assert_eq!((ext.id.as_str(), ext.name.as_str()), ("toy", "Toy"));
    assert_eq!(ext.languages[0].extensions, ["toy"]);

    let mut registry = Registry::default();
    registry.add_extensions(installed);
    assert_eq!(registry.language_name("dir/A.TOY"), Some("toy"));
    let themes = registry.themes();
    assert_eq!(themes.len(), 1);
    assert_eq!(themes[0].reference, "toy-dark");
    assert!(themes[0].path.ends_with("themes/toy-dark/theme.toml"));
}

#[test]
fn broken_extensions_are_reported_and_skipped() {
    let root = scratch("broken");
    type Case = (&'static str, fn(String) -> String, &'static str);
    let cases: [Case; 5] = [
        (
            "newer",
            |m| m.replace("diffz = \"0.1\"", "diffz = \"0.9\""),
            "needs extension contract 0.9",
        ),
        (
            "escape",
            |m| m.replace("grammars/toy.wasm", "../toy.wasm"),
            "leaves the extension",
        ),
        (
            "bad-id",
            |m| m.replace("id = \"toy\"", "id = \"a b\""),
            "invalid id",
        ),
        ("garbled", |_| "id = ".into(), "extension.toml"),
        ("twin", |m| m, "duplicate id toy"),
    ];
    copy_toy(&root.join("a-original"), |m| m);
    for (name, manifest, _) in &cases {
        copy_toy(&root.join(name), manifest);
    }
    let installed = extension::discover(std::slice::from_ref(&root));
    assert_eq!(installed.extensions.len(), 1);
    assert_eq!(
        installed.problems.len(),
        cases.len(),
        "{:?}",
        installed.problems
    );
    for (name, _, expected) in cases {
        assert!(
            installed
                .problems
                .iter()
                .any(|p| p.contains(&format!("/{name}:")) && p.contains(expected)),
            "{name}: {:?}",
            installed.problems
        );
    }
    assert_eq!(CONTRACT, (0, 1));
    let _ = fs::remove_dir_all(&root);
}

#[cfg(feature = "wasm")]
mod wasm {
    use super::*;
    use diffz_core::syntax::Token;
    use std::sync::{Arc, atomic::AtomicUsize};

    fn text<'a>(source: &'a str, line: &[diffz_core::syntax::Span], token: Token) -> Vec<&'a str> {
        line.iter()
            .filter(|s| s.token == token)
            .map(|s| &source[s.bytes.clone()])
            .collect()
    }

    #[test]
    fn a_wasm_grammar_highlights_and_compiles_cleanly() {
        let mut registry = Registry::default();
        registry.add_extensions(extension::discover(&[fixtures()]));
        assert_eq!(registry.check_extensions(), Vec::<String>::new());
        let source = "let x \"hi\" 42 # note";
        let lines = registry.highlight("a.toy", source, &AtomicUsize::new(0));
        assert_eq!(text(source, &lines[0], Token::Keyword), ["let"]);
        assert_eq!(text(source, &lines[0], Token::String), ["\"hi\""]);
        assert_eq!(text(source, &lines[0], Token::Number), ["42"]);
        assert_eq!(text(source, &lines[0], Token::Comment), ["# note"]);
    }

    #[test]
    fn extension_grammars_outrank_builtins_and_share_across_threads() {
        let root = scratch("outrank");
        copy_toy(&root.join("rust-as-toy"), |m| {
            m.replace("extensions = [\"toy\"]", "extensions = [\"rs\"]")
        });
        let mut registry = Registry::builtin();
        registry.add_extensions(extension::discover(std::slice::from_ref(&root)));
        assert_eq!(registry.language_name("main.rs"), Some("toy"));
        let registry = Arc::new(registry);
        let threads: Vec<_> = (0..4)
            .map(|i| {
                let registry = registry.clone();
                std::thread::spawn(move || {
                    let source = format!("fn f{i}");
                    let lines = registry.highlight("a.rs", &source, &AtomicUsize::new(0));
                    text(&source, &lines[0], Token::Keyword).len()
                })
            })
            .collect();
        for t in threads {
            assert_eq!(t.join().unwrap(), 1);
        }
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn a_broken_grammar_is_reported_by_the_check() {
        let root = scratch("corrupt");
        copy_toy(&root.join("corrupt"), |m| m);
        fs::write(root.join("corrupt/grammars/toy.wasm"), b"not wasm").unwrap();
        let mut registry = Registry::default();
        registry.add_extensions(extension::discover(std::slice::from_ref(&root)));
        let problems = registry.check_extensions();
        assert_eq!(problems.len(), 1);
        assert!(problems[0].starts_with("toy: toy: "), "{problems:?}");
        let lines = registry.highlight("a.toy", "let x", &AtomicUsize::new(0));
        assert!(lines.iter().all(Vec::is_empty));
        let _ = fs::remove_dir_all(&root);
    }
}
