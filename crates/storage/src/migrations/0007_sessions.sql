-- Squib schema migration 7 (session lifecycle, notes, and cost snapshot).

-- A session is one range visit. It ends by an explicit action or after a long idle gap.
ALTER TABLE session ADD COLUMN ended_utc_ms INTEGER;
-- Per-round cost saved with the session (JSON {per_round_minor, currency}), so a later
-- price change does not rewrite what earlier visits cost.
ALTER TABLE session ADD COLUMN cost_json TEXT;
CREATE INDEX session_started ON session(started_utc_ms);
CREATE INDEX run_session ON run(session_id);
