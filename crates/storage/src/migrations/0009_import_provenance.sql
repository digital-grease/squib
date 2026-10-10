-- Squib schema migration 9 (import as copy, docs/squib/06 "Import contract").

-- When an archive is imported as a copy, every journal record gets a new id. The
-- original ids are kept here as provenance, with when and from which archive.
CREATE TABLE import_provenance (
    entity          TEXT NOT NULL,
    new_id          TEXT NOT NULL,
    original_id     TEXT NOT NULL,
    archive_created_utc_ms INTEGER NOT NULL,
    imported_utc_ms INTEGER NOT NULL,
    PRIMARY KEY (entity, new_id)
);
CREATE INDEX import_provenance_original ON import_provenance(original_id);
