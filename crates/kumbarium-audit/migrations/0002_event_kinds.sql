-- 0002: widen the event-kind enum with 'link' and 'import'.
--
-- SQLite cannot alter a CHECK constraint in place; the table is
-- rebuilt and rows copied. Append-only discipline: 0001 stays
-- untouched, existing rows carry over verbatim.

CREATE TABLE events_new (
  id TEXT PRIMARY KEY,
  at TEXT NOT NULL,
  agent_id TEXT NOT NULL,
  kind TEXT NOT NULL CHECK (
    kind IN (
      'recall', 'remember', 'supersede', 'forget', 'eval_run',
      'link', 'import'
    )
  ),
  scope TEXT NOT NULL DEFAULT '',
  detail TEXT NOT NULL DEFAULT '{}'
);

INSERT INTO events_new SELECT * FROM events;
DROP TABLE events;
ALTER TABLE events_new RENAME TO events;

CREATE INDEX idx_events_at ON events (at);
CREATE INDEX idx_events_agent ON events (agent_id);
