-- Squib schema migration 4 (M4 day plans, checklists, coach rotation).

-- A practice or match day: agenda, checklist, notes. Private data (backups only).
CREATE TABLE day_plan (
    id             TEXT PRIMARY KEY,
    title          TEXT NOT NULL,
    date_local     TEXT NOT NULL CHECK (length(date_local) = 10),
    kind           TEXT NOT NULL CHECK (kind IN ('practice', 'match')),
    notes          TEXT NOT NULL DEFAULT '',
    created_utc_ms INTEGER NOT NULL,
    archived       INTEGER NOT NULL DEFAULT 0 CHECK (archived IN (0, 1))
);

-- Agenda items: a drill to run (session plan), a match stage, or a timed event.
CREATE TABLE plan_item (
    id             TEXT PRIMARY KEY,
    plan_id        TEXT NOT NULL REFERENCES day_plan(id),
    ordinal        INTEGER NOT NULL,
    kind           TEXT NOT NULL CHECK (kind IN ('drill', 'stage', 'event')),
    title          TEXT NOT NULL,
    time_local     TEXT CHECK (time_local IS NULL OR length(time_local) = 5),
    drill_id       TEXT REFERENCES drill(id),
    drill_version  INTEGER,
    target_strings INTEGER CHECK (target_strings IS NULL OR target_strings BETWEEN 1 AND 50),
    notes          TEXT NOT NULL DEFAULT '',
    skipped        INTEGER NOT NULL DEFAULT 0 CHECK (skipped IN (0, 1))
);
CREATE INDEX plan_item_plan ON plan_item(plan_id, ordinal);

-- Runs made from an agenda item. Completion is derived from run outcomes, so a
-- skipped item never has a fabricated result.
CREATE TABLE plan_run (
    item_id TEXT NOT NULL REFERENCES plan_item(id),
    run_id  TEXT NOT NULL REFERENCES run(id),
    PRIMARY KEY (item_id, run_id)
);

-- Checklist items: `plan_id` NULL is the reusable template.
CREATE TABLE checklist_item (
    id      TEXT PRIMARY KEY,
    plan_id TEXT REFERENCES day_plan(id),
    ordinal INTEGER NOT NULL,
    text    TEXT NOT NULL,
    checked INTEGER NOT NULL DEFAULT 0 CHECK (checked IN (0, 1))
);
CREATE INDEX checklist_plan ON checklist_item(plan_id, ordinal);
