use diffz_core::{
    domain::FileId,
    file_tree::{FileTree, TreeRow},
};
use gpui_kit::{FocusHandle, ListAlignment, ListState, px};
use std::{collections::HashSet, sync::Arc};

pub(crate) struct FileBrowserState {
    pub rows: Arc<Vec<diffz_core::file_tree::TreeRow>>,
    pub collapsed: HashSet<String>,
    pub cursor: usize,
    pub focus: FocusHandle,
    pub list: ListState,
    pub visible_files: Vec<FileId>,
}

impl FileBrowserState {
    pub(crate) fn new(focus: FocusHandle) -> Self {
        Self {
            rows: Arc::new(vec![]),
            collapsed: Default::default(),
            cursor: 0,
            focus,
            list: ListState::new(0, ListAlignment::Top, px(200.)),
            visible_files: vec![],
        }
    }

    pub(crate) fn rebuild(&mut self, entries: Vec<(FileId, String)>, query: &str) {
        let (visible_files, rows) = project(entries, &self.collapsed, query);
        self.visible_files = visible_files;
        self.rows = Arc::new(rows);
        self.cursor = self.cursor.min(self.rows.len().saturating_sub(1));
        self.list = ListState::new(self.rows.len(), ListAlignment::Top, px(180.));
    }
}

fn project(
    entries: Vec<(FileId, String)>,
    collapsed: &HashSet<String>,
    query: &str,
) -> (Vec<FileId>, Vec<TreeRow>) {
    let tree = FileTree::new(entries);
    let visible_files = tree
        .rows(&Default::default(), query)
        .into_iter()
        .filter_map(|row| row.file)
        .collect();
    (visible_files, tree.rows(collapsed, query))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn collapsed_tree_keeps_matching_files_in_navigation() {
        let entries = vec![
            (FileId("a".into()), "src/a.rs".into()),
            (FileId("b".into()), "src/b.rs".into()),
        ];
        let collapsed = HashSet::from(["src".to_string()]);
        let (visible, rows) = project(entries.clone(), &collapsed, "");
        assert_eq!(visible, vec![FileId("a".into()), FileId("b".into())]);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].path, "src");

        let (visible, rows) = project(entries, &collapsed, "a.rs");
        assert_eq!(visible, vec![FileId("a".into())]);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].prefix, "src/");
    }
}
