use crate::{
    domain::{Side, Snapshot},
    patch::RowKind,
    provider::Cancellation,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Severity {
    Note,
    Info,
    Warning,
    Error,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Anchor {
    File {
        path: String,
    },
    Lines {
        path: String,
        side: Side,
        start: u32,
        end: u32,
    },
}

impl Anchor {
    pub fn path(&self) -> &str {
        match self {
            Anchor::File { path } | Anchor::Lines { path, .. } => path,
        }
    }

    pub fn covers(&self, side: Side, line: u32) -> bool {
        matches!(self, Anchor::Lines { side: s, start, end, .. } if *s == side && (*start..=*end).contains(&line))
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Annotation {
    pub anchor: Anchor,
    pub severity: Severity,
    pub title: String,
    pub body: Option<String>,
    pub source: String,
}

pub trait Annotator: Send + Sync {
    fn id(&self) -> &str;
    fn annotate(
        &self,
        snapshot: &Snapshot,
        cancel: &Cancellation,
    ) -> Result<Vec<Annotation>, String>;
}

pub(crate) fn in_snapshot(snapshot: &Snapshot, anchor: &Anchor) -> bool {
    let Some(file) = snapshot
        .patch
        .files
        .iter()
        .find(|f| f.display_path() == anchor.path())
    else {
        return false;
    };
    match anchor {
        Anchor::File { .. } => true,
        Anchor::Lines {
            side, start, end, ..
        } => {
            *start >= 1
                && start <= end
                && file.line(*side, *start).is_some()
                && file.line(*side, *end).is_some()
        }
    }
}

pub struct DiffCheck;

impl DiffCheck {
    fn problem(text: &str) -> Option<(Severity, &'static str)> {
        let marker = ["<<<<<<< ", ">>>>>>> "].iter().any(|m| text.starts_with(m))
            || text == "======="
            || text == "<<<<<<<"
            || text == ">>>>>>>";
        if marker {
            return Some((Severity::Error, "Conflict marker"));
        }
        if text.ends_with([' ', '\t']) {
            return Some((Severity::Warning, "Trailing whitespace"));
        }
        let indent = &text[..text.len() - text.trim_start_matches([' ', '\t']).len()];
        if indent.contains(" \t") {
            return Some((Severity::Warning, "Space before tab in indentation"));
        }
        None
    }
}

impl Annotator for DiffCheck {
    fn id(&self) -> &str {
        "diff check"
    }

    fn annotate(
        &self,
        snapshot: &Snapshot,
        cancel: &Cancellation,
    ) -> Result<Vec<Annotation>, String> {
        let mut out: Vec<Annotation> = vec![];
        for file in &snapshot.patch.files {
            if cancel.cancelled() {
                return Err("cancelled".into());
            }
            let path = file.display_path();
            for row in file.hunks.iter().flat_map(|h| &h.rows) {
                let (RowKind::Added, Some(line)) = (row.kind, row.new_line) else {
                    continue;
                };
                let Some((severity, title)) = Self::problem(&row.text) else {
                    continue;
                };
                if let Some(Annotation {
                    anchor: Anchor::Lines { path: p, end, .. },
                    title: t,
                    ..
                }) = out.last_mut()
                    && *p == path
                    && *t == title
                    && *end + 1 == line
                {
                    *end = line;
                    continue;
                }
                out.push(Annotation {
                    anchor: Anchor::Lines {
                        path: path.clone(),
                        side: Side::Right,
                        start: line,
                        end: line,
                    },
                    severity,
                    title: title.into(),
                    body: None,
                    source: self.id().into(),
                });
            }
        }
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::patch::{ParseLimits, parse_patch};

    fn snapshot(patch: &str) -> Snapshot {
        Snapshot::new(
            "t".into(),
            parse_patch(patch.as_bytes(), ParseLimits::default()).unwrap(),
            None,
            vec![],
        )
    }

    #[test]
    fn diff_check_flags_added_lines_and_merges_runs() {
        let s = snapshot(concat!(
            "diff --git a/a.txt b/a.txt\n--- a/a.txt\n+++ b/a.txt\n@@ -1,2 +1,7 @@\n",
            " keep  \n",
            "-gone  \n",
            "+one \n",
            "+two\t\n",
            "+fine\n",
            "+<<<<<<< HEAD\n",
            "+ \tindented\n",
            "+=======\n",
        ));
        let found: Vec<_> = DiffCheck
            .annotate(&s, &Cancellation::default())
            .unwrap()
            .into_iter()
            .map(|a| match a.anchor {
                Anchor::Lines { start, end, .. } => (a.title, a.severity, start, end),
                Anchor::File { .. } => unreachable!(),
            })
            .collect();
        assert_eq!(
            found,
            [
                ("Trailing whitespace".into(), Severity::Warning, 2, 3),
                ("Conflict marker".into(), Severity::Error, 5, 5),
                (
                    "Space before tab in indentation".into(),
                    Severity::Warning,
                    6,
                    6
                ),
                ("Conflict marker".into(), Severity::Error, 7, 7),
            ]
        );
    }

    #[test]
    fn anchors_outside_the_snapshot_are_rejected() {
        let s = snapshot("diff --git a/a b/a\n--- a/a\n+++ b/a\n@@ -1 +1 @@\n-x\n+y\n");
        let lines = |path: &str, start, end| Anchor::Lines {
            path: path.into(),
            side: Side::Right,
            start,
            end,
        };
        assert!(in_snapshot(&s, &lines("a", 1, 1)));
        assert!(in_snapshot(&s, &Anchor::File { path: "a".into() }));
        assert!(!in_snapshot(&s, &lines("a", 1, 2)));
        assert!(!in_snapshot(&s, &lines("a", 0, 1)));
        assert!(!in_snapshot(&s, &lines("b", 1, 1)));
        assert!(!in_snapshot(&s, &Anchor::File { path: "b".into() }));
    }
}
