-- Squib schema migration 2 (M2 range conditions).

-- Resolved conditions pinned to runs. Immutable: a refresh creates a new snapshot.
-- `retention` records whether precise provenance was kept or redacted at write time.
CREATE TABLE environment_snapshot (
    id             TEXT PRIMARY KEY,
    created_utc_ms INTEGER NOT NULL,
    policy_version TEXT NOT NULL,
    retention      TEXT NOT NULL CHECK (retention IN ('redacted', 'precise')),
    content_json   TEXT NOT NULL
);
CREATE TRIGGER environment_snapshot_no_update BEFORE UPDATE ON environment_snapshot
BEGIN SELECT RAISE(ABORT, 'environment snapshots are immutable'); END;
CREATE TRIGGER environment_snapshot_no_delete BEFORE DELETE ON environment_snapshot
BEGIN SELECT RAISE(ABORT, 'environment snapshots are immutable'); END;

ALTER TABLE run ADD COLUMN environment_snapshot_id TEXT REFERENCES environment_snapshot(id);

-- Provider documents for offline display. Separable from the journal: clearing it
-- never touches runs or snapshots.
CREATE TABLE provider_cache (
    provider       TEXT NOT NULL,
    cache_key      TEXT NOT NULL,
    fetched_utc_ms INTEGER NOT NULL,
    expires_utc_ms INTEGER NOT NULL,
    etag           TEXT,
    body           BLOB NOT NULL,
    PRIMARY KEY (provider, cache_key)
);

-- Active manual overrides, one per field, kept until cleared or expired.
CREATE TABLE env_override (
    field          TEXT PRIMARY KEY,
    candidate_json TEXT NOT NULL,
    set_utc_ms     INTEGER NOT NULL,
    expires_utc_ms INTEGER
);

CREATE TABLE saved_place (
    id             TEXT PRIMARY KEY,
    label          TEXT NOT NULL,
    lat            REAL NOT NULL CHECK (lat BETWEEN -90 AND 90),
    lon            REAL NOT NULL CHECK (lon BETWEEN -180 AND 180),
    created_utc_ms INTEGER NOT NULL
);

CREATE TABLE app_setting (
    key   TEXT PRIMARY KEY,
    value TEXT NOT NULL
);
