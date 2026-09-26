wit_bindgen::generate!({ path: "../../../../../wit", world: "extension" });

use diffz::extension::types::{Anchor, Annotation, LineRange, Review, RowKind, Severity, Side};
use exports::diffz::extension::annotator::Guest;

struct Todo;

impl Guest for Todo {
    fn annotate(review: Review) -> Result<Vec<Annotation>, String> {
        // Triggers for the host's sandbox tests.
        match review.title.as_str() {
            "diffz-test-spin" => loop {
                std::hint::black_box(0);
            },
            "diffz-test-grow" => {
                let mut hog = Vec::new();
                loop {
                    hog.push(vec![1u8; 1 << 20]);
                    std::hint::black_box(&hog);
                }
            }
            "diffz-test-fail" => return Err("asked to fail".into()),
            _ => {}
        }
        let mut out = Vec::new();
        for file in review.files {
            for row in file.hunks.iter().flat_map(|h| &h.rows) {
                if let (RowKind::Added, Some(line)) = (row.kind, row.new_line)
                    && row.text.contains("TODO")
                {
                    out.push(Annotation {
                        anchor: Anchor::Lines(LineRange {
                            path: file.path.clone(),
                            side: Side::New,
                            start: line,
                            end: line,
                        }),
                        severity: Severity::Info,
                        title: "TODO".into(),
                        body: Some(row.text.trim().into()),
                    });
                }
            }
        }
        Ok(out)
    }
}

export!(Todo);
