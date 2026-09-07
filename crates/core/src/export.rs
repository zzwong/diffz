//! Deliberate export of provider-independent data. Paths remain data, never extraction targets.
use crate::domain::*;
use serde::{Deserialize, Serialize};
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContextExport {
    pub schema_version: u32,
    pub snapshot: SnapshotId,
    pub title: String,
    pub source_path: String,
    pub source_path_bytes: Vec<u8>,
    pub origin: String,
    pub repository: Option<RepositoryKey>,
    pub reviewed_head: Option<String>,
    pub selection: SourceSelection,
    pub text: String,
    pub notes: Vec<String>,
    pub provenance: String,
}
impl ContextExport {
    pub fn selected(
        snapshot: &Snapshot,
        selection: SourceSelection,
        notes: Vec<String>,
    ) -> Result<Self, String> {
        let text = snapshot.copy_selection(&selection)?;
        if text.len() > 1024 * 1024 || notes.iter().map(String::len).sum::<usize>() > 1024 * 1024 {
            return Err("selected context is larger than 1 MiB".into());
        }
        let path = snapshot
            .file(&selection.start.file)
            .ok_or("selected source file is missing")?
            .path();
        Ok(Self {
            schema_version: 1,
            snapshot: snapshot.id.clone(),
            title: snapshot.title.clone(),
            source_path: path.display(),
            source_path_bytes: path.bytes().to_vec(),
            origin: snapshot.origin.clone(),
            repository: snapshot.remote.as_ref().map(|r| r.repository.clone()),
            reviewed_head: snapshot.remote.as_ref().map(|r| r.head.clone()),
            selection,
            text,
            notes,
            provenance:
                "User-selected source from diffz; no model or remote transmission performed".into(),
        })
    }
}
