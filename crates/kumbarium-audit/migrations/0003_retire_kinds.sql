-- 0003: widen the event-kind enum with 'retire'/'unretire'.
-- Same rebuild dance as 0002: SQLite cannot alter a CHECK.

CREATE TABLE events_new (
  id TEXT PRIMARY KEY,
  at TEXT NOT NULL,
  agent_id TEXT NOT NULL,
  kind TEXT NOT NULL CHECK (
    kind IN (
      'recall', 'remember', 'supersede', 'forget', 'eval_run',
      'link', 'import', 'retire', 'unretire'
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
