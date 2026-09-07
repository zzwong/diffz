//! Review state is deterministic. Run side effects outside the render callback.
use crate::{
    anchor::{NavigationReason, ViewportAnchor},
    domain::*,
};
use std::collections::BTreeMap;
#[derive(Debug, Clone)]
pub struct Session {
    pub snapshot: Snapshot,
    pub selected_file: Option<FileId>,
    pub drafts: BTreeMap<DraftId, Draft>,
    pub preferences: Preferences,
    pub anchors: BTreeMap<String, ViewportAnchor>,
    pub revision_offer: Option<String>,
    pub last_error: Option<String>,
}
#[derive(Debug, Clone)]
pub enum Event {
    SelectFile(FileId),
    Navigate(SourcePoint, NavigationReason),
    EditDraft { id: DraftId, body: String },
    DraftSaved { id: DraftId, version: u64 },
    RevisionOffered { current: SnapshotId, head: String },
    StoreFailed(String),
    SaveAnchor(ViewportAnchor),
}
#[derive(Debug, Clone)]
pub enum Effect {
    PersistDraft(Draft),
    Navigate(SourcePoint, NavigationReason),
    PersistPreferences,
}
impl Session {
    pub fn new(snapshot: Snapshot) -> Self {
        let selected_file = snapshot.patch.files.first().map(|f| f.id.clone());
        Self {
            snapshot,
            selected_file,
            drafts: BTreeMap::new(),
            preferences: Preferences::default(),
            anchors: BTreeMap::new(),
            revision_offer: None,
            last_error: None,
        }
    }
    pub fn reduce(&mut self, e: Event) -> Vec<Effect> {
        match e {
            Event::SelectFile(id) => {
                if self.snapshot.file(&id).is_some() {
                    self.selected_file = Some(id);
                }
                vec![]
            }
            Event::Navigate(p, reason) => {
                if self.snapshot.validate_point(&p).is_ok() {
                    vec![Effect::Navigate(p, reason)]
                } else {
                    vec![]
                }
            }
            Event::EditDraft { id, body } => {
                let Some(d) = self.drafts.get_mut(&id) else {
                    return vec![];
                };
                match edit_draft(d, body) {
                    Ok(true) => vec![Effect::PersistDraft(d.clone())],
                    Ok(false) => vec![],
                    Err(e) => {
                        self.last_error = Some(e);
                        vec![]
                    }
                }
            }
            Event::DraftSaved { id, version } => {
                if let Some(d) = self.drafts.get_mut(&id) {
                    acknowledge_save(d, version)
                }
                vec![]
            }
            Event::RevisionOffered { current, head } => {
                if current == self.snapshot.id
                    && self
                        .snapshot
                        .remote
                        .as_ref()
                        .is_some_and(|r| r.head != head)
                {
                    self.revision_offer = Some(head)
                }
                vec![]
            }
            Event::StoreFailed(message) => {
                self.last_error = Some(message);
                vec![]
            }
            Event::SaveAnchor(a) => {
                if a.point.snapshot == self.snapshot.id {
                    self.anchors.insert(a.point.file.0.clone(), a);
                }
                vec![Effect::PersistPreferences]
            }
        }
    }
}

/// Used by both the deterministic reducer and the native input adapter.
pub fn edit_draft(d: &mut Draft, body: String) -> Result<bool, String> {
    if d.published || d.body == body {
        return Ok(false);
    }
    let next = d.version.checked_add(1).ok_or("draft revision overflow")?;
    d.body = body;
    d.version = next;
    Ok(true)
}
pub fn acknowledge_save(d: &mut Draft, version: u64) {
    if version <= d.version {
        d.saved_version = d.saved_version.max(version);
    }
}
