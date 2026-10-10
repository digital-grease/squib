-- Squib schema migration 8 (versioned equipment profiles, docs/squib/06).

-- Optional local equipment labels. Editing appends a version; runs keep the version
-- they used. No serial numbers are asked for.
CREATE TABLE equipment (
    id             TEXT PRIMARY KEY,
    created_utc_ms INTEGER NOT NULL,
    archived       INTEGER NOT NULL DEFAULT 0 CHECK (archived IN (0, 1))
);
CREATE TABLE equipment_version (
    equipment_id   TEXT NOT NULL REFERENCES equipment(id),
    version        INTEGER NOT NULL CHECK (version >= 1),
    name           TEXT NOT NULL CHECK (length(name) BETWEEN 1 AND 80),
    category       TEXT NOT NULL CHECK (category IN ('pistol', 'rifle', 'pcc', 'shotgun', 'rimfire', 'other')),
    notes          TEXT NOT NULL DEFAULT '' CHECK (length(notes) <= 1000),
    created_utc_ms INTEGER NOT NULL,
    PRIMARY KEY (equipment_id, version)
);
CREATE TRIGGER equipment_version_no_update BEFORE UPDATE ON equipment_version
BEGIN SELECT RAISE(ABORT, 'equipment versions are immutable'); END;
ALTER TABLE run ADD COLUMN equipment_id TEXT REFERENCES equipment(id);
ALTER TABLE run ADD COLUMN equipment_version INTEGER;
