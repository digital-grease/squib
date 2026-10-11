-- Squib schema migration 10 (plan phase 2.3: vibration and screen start cues).

-- Two new start methods. SQLite cannot change a CHECK constraint in place, so the run
-- table is rebuilt (the documented 12-step procedure). The runner applies this
-- migration with foreign keys off and verifies them before commit, because many tables
-- reference run(id) and must keep pointing at the same rows.
CREATE TABLE run_v10 (
    id                        TEXT PRIMARY KEY,
    session_id                TEXT NOT NULL REFERENCES session(id),
    shooter_id                TEXT NOT NULL REFERENCES shooter_profile(id),
    created_utc_ms            INTEGER NOT NULL,
    tz_offset_min             INTEGER NOT NULL,
    source_mode               TEXT NOT NULL,
    config_json               TEXT NOT NULL,
    config_hash               TEXT NOT NULL,
    calibration_profile_id    TEXT REFERENCES calibration_profile_version(id),
    app_build                 TEXT NOT NULL,
    detector_version          TEXT,
    domain_schema_version     INTEGER NOT NULL,
    outcome                   TEXT CHECK (outcome IN ('complete', 'interrupted', 'cancelled', 'failed_to_start')),
    persist_state             TEXT NOT NULL CHECK (persist_state IN ('pending', 'saved', 'failed')),
    review_state              TEXT CHECK (review_state IN ('automatic', 'needs_review', 'reviewed')),
    start_method              TEXT CHECK (start_method IN ('acoustic_cue', 'scheduled_render', 'requested_only',
                                                           'vibration_request', 'screen_frame', 'unresolved')),
    start_ref_frame           INTEGER,
    start_ref_mono_ns         INTEGER,
    armed_mono_ns             INTEGER,
    stop_mono_ns              INTEGER,
    stop_frame                INTEGER,
    interrupt_reason          TEXT,
    last_durable_seq          INTEGER NOT NULL DEFAULT 0,
    uncommitted_tail_possible INTEGER NOT NULL DEFAULT 0 CHECK (uncommitted_tail_possible IN (0, 1)),
    finalized_utc_ms          INTEGER,
    error                     TEXT,
    environment_snapshot_id   TEXT REFERENCES environment_snapshot(id),
    drill_id                  TEXT REFERENCES drill(id),
    drill_version             INTEGER,
    equipment_id              TEXT REFERENCES equipment(id),
    equipment_version         INTEGER
);
INSERT INTO run_v10 (id, session_id, shooter_id, created_utc_ms, tz_offset_min, source_mode, config_json, config_hash,
                     calibration_profile_id, app_build, detector_version, domain_schema_version, outcome, persist_state,
                     review_state, start_method, start_ref_frame, start_ref_mono_ns, armed_mono_ns, stop_mono_ns,
                     stop_frame, interrupt_reason, last_durable_seq, uncommitted_tail_possible, finalized_utc_ms, error,
                     environment_snapshot_id, drill_id, drill_version, equipment_id, equipment_version)
    SELECT id, session_id, shooter_id, created_utc_ms, tz_offset_min, source_mode, config_json, config_hash,
           calibration_profile_id, app_build, detector_version, domain_schema_version, outcome, persist_state,
           review_state, start_method, start_ref_frame, start_ref_mono_ns, armed_mono_ns, stop_mono_ns,
           stop_frame, interrupt_reason, last_durable_seq, uncommitted_tail_possible, finalized_utc_ms, error,
           environment_snapshot_id, drill_id, drill_version, equipment_id, equipment_version
    FROM run;
DROP TABLE run;
ALTER TABLE run_v10 RENAME TO run;
CREATE INDEX run_history ON run(created_utc_ms DESC);
CREATE INDEX run_unfinished ON run(outcome) WHERE outcome IS NULL;
CREATE INDEX run_session ON run(session_id);
