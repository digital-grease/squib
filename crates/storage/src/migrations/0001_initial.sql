-- Squib schema migration 1 (M1 journal core).
-- Timing values are integer nanoseconds or frame indices. Unknown is NULL, never 0.

CREATE TABLE app_meta (
    key   TEXT PRIMARY KEY,
    value TEXT NOT NULL
);

CREATE TABLE shooter_profile (
    id             TEXT PRIMARY KEY,
    name           TEXT NOT NULL,
    notes          TEXT,
    created_utc_ms INTEGER NOT NULL
);

CREATE TABLE session (
    id             TEXT PRIMARY KEY,
    shooter_id     TEXT NOT NULL REFERENCES shooter_profile(id),
    started_utc_ms INTEGER NOT NULL,
    tz_offset_min  INTEGER NOT NULL,
    notes          TEXT,
    active         INTEGER NOT NULL CHECK (active IN (0, 1))
);

CREATE TABLE calibration_profile_version (
    id                   TEXT PRIMARY KEY,
    route_signature      TEXT NOT NULL,
    label                TEXT,
    algorithm_version    TEXT NOT NULL,
    detector_config_json TEXT NOT NULL,
    suggestion_json      TEXT NOT NULL,
    verdict              TEXT NOT NULL,
    impulse_count        INTEGER NOT NULL,
    os_build             TEXT,
    created_utc_ms       INTEGER NOT NULL
);
CREATE INDEX calibration_route ON calibration_profile_version(route_signature, created_utc_ms);

CREATE TABLE run (
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
    start_method              TEXT CHECK (start_method IN ('acoustic_cue', 'scheduled_render', 'requested_only', 'unresolved')),
    start_ref_frame           INTEGER,
    start_ref_mono_ns         INTEGER,
    armed_mono_ns             INTEGER,
    stop_mono_ns              INTEGER,
    stop_frame                INTEGER,
    interrupt_reason          TEXT,
    last_durable_seq          INTEGER NOT NULL DEFAULT 0,
    uncommitted_tail_possible INTEGER NOT NULL DEFAULT 0 CHECK (uncommitted_tail_possible IN (0, 1)),
    finalized_utc_ms          INTEGER,
    error                     TEXT
);
CREATE INDEX run_history ON run(created_utc_ms DESC);
CREATE INDEX run_unfinished ON run(outcome) WHERE outcome IS NULL;

CREATE TABLE capture_epoch (
    id               TEXT PRIMARY KEY,
    run_id           TEXT NOT NULL REFERENCES run(id),
    sample_rate_hz   INTEGER NOT NULL CHECK (sample_rate_hz > 0),
    channels         INTEGER NOT NULL CHECK (channels > 0),
    sample_format    TEXT NOT NULL,
    route_json       TEXT NOT NULL,
    route_signature  TEXT NOT NULL,
    clock_domain     TEXT NOT NULL,
    started_mono_ns  INTEGER,
    summary_json     TEXT
);
CREATE INDEX capture_epoch_run ON capture_epoch(run_id);

-- Cue observations evolve during a run (requested -> rendered -> heard), so they are
-- upserted; they are run state, not detector observations.
CREATE TABLE cue_observation (
    run_id                 TEXT NOT NULL REFERENCES run(id),
    cue_id                 INTEGER NOT NULL,
    kind                   TEXT NOT NULL CHECK (kind IN ('start', 'par')),
    par_index              INTEGER,
    template_version       TEXT NOT NULL,
    requested_mono_ns      INTEGER NOT NULL,
    issued_mono_ns         INTEGER NOT NULL,
    render_mono_ns         INTEGER,
    epoch_id               TEXT REFERENCES capture_epoch(id),
    acoustic_onset_frame   INTEGER,
    acoustic_onset_mono_ns INTEGER,
    match_ncc              REAL,
    span_start_frame       INTEGER,
    span_end_frame         INTEGER,
    span_exact             INTEGER CHECK (span_exact IN (0, 1)),
    missed                 INTEGER NOT NULL CHECK (missed IN (0, 1)),
    heard                  INTEGER CHECK (heard IN (0, 1)),
    PRIMARY KEY (run_id, cue_id)
);

