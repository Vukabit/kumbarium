-- 0006: widen the event-kind enum with the reading room's
-- verbs (D-043): lease_take, lease_release, lease_break.
-- Expiry is deliberately absent: a lapsed lease is an absence
-- computed at read time, never a fired event. Same rebuild
-- dance as ever.

CREATE TABLE events_new (
  id TEXT PRIMARY KEY,
  at TEXT NOT NULL,
  agent_id TEXT NOT NULL,
  kind TEXT NOT NULL CHECK (
    kind IN (
      'recall', 'remember', 'supersede', 'forget', 'eval_run',
      'link', 'import', 'retire', 'unretire', 'confirm',
      'janitor', 'approve', 'reject', 'task_file', 'task_update',
      'task_done', 'task_drop', 'handoff_write', 'secret_set',
      'secret_read', 'secret_grant', 'secret_revoke',
      'secret_shred', 'secret_copy', 'secret_exec',
      'secret_leakscan', 'lease_take', 'lease_release',
      'lease_break'
    )
  ),
  scope TEXT NOT NULL DEFAULT '',
  detail TEXT NOT NULL DEFAULT '{}',
  hash TEXT
);

-- Explicit columns, not SELECT *: these rebuilds must stay
-- correct if ever re-applied after a later migration widened
-- the table (the legacy-collapse path does exactly that).
INSERT INTO events_new (id, at, agent_id, kind, scope, detail, hash)
  SELECT id, at, agent_id, kind, scope, detail, hash FROM events;
DROP TABLE events;
ALTER TABLE events_new RENAME TO events;

CREATE INDEX idx_events_at ON events (at);
CREATE INDEX idx_events_agent ON events (agent_id);
