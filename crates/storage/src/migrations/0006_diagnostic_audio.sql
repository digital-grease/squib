-- Squib schema migration 6 (M5 opt-in diagnostic recordings).

-- Attachments may now be opt-in diagnostic audio. Rebuilt as in migration 5.
CREATE TABLE attachment_v6 (
    id             TEXT PRIMARY KEY,
    run_id         TEXT REFERENCES run(id),
    kind           TEXT NOT NULL CHECK (kind IN ('photo', 'video', 'diagnostic_audio')),
    relative_path  TEXT NOT NULL UNIQUE,
    sha256         TEXT NOT NULL,
    bytes          INTEGER NOT NULL CHECK (bytes >= 0),
    mime           TEXT NOT NULL,
    metadata_stripped INTEGER NOT NULL CHECK (metadata_stripped IN (0, 1)),
    created_utc_ms INTEGER NOT NULL
);
INSERT INTO attachment_v6 (id, run_id, kind, relative_path, sha256, bytes, mime, metadata_stripped, created_utc_ms)
    SELECT id, run_id, kind, relative_path, sha256, bytes, mime, metadata_stripped, created_utc_ms FROM attachment;
-- video_clip references attachment; keep its rows valid across the rebuild.
CREATE TABLE video_clip_keep AS SELECT * FROM video_clip;
DROP TABLE video_clip;
DROP TABLE attachment;
ALTER TABLE attachment_v6 RENAME TO attachment;
CREATE INDEX attachment_run ON attachment(run_id);
CREATE TABLE video_clip (
    attachment_id            TEXT PRIMARY KEY REFERENCES attachment(id),
    run_id                   TEXT NOT NULL REFERENCES run(id),
    width                    INTEGER NOT NULL CHECK (width > 0),
    height                   INTEGER NOT NULL CHECK (height > 0),
    frame_rate_milli         INTEGER NOT NULL CHECK (frame_rate_milli > 0),
    duration_ms              INTEGER NOT NULL CHECK (duration_ms >= 0),
    first_frame_camera_ns    INTEGER NOT NULL,
    camera_clock             TEXT NOT NULL CHECK (camera_clock IN ('realtime', 'unknown')),
    probe_camera_ns          INTEGER NOT NULL,
    probe_mono_ns            INTEGER NOT NULL,
    probe_boot_ns            INTEGER NOT NULL,
    start_boot_minus_mono_ns INTEGER NOT NULL,
    end_boot_minus_mono_ns   INTEGER NOT NULL,
    created_utc_ms           INTEGER NOT NULL
);
INSERT INTO video_clip SELECT * FROM video_clip_keep;
DROP TABLE video_clip_keep;
CREATE INDEX video_clip_run ON video_clip(run_id);

-- One diagnostic recording per run: mono PCM16 WAV of exactly what the detector
-- processed. File frame f is epoch frame first_epoch_frame + f. Frames the recorder
-- could not keep (or the capture lost) are silence in the file and listed in gaps_json
-- as [start, end) epoch frames, so no sample position is ever shifted.
CREATE TABLE diagnostic_recording (
    attachment_id     TEXT PRIMARY KEY REFERENCES attachment(id),
    run_id            TEXT NOT NULL UNIQUE REFERENCES run(id),
    epoch_id          TEXT,
    sample_rate_hz    INTEGER NOT NULL CHECK (sample_rate_hz > 0),
    first_epoch_frame INTEGER NOT NULL,
    frames            INTEGER NOT NULL CHECK (frames >= 0),
    gaps_json         TEXT NOT NULL,
    truncated         INTEGER NOT NULL CHECK (truncated IN (0, 1)),
    dropped_frames    INTEGER NOT NULL CHECK (dropped_frames >= 0),
    created_utc_ms    INTEGER NOT NULL
);