CREATE TABLE detected_candidate (
    run_id               TEXT NOT NULL REFERENCES run(id),
    epoch_id             TEXT NOT NULL REFERENCES capture_epoch(id),
    sequence             INTEGER NOT NULL,
    onset_frame          INTEGER NOT NULL,
    peak_frame           INTEGER NOT NULL,
    features_json        TEXT NOT NULL,
    suggestion           TEXT NOT NULL,
    reasons_json         TEXT NOT NULL,
    detector_score       REAL NOT NULL CHECK (detector_score >= 0 AND detector_score <= 1),
    -- Detector scores are ranking scores; this column documents that explicitly.
    score_is_probability INTEGER NOT NULL DEFAULT 0 CHECK (score_is_probability = 0),
    algorithm_version    TEXT NOT NULL,
    config_hash          TEXT NOT NULL,
    batch_seq            INTEGER NOT NULL,
    PRIMARY KEY (run_id, epoch_id, sequence)
);
CREATE INDEX detected_candidate_onset ON detected_candidate(run_id, onset_frame);

CREATE TABLE quality_event (
    run_id      TEXT NOT NULL REFERENCES run(id),
    seq         INTEGER NOT NULL,
    epoch_id    TEXT REFERENCES capture_epoch(id),
    kind        TEXT NOT NULL,
    severity    TEXT NOT NULL CHECK (severity IN ('info', 'warning', 'integrity')),
    start_frame INTEGER,
    end_frame   INTEGER,
    at_mono_ns  INTEGER,
    detail      TEXT NOT NULL,
    batch_seq   INTEGER NOT NULL,
    PRIMARY KEY (run_id, seq)
);

-- Coarse non-playable energy (2 bytes per 10 ms). Never PCM.
CREATE TABLE energy_envelope (
    run_id      TEXT NOT NULL REFERENCES run(id),
    epoch_id    TEXT NOT NULL REFERENCES capture_epoch(id),
    first_frame INTEGER NOT NULL,
    hop_frames  INTEGER NOT NULL CHECK (hop_frames > 0),
    version     TEXT NOT NULL,
    data        BLOB NOT NULL,
    PRIMARY KEY (run_id, epoch_id, first_frame)
);

CREATE TABLE run_revision (
    run_id         TEXT NOT NULL REFERENCES run(id),
    number         INTEGER NOT NULL CHECK (number >= 1),
    parent_number  INTEGER,
    parent_hash    TEXT,
    content_hash   TEXT NOT NULL,
    editor         TEXT NOT NULL,
    created_utc_ms INTEGER NOT NULL,
    content_json   TEXT NOT NULL,
    PRIMARY KEY (run_id, number),
    FOREIGN KEY (run_id, parent_number) REFERENCES run_revision(run_id, number),
    CHECK ((number = 1 AND parent_number IS NULL) OR (number > 1 AND parent_number = number - 1))
);

-- Original observations and review history are append-only.
CREATE TRIGGER detected_candidate_no_update BEFORE UPDATE ON detected_candidate
BEGIN SELECT RAISE(ABORT, 'detected candidates are immutable'); END;
CREATE TRIGGER detected_candidate_no_delete BEFORE DELETE ON detected_candidate
BEGIN SELECT RAISE(ABORT, 'detected candidates are immutable'); END;
CREATE TRIGGER quality_event_no_update BEFORE UPDATE ON quality_event
BEGIN SELECT RAISE(ABORT, 'quality events are immutable'); END;
CREATE TRIGGER quality_event_no_delete BEFORE DELETE ON quality_event
BEGIN SELECT RAISE(ABORT, 'quality events are immutable'); END;
CREATE TRIGGER run_revision_no_update BEFORE UPDATE ON run_revision
BEGIN SELECT RAISE(ABORT, 'revisions are immutable'); END;
CREATE TRIGGER run_revision_no_delete BEFORE DELETE ON run_revision
BEGIN SELECT RAISE(ABORT, 'revisions are immutable'); END;
