-- 0001: the append-only audit event log.
--
-- Lives in its OWN database file so audit volume never contends
-- with the Library's WAL. Structured events only; the exportable
-- meeting-minutes rendering is a deterministic template over
-- `SELECT ... ORDER BY at`.

CREATE TABLE events (
  id TEXT PRIMARY KEY,
  at TEXT NOT NULL,
  agent_id TEXT NOT NULL,
  kind TEXT NOT NULL CHECK (
    kind IN (
      'recall', 'remember', 'supersede', 'forget', 'eval_run'
    )
  ),
  -- The namespace scope the request declared ('' when N/A).
  scope TEXT NOT NULL DEFAULT '',
  -- Kind-specific payload as JSON: query text, returned entry
  -- ids with scores, written entry id, eval rank diffs, ...
  detail TEXT NOT NULL DEFAULT '{}'
);

CREATE INDEX idx_events_at ON events (at);
CREATE INDEX idx_events_agent ON events (agent_id);
