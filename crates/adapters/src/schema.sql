PRAGMA foreign_keys = ON;
CREATE TABLE IF NOT EXISTS snapshots (
 id TEXT PRIMARY KEY NOT NULL,
 title TEXT NOT NULL,
 data TEXT NOT NULL,
 updated_at INTEGER NOT NULL DEFAULT (unixepoch())
);
CREATE TABLE IF NOT EXISTS drafts (
 id TEXT PRIMARY KEY NOT NULL,
 snapshot_id TEXT NOT NULL REFERENCES snapshots(id),
 version INTEGER NOT NULL CHECK(version >= 1),
 data TEXT NOT NULL,
 published INTEGER NOT NULL DEFAULT 0 CHECK(published IN (0,1))
);
CREATE INDEX IF NOT EXISTS drafts_snapshot ON drafts(snapshot_id);
CREATE TABLE IF NOT EXISTS views (
 snapshot_id TEXT PRIMARY KEY REFERENCES snapshots(id),
 revision INTEGER NOT NULL DEFAULT 0,
 data TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS outbox (
 id TEXT PRIMARY KEY,
 state TEXT NOT NULL CHECK(state IN ('prepared','in_flight','confirmed','rejected','unknown')),
 data TEXT NOT NULL,
 updated_at INTEGER NOT NULL DEFAULT (unixepoch())
);
CREATE TABLE IF NOT EXISTS hidden_recents (snapshot_id TEXT PRIMARY KEY REFERENCES snapshots(id));
CREATE TABLE IF NOT EXISTS settings (key TEXT PRIMARY KEY NOT NULL, data TEXT NOT NULL);
PRAGMA user_version = 1;
