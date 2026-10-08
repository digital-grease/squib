-- Squib schema migration 3 (M3 training, journal ownership).

-- Drills and their immutable versions. Editing a drill appends a version (A14).
CREATE TABLE drill (
    id             TEXT PRIMARY KEY,
    created_utc_ms INTEGER NOT NULL,
    archived       INTEGER NOT NULL DEFAULT 0 CHECK (archived IN (0, 1))
);
CREATE TABLE drill_version (
    drill_id       TEXT NOT NULL REFERENCES drill(id),
    version        INTEGER NOT NULL CHECK (version >= 1),
    recipe_json    TEXT NOT NULL,
    content_hash   TEXT NOT NULL,
    created_utc_ms INTEGER NOT NULL,
    PRIMARY KEY (drill_id, version)
);
ALTER TABLE run ADD COLUMN drill_id TEXT REFERENCES drill(id);
ALTER TABLE run ADD COLUMN drill_version INTEGER;

-- Score entries are revisions independent of detector observations.
CREATE TABLE score_revision (
    run_id         TEXT NOT NULL REFERENCES run(id),
    number         INTEGER NOT NULL CHECK (number >= 1),
    parent_hash    TEXT,
    content_hash   TEXT NOT NULL,
    entry_json     TEXT NOT NULL,
    editor         TEXT NOT NULL,
    created_utc_ms INTEGER NOT NULL,
    PRIMARY KEY (run_id, number)
);

-- A string entered from another timer, with its precision and source label (A20).
CREATE TABLE manual_string (
    run_id       TEXT PRIMARY KEY REFERENCES run(id),
    source_label TEXT NOT NULL,
    precision_ns INTEGER NOT NULL CHECK (precision_ns > 0),
    times_json   TEXT NOT NULL
);

-- User-confirmed round counts; the proposal is kept beside the confirmation (A24).
CREATE TABLE round_count (
    run_id            TEXT PRIMARY KEY REFERENCES run(id),
    proposed          INTEGER NOT NULL CHECK (proposed >= 0),
    confirmed         INTEGER CHECK (confirmed >= 0),
    confirmed_utc_ms  INTEGER
);

-- App-owned attachment files (paths relative to the app attachment directory).
CREATE TABLE attachment (
    id             TEXT PRIMARY KEY,
    run_id         TEXT REFERENCES run(id),
    kind           TEXT NOT NULL CHECK (kind IN ('photo')),
    relative_path  TEXT NOT NULL UNIQUE,
    sha256         TEXT NOT NULL,
    bytes          INTEGER NOT NULL CHECK (bytes >= 0),
    mime           TEXT NOT NULL,
    metadata_stripped INTEGER NOT NULL CHECK (metadata_stripped IN (0, 1)),
    created_utc_ms INTEGER NOT NULL
);
CREATE INDEX attachment_run ON attachment(run_id);

ALTER TABLE shooter_profile ADD COLUMN archived INTEGER NOT NULL DEFAULT 0;

-- Deliberate deletion (docs/squib/08): original records stay append-only for every
-- other path. The repository inserts a guard row inside its deletion transaction;
-- deletes are refused whenever no guard row exists. Updates remain refused always.
CREATE TABLE deletion_guard (token TEXT PRIMARY KEY);

DROP TRIGGER detected_candidate_no_delete;
CREATE TRIGGER detected_candidate_no_delete BEFORE DELETE ON detected_candidate
WHEN NOT EXISTS (SELECT 1 FROM deletion_guard)
BEGIN SELECT RAISE(ABORT, 'detected candidates are immutable'); END;
DROP TRIGGER quality_event_no_delete;
CREATE TRIGGER quality_event_no_delete BEFORE DELETE ON quality_event
WHEN NOT EXISTS (SELECT 1 FROM deletion_guard)
BEGIN SELECT RAISE(ABORT, 'quality events are immutable'); END;
DROP TRIGGER run_revision_no_delete;
CREATE TRIGGER run_revision_no_delete BEFORE DELETE ON run_revision
WHEN NOT EXISTS (SELECT 1 FROM deletion_guard)
BEGIN SELECT RAISE(ABORT, 'revisions are immutable'); END;
DROP TRIGGER environment_snapshot_no_delete;
CREATE TRIGGER environment_snapshot_no_delete BEFORE DELETE ON environment_snapshot
WHEN NOT EXISTS (SELECT 1 FROM deletion_guard)
BEGIN SELECT RAISE(ABORT, 'environment snapshots are immutable'); END;

CREATE TRIGGER drill_version_no_update BEFORE UPDATE ON drill_version
BEGIN SELECT RAISE(ABORT, 'drill versions are immutable'); END;
CREATE TRIGGER drill_version_no_delete BEFORE DELETE ON drill_version
WHEN NOT EXISTS (SELECT 1 FROM deletion_guard)
BEGIN SELECT RAISE(ABORT, 'drill versions are immutable'); END;
CREATE TRIGGER score_revision_no_update BEFORE UPDATE ON score_revision
BEGIN SELECT RAISE(ABORT, 'score revisions are immutable'); END;
CREATE TRIGGER score_revision_no_delete BEFORE DELETE ON score_revision
WHEN NOT EXISTS (SELECT 1 FROM deletion_guard)
BEGIN SELECT RAISE(ABORT, 'score revisions are immutable'); END;
CREATE TRIGGER manual_string_no_update BEFORE UPDATE ON manual_string
BEGIN SELECT RAISE(ABORT, 'manual strings are immutable'); END;
