-- Squib schema migration 12 (plan phase 6.3 and 6.4: ballistics loads and saved range cards).

-- A load is a cartridge and rifle as the ballistics solver needs it. Editing appends a
-- version; saved range cards keep the version they used. Values are SI.
CREATE TABLE load (
    id             TEXT PRIMARY KEY,
    created_utc_ms INTEGER NOT NULL,
    archived       INTEGER NOT NULL DEFAULT 0 CHECK (archived IN (0, 1))
);
CREATE TABLE load_version (
    load_id             TEXT NOT NULL REFERENCES load(id),
    version             INTEGER NOT NULL CHECK (version >= 1),
    name                TEXT NOT NULL CHECK (length(name) BETWEEN 1 AND 80),
    -- The rifle it is for, if the shooter keeps equipment profiles.
    equipment_id        TEXT REFERENCES equipment(id),
    drag_model          TEXT NOT NULL CHECK (drag_model IN ('G1', 'G7')),
    bc_lb_in2           REAL NOT NULL CHECK (bc_lb_in2 BETWEEN 0.02 AND 2.0),
    bullet_mass_kg      REAL NOT NULL CHECK (bullet_mass_kg BETWEEN 0.0001 AND 0.2),
    muzzle_velocity_mps REAL NOT NULL CHECK (muzzle_velocity_mps BETWEEN 60 AND 1600),
    sight_height_m      REAL NOT NULL CHECK (sight_height_m BETWEEN 0 AND 0.3),
    zero_range_m        REAL NOT NULL CHECK (zero_range_m BETWEEN 5 AND 1000),
    -- Kept for later optional terms (spin drift); not used by the solver yet. Twist is
    -- the length of one turn, negative for a left-hand twist.
    bullet_diameter_m   REAL CHECK (bullet_diameter_m IS NULL OR bullet_diameter_m BETWEEN 0.002 AND 0.03),
    bullet_length_m     REAL CHECK (bullet_length_m IS NULL OR bullet_length_m BETWEEN 0.005 AND 0.08),
    twist_m             REAL CHECK (twist_m IS NULL OR abs(twist_m) BETWEEN 0.1 AND 1.0),
    notes               TEXT NOT NULL DEFAULT '' CHECK (length(notes) <= 1000),
    created_utc_ms      INTEGER NOT NULL,
    PRIMARY KEY (load_id, version)
);
CREATE TRIGGER load_version_no_update BEFORE UPDATE ON load_version
BEGIN SELECT RAISE(ABORT, 'load versions are immutable'); END;

-- A saved range card: the solver's full input and output, the method that produced it,
-- and where its conditions came from. Immutable; deleted only through the guarded path.
CREATE TABLE ballistic_card (
    id                      TEXT PRIMARY KEY,
    load_id                 TEXT NOT NULL,
    load_version            INTEGER NOT NULL,
    environment_snapshot_id TEXT REFERENCES environment_snapshot(id),
    session_id              TEXT REFERENCES session(id),
    title                   TEXT NOT NULL DEFAULT '' CHECK (length(title) <= 80),
    inputs_json             TEXT NOT NULL,
    result_json             TEXT NOT NULL,
    method                  TEXT NOT NULL CHECK (length(method) BETWEEN 1 AND 400),
    created_utc_ms          INTEGER NOT NULL,
    FOREIGN KEY (load_id, load_version) REFERENCES load_version(load_id, version)
);
CREATE INDEX ballistic_card_load ON ballistic_card(load_id);
CREATE TRIGGER ballistic_card_no_update BEFORE UPDATE ON ballistic_card
BEGIN SELECT RAISE(ABORT, 'range cards are immutable'); END;
CREATE TRIGGER ballistic_card_no_delete BEFORE DELETE ON ballistic_card
WHEN NOT EXISTS (SELECT 1 FROM deletion_guard)
BEGIN SELECT RAISE(ABORT, 'range cards are immutable'); END;
