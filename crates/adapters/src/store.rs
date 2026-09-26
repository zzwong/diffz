//! SQLite stores durable acknowledgements. One file lock keeps concurrent UI writers apart.
use crate::{AdapterError, Result};
use diffz_core::{
    domain::*,
    provider::{Opened, RecentSession, ReviewRules, SavedView},
    review::*,
};
use fs2::FileExt;
use rusqlite::{Connection, OptionalExtension, TransactionBehavior, params};
use std::{
    fs::{File, OpenOptions},
    path::{Path, PathBuf},
    sync::Mutex,
    time::Duration,
};

pub struct Store {
    conn: Mutex<Connection>,
    _lock: File,
    pub directory: PathBuf,
}
impl Drop for Store {
    fn drop(&mut self) {
        let _ = FileExt::unlock(&self._lock);
    }
}
fn private_file(path: &Path) -> Result<File> {
    let mut o = OpenOptions::new();
    o.read(true).write(true).create(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        o.mode(0o600).custom_flags(libc::O_NOFOLLOW);
    }
    Ok(o.open(path)?)
}
impl Store {
    pub fn open(dir: &Path) -> Result<Self> {
        if dir.exists() && dir.symlink_metadata()?.file_type().is_symlink() {
            return Err("state directory cannot be symlinked".into());
        }
        std::fs::create_dir_all(dir)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700))?;
        }
        let lock = private_file(&dir.join("writer.lock"))?;
        lock.try_lock_exclusive().map_err(|_| {
            AdapterError::Message("another diffz instance is using this state directory".into())
        })?;
        let db = dir.join("review.sqlite3");
        drop(private_file(&db)?);
        let mut c = Connection::open(&db)?;
        c.busy_timeout(Duration::from_secs(2))?;
        c.pragma_update(None, "foreign_keys", "ON")?;
        c.pragma_update(None, "journal_mode", "WAL")?;
        c.pragma_update(None, "synchronous", "FULL")?;
        let v: i64 = c.pragma_query_value(None, "user_version", |r| r.get(0))?;
        if v > 1 {
            return Err("the draft database is newer; downgrading is not supported".into());
        }
        let tx = c.transaction_with_behavior(TransactionBehavior::Immediate)?;
        tx.execute_batch(include_str!("schema.sql"))?;
        tx.commit()?;
        let s = Self {
            conn: Mutex::new(c),
            _lock: lock,
            directory: dir.to_path_buf(),
        };
        s.recover_inflight()?;
        Ok(s)
    }
    fn db(&self) -> Result<std::sync::MutexGuard<'_, Connection>> {
        self.conn
            .lock()
            .map_err(|_| "draft store lock was poisoned".into())
    }
    pub fn put_snapshot(&self, s: &Snapshot) -> Result<()> {
        if !s.verify_identity() {
            return Err("snapshot identity and its source data do not agree".into());
        }
        // Encoding a large patch must not hold up settings and other readers of the store.
        let data = serde_json::to_string(s)?;
        self.db()?.execute("INSERT INTO snapshots(id,title,data) VALUES(?1,?2,?3) ON CONFLICT(id) DO UPDATE SET title=excluded.title,data=excluded.data,updated_at=unixepoch()",params![s.id.0,s.title,data])?;
        Ok(())
    }
    pub fn snapshot(&self, id: &SnapshotId) -> Result<Snapshot> {
        let raw: String =
            self.db()?
                .query_row("SELECT data FROM snapshots WHERE id=?1", [&id.0], |r| {
                    r.get(0)
                })?;
        let s: Snapshot = serde_json::from_str(&raw)?;
        if !s.verify_identity() {
            return Err("saved snapshot integrity check failed".into());
        }
        Ok(s)
    }
    pub fn save_draft(&self, mut d: Draft) -> Result<u64> {
        if d.version == 0 || d.version > i64::MAX as u64 || d.body.len() > 1024 * 1024 {
            return Err("draft metadata fails version or size checks".into());
        }
        let s = self.snapshot(&d.snapshot)?;
        let f = s.file(&d.file).ok_or("draft file is absent")?;
        if !d.is_file_level() && !f.eligible(d.side, d.start_line, d.line) {
            return Err("the draft range does not exist in the frozen patch".into());
        }
        let mut c = self.db()?;
        let tx = c.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let old: Option<String> = tx
            .query_row("SELECT data FROM drafts WHERE id=?1", [&d.id.0], |r| {
                r.get(0)
            })
            .optional()?;
        if let Some(old) = old {
            let p: Draft = serde_json::from_str(&old)?;
            if p.snapshot != d.snapshot
                || p.file != d.file
                || p.side != d.side
                || p.start_line != d.start_line
                || p.line != d.line
                || p.published
            {
                return Err(
                    "a draft's source identity and published state cannot be rewritten".into(),
                );
            }
            if p.version > d.version {
                return Err("the draft write is stale; the newer text remains stored".into());
            }
            if p.version == d.version && p.body != d.body {
                return Err("draft text differs for the existing version".into());
            }
        }
        if d.published {
            return Err("only confirmed outbox evidence may mark a draft published".into());
        }
        d.saved_version = d.version;
        let raw = serde_json::to_string(&d)?;
        tx.execute("INSERT INTO drafts(id,snapshot_id,version,data,published) VALUES(?1,?2,?3,?4,0) ON CONFLICT(id) DO UPDATE SET version=excluded.version,data=excluded.data",params![d.id.0,d.snapshot.0,d.version as i64,raw])?;
        tx.commit()?;
        Ok(d.version)
    }
    pub fn discard_draft(&self, id: &DraftId, version: u64) -> Result<()> {
        let changed = self.db()?.execute(
            "DELETE FROM drafts WHERE id=?1 AND version=?2 AND published=0",
            params![id.0, version as i64],
        )?;
        if changed != 1 {
            return Err("the draft changed, is published, or no longer exists".into());
        }
        Ok(())
    }
    pub fn drafts(&self, id: &SnapshotId) -> Result<Vec<Draft>> {
        let c = self.db()?;
        let mut q = c.prepare("SELECT data FROM drafts WHERE snapshot_id=?1 ORDER BY id")?;
        let raws = q
            .query_map([&id.0], |r| r.get::<_, String>(0))?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        raws.into_iter()
            .map(|s| Ok(serde_json::from_str(&s)?))
            .collect()
    }
    pub fn save_view(&self, id: &SnapshotId, v: &SavedView) -> Result<()> {
        self.db()?.execute("INSERT INTO views(snapshot_id,revision,data) VALUES(?1,?2,?3) ON CONFLICT(snapshot_id) DO UPDATE SET revision=excluded.revision,data=excluded.data WHERE excluded.revision >= views.revision",params![id.0,v.revision.min(i64::MAX as u64) as i64,serde_json::to_string(v)?])?;
        Ok(())
    }
    pub fn view(&self, id: &SnapshotId) -> Result<SavedView> {
        let raw: Option<String> = self
            .db()?
            .query_row(
                "SELECT data FROM views WHERE snapshot_id=?1",
                [&id.0],
                |r| r.get(0),
            )
            .optional()?;
        Ok(match raw {
            Some(s) => serde_json::from_str(&s)?,
            None => SavedView::default(),
        })
    }
    pub fn settings(&self) -> Result<diffz_core::domain::Settings> {
        let raw: Option<String> = self
            .db()?
            .query_row("SELECT data FROM settings WHERE key='reader'", [], |r| {
                r.get(0)
            })
            .optional()?;
        Ok(match raw {
            Some(s) => serde_json::from_str(&s)?,
            None => Default::default(),
        })
    }
    pub fn save_settings(&self, s: &diffz_core::domain::Settings) -> Result<()> {
        self.db()?.execute(
            "INSERT INTO settings(key,data) VALUES('reader',?1) ON CONFLICT(key) DO UPDATE SET data=excluded.data",
            params![serde_json::to_string(s)?],
        )?;
        Ok(())
    }
    pub fn opened(&self, id: &SnapshotId) -> Result<Opened> {
        self.show_recent(id)?;
        Ok(Opened {
            snapshot: self.snapshot(id)?,
            drafts: self.drafts(id)?,
            view: self.view(id)?,
        })
    }
    pub fn show_recent(&self, id: &SnapshotId) -> Result<()> {
        let mut c = self.db()?;
        let tx = c.transaction()?;
        tx.execute("DELETE FROM hidden_recents WHERE snapshot_id=?1", [&id.0])?;
        tx.execute(
            "UPDATE snapshots SET updated_at=unixepoch() WHERE id=?1",
            [&id.0],
        )?;
        tx.commit()?;
        Ok(())
    }
    pub fn hide_recent(&self, id: Option<&SnapshotId>) -> Result<()> {
        if let Some(id) = id {
            self.db()?.execute(
                "INSERT OR IGNORE INTO hidden_recents SELECT id FROM snapshots WHERE id=?1",
                [&id.0],
            )?;
        } else {
            self.db()?.execute(
                "INSERT OR IGNORE INTO hidden_recents SELECT id FROM snapshots",
                [],
            )?;
        }
        Ok(())
    }
    pub fn recent(&self) -> Result<Vec<RecentSession>> {
        let c = self.db()?;
        let mut q =
            c.prepare("SELECT id,title FROM snapshots WHERE id NOT IN (SELECT snapshot_id FROM hidden_recents) ORDER BY updated_at DESC,rowid DESC LIMIT 30")?;
        Ok(q.query_map([], |r| {
            Ok(RecentSession {
                id: SnapshotId(r.get(0)?),
                title: r.get(1)?,
            })
        })?
        .collect::<std::result::Result<Vec<_>, _>>()?)
    }
    pub fn insert_prepared(
        &self,
        p: &PreparedReview,
        rules: &dyn ReviewRules,
    ) -> Result<OutboxEntry> {
        if !p.verify(rules) {
            return Err("prepared review fingerprint mismatch".into());
        }
        let e = OutboxEntry {
            prepared: p.clone(),
            state: OutboxState::Prepared,
            baseline_review_ids: vec![],
            remote_id: None,
            diagnostic: None,
        };
        self.db()?.execute(
            "INSERT INTO outbox(id,state,data) VALUES(?1,'prepared',?2)",
            params![p.id.0, serde_json::to_string(&e)?],
        )?;
        Ok(e)
    }
    pub fn outbox(&self) -> Result<Vec<OutboxEntry>> {
        let c = self.db()?;
        let mut q = c.prepare("SELECT data FROM outbox ORDER BY updated_at DESC")?;
        let raws = q
            .query_map([], |r| r.get::<_, String>(0))?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        raws.into_iter()
            .map(|s| Ok(serde_json::from_str(&s)?))
            .collect()
    }
    pub fn operation(&self, id: &OperationId) -> Result<OutboxEntry> {
        let raw: String =
            self.db()?
                .query_row("SELECT data FROM outbox WHERE id=?1", [&id.0], |r| r.get(0))?;
        Ok(serde_json::from_str(&raw)?)
    }
    pub fn transition(&self, e: &OutboxEntry, rules: &dyn ReviewRules) -> Result<()> {
        self.write_transition(e, Some(rules))
    }
    /// Skips re-verification; only for moves that leave the stored review byte-identical.
    fn write_transition(&self, e: &OutboxEntry, rules: Option<&dyn ReviewRules>) -> Result<()> {
        let mut c = self.db()?;
        let tx = c.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let raw: String = tx.query_row(
            "SELECT data FROM outbox WHERE id=?1",
            [&e.prepared.id.0],
            |r| r.get(0),
        )?;
        let old: OutboxEntry = serde_json::from_str(&raw)?;
        if old.prepared.fingerprint != e.prepared.fingerprint
            || rules.is_some_and(|r| !e.prepared.verify(r))
        {
            return Err("outbox payload changed after preview".into());
        }
        if !old.state.can_transition(e.state) {
            return Err(format!("bad outbox move {:?} → {:?}", old.state, e.state).into());
        }
        tx.execute(
            "UPDATE outbox SET state=?1,data=?2,updated_at=unixepoch() WHERE id=?3",
            params![e.state.as_str(), serde_json::to_string(e)?, e.prepared.id.0],
        )?;
        if e.state == OutboxState::Confirmed {
            if e.remote_id.is_none() {
                return Err("the confirmed review lacks its remote evidence ID".into());
            }
            for comment in &e.prepared.comments {
                let data: Option<String> = tx
                    .query_row(
                        "SELECT data FROM drafts WHERE id=?1",
                        [&comment.draft.0],
                        |r| r.get(0),
                    )
                    .optional()?;
                if let Some(data) = data {
                    let mut d: Draft = serde_json::from_str(&data)?;
                    if d.version == comment.version && d.body == comment.body {
                        d.published = true;
                        tx.execute(
                            "UPDATE drafts SET published=1,data=?1 WHERE id=?2",
                            params![serde_json::to_string(&d)?, d.id.0],
                        )?;
                    }
                }
            }
        }
        tx.commit()?;
        Ok(())
    }
    fn recover_inflight(&self) -> Result<()> {
        for mut e in self.outbox()? {
            if e.state == OutboxState::InFlight {
                e.state = OutboxState::UnknownOutcome;
                e.diagnostic=Some("the process stopped after sending became possible; reconcile before submitting again".into());
                self.write_transition(&e, None)?;
            }
        }
        Ok(())
    }
}
