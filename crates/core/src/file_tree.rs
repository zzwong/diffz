//! A small directory tree where filters keep ancestors and file identity unchanged.
use crate::domain::FileId;
use std::collections::{BTreeMap, HashSet};
#[derive(Debug, Clone)]
pub struct TreeRow {
    pub path: String,
    pub label: String,
    pub depth: usize,
    pub file: Option<FileId>,
    pub expanded: bool,
}
#[derive(Default, Debug)]
struct Directory {
    dirs: BTreeMap<String, Directory>,
    files: BTreeMap<String, (FileId, String)>,
}
#[derive(Default, Debug)]
pub struct FileTree {
    entries: Vec<(FileId, String)>,
}
impl FileTree {
    pub fn new(entries: Vec<(FileId, String)>) -> Self {
        Self { entries }
    }
    pub fn rows(&self, collapsed: &HashSet<String>, query: &str) -> Vec<TreeRow> {
        let query = query.to_lowercase();
        let mut root = Directory::default();
        for (id, path) in &self.entries {
            if !path.to_lowercase().contains(&query) {
                continue;
            }
            let mut parts = path.split('/').peekable();
            let mut dir = &mut root;
            while let Some(part) = parts.next() {
                if parts.peek().is_none() {
                    dir.files.insert(part.into(), (id.clone(), path.clone()));
                    break;
                }
                dir = dir.dirs.entry(part.into()).or_default();
            }
        }
        let mut rows = vec![];
        flatten(&root, "", 0, collapsed, !query.is_empty(), &mut rows);
        rows
    }
}
fn flatten(
    dir: &Directory,
    parent: &str,
    depth: usize,
    collapsed: &HashSet<String>,
    filtering: bool,
    rows: &mut Vec<TreeRow>,
) {
    for (name, child) in &dir.dirs {
        let mut label = name.clone();
        let mut path = if parent.is_empty() {
            name.clone()
        } else {
            format!("{parent}/{name}")
        };
        let mut node = child;
        while node.files.is_empty() && node.dirs.len() == 1 {
            let (name, child) = node.dirs.first_key_value().unwrap();
            label.push('/');
            label.push_str(name);
            path.push('/');
            path.push_str(name);
            node = child;
        }
        let expanded = filtering || !collapsed.contains(&path);
        rows.push(TreeRow {
            path: path.clone(),
            label,
            depth,
            file: None,
            expanded,
        });
        if expanded {
            flatten(node, &path, depth + 1, collapsed, filtering, rows);
        }
    }
    for (name, (id, path)) in &dir.files {
        rows.push(TreeRow {
            path: path.clone(),
            label: name.clone(),
            depth,
            file: Some(id.clone()),
            expanded: false,
        });
    }
}
