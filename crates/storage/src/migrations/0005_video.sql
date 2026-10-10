-- Squib schema migration 5 (M4 video proof of concept).

-- Attachments may now be video clips. SQLite cannot alter a CHECK constraint, so the
-- table is rebuilt with identical columns and rows.
CREATE TABLE attachment_v5 (
    id             TEXT PRIMARY KEY,
    run_id         TEXT REFERENCES run(id),
    kind           TEXT NOT NULL CHECK (kind IN ('photo', 'video')),
    relative_path  TEXT NOT NULL UNIQUE,
    sha256         TEXT NOT NULL,
    bytes          INTEGER NOT NULL CHECK (bytes >= 0),
    mime           TEXT NOT NULL,
    metadata_stripped INTEGER NOT NULL CHECK (metadata_stripped IN (0, 1)),
    created_utc_ms INTEGER NOT NULL
);
INSERT INTO attachment_v5 (id, run_id, kind, relative_path, sha256, bytes, mime, metadata_stripped, created_utc_ms)
    SELECT id, run_id, kind, relative_path, sha256, bytes, mime, metadata_stripped, created_utc_ms FROM attachment;
DROP TABLE attachment;
ALTER TABLE attachment_v5 RENAME TO attachment;
CREATE INDEX attachment_run ON attachment(run_id);

-- A video clip recorded with a run. Only raw clock observations are stored; the
-- mapping from file time to the run timeline is derived when read, so a better
-- mapping method can be applied later without rewriting records.
CREATE TABLE video_clip (
    attachment_id            TEXT PRIMARY KEY REFERENCES attachment(id),
    run_id                   TEXT NOT NULL REFERENCES run(id),
    width                    INTEGER NOT NULL CHECK (width > 0),
    height                   INTEGER NOT NULL CHECK (height > 0),
    frame_rate_milli         INTEGER NOT NULL CHECK (frame_rate_milli > 0),
    duration_ms              INTEGER NOT NULL CHECK (duration_ms >= 0),
    -- Camera timestamp of the first frame in the file (file time zero).
    first_frame_camera_ns    INTEGER NOT NULL,
    -- Camera2 SENSOR_INFO_TIMESTAMP_SOURCE: 'realtime' (boot time) or 'unknown'.
    camera_clock             TEXT NOT NULL CHECK (camera_clock IN ('realtime', 'unknown')),
    -- One capture-started callback: the frame's camera timestamp and both host clocks read in the callback.
    probe_camera_ns          INTEGER NOT NULL,
    probe_mono_ns            INTEGER NOT NULL,
    probe_boot_ns            INTEGER NOT NULL,
    -- Boot time minus monotonic time, sampled when recording started and stopped.
    start_boot_minus_mono_ns INTEGER NOT NULL,
    end_boot_minus_mono_ns   INTEGER NOT NULL,
    created_utc_ms           INTEGER NOT NULL
);
CREATE INDEX video_clip_run ON video_clip(run_id);
