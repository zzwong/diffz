//! A small directory tree where filters keep ancestors and file identity unchanged.
use crate::domain::FileId;
use std::collections::{BTreeMap, HashMap, HashSet};
#[derive(Debug, Clone)]
pub struct TreeRow {
    pub path: String,
    pub label: String,
    /// Dimmed folder path shown before `label` when a single-file folder is folded into its file row.
    pub prefix: String,
    pub depth: usize,
    pub file: Option<FileId>,
    pub expanded: bool,
    /// Full path a renamed or copied file came from.
    pub old_path: Option<String>,
}
#[derive(Default, Debug)]
struct Directory {
    dirs: BTreeMap<String, Directory>,
    files: BTreeMap<String, (FileId, String)>,
}
#[derive(Default, Debug)]
pub struct FileTree {
    entries: Vec<(FileId, String)>,
    old_paths: HashMap<FileId, String>,
}
impl FileTree {
    pub fn new(entries: Vec<(FileId, String)>) -> Self {
        Self {
            entries,
            old_paths: HashMap::new(),
        }
    }
    /// Where renamed or copied files came from; a filter also matches these paths.
    pub fn with_old_paths(mut self, old_paths: HashMap<FileId, String>) -> Self {
        self.old_paths = old_paths;
        self
    }
    pub fn rows(&self, collapsed: &HashSet<String>, query: &str) -> Vec<TreeRow> {
        let query = query.to_lowercase();
        let mut root = Directory::default();
        for (id, path) in &self.entries {
            let old = self.old_paths.get(id);
            if !path.to_lowercase().contains(&query)
                && !old.is_some_and(|o| o.to_lowercase().contains(&query))
            {
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
        flatten(
            &root,
            "",
            0,
            collapsed,
            !query.is_empty(),
            &self.old_paths,
            &mut rows,
        );
        rows
    }
}
fn flatten(
    dir: &Directory,
    parent: &str,
    depth: usize,
    collapsed: &HashSet<String>,
    filtering: bool,
    old: &HashMap<FileId, String>,
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
        if node.dirs.is_empty() && node.files.len() == 1 {
            let (name, (id, file_path)) = node.files.first_key_value().unwrap();
            rows.push(TreeRow {
                path: file_path.clone(),
                label: name.clone(),
                prefix: format!("{label}/"),
                depth,
                file: Some(id.clone()),
                expanded: false,
                old_path: old.get(id).cloned(),
            });
            continue;
        }
        let expanded = filtering || !collapsed.contains(&path);
        rows.push(TreeRow {
            path: path.clone(),
            label,
            prefix: String::new(),
            depth,
            file: None,
            expanded,
            old_path: None,
        });
        if expanded {
            flatten(node, &path, depth + 1, collapsed, filtering, old, rows);
        }
    }
    for (name, (id, path)) in &dir.files {
        rows.push(TreeRow {
            path: path.clone(),
            label: name.clone(),
            prefix: String::new(),
            depth,
            file: Some(id.clone()),
            expanded: false,
            old_path: old.get(id).cloned(),
        });
    }
}
