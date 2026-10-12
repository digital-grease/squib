-- Squib schema migration 11 (plan phase 4.2: manual target-photo annotations).

-- Each save is a new numbered version for the photo; earlier versions are kept. A version
-- records the run's review revision and score revision at the time, so later review or
-- score changes are visible next to the marks. Deleted only with the run (guarded path).
CREATE TABLE photo_annotation (
    id              TEXT PRIMARY KEY,
    attachment_id   TEXT NOT NULL REFERENCES attachment(id),
    run_id          TEXT NOT NULL REFERENCES run(id),
    number          INTEGER NOT NULL CHECK (number >= 1),
    run_revision    INTEGER,
    score_revision  INTEGER,
    -- {"marks":[{"x","y","kind","label"?}],"note"}: positions are fractions of the photo.
    content_json    TEXT NOT NULL,
    created_utc_ms  INTEGER NOT NULL,
    UNIQUE (attachment_id, number)
);
CREATE INDEX photo_annotation_run ON photo_annotation(run_id);

CREATE TRIGGER photo_annotation_no_update BEFORE UPDATE ON photo_annotation
BEGIN SELECT RAISE(ABORT, 'photo annotations are immutable'); END;
CREATE TRIGGER photo_annotation_no_delete BEFORE DELETE ON photo_annotation
WHEN NOT EXISTS (SELECT 1 FROM deletion_guard)
BEGIN SELECT RAISE(ABORT, 'photo annotations are immutable'); END;
