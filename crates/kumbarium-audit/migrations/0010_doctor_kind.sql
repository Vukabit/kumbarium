-- 0010: the doctor kind. kum doctor --apply witnesses one
-- event carrying its repair manifest (the janitor's batch
-- pattern); preview writes nothing. Same rebuild dance; hashes
-- untouched (a new kind string is a new value in an existing
-- field, not a recipe change).

CREATE TABLE events_new (
  id TEXT PRIMARY KEY,
  at TEXT NOT NULL,
  agent_id TEXT NOT NULL,
  kind TEXT NOT NULL CHECK (
    kind IN (
      'recall', 'remember', 'supersede', 'forget', 'eval_run',
      'link', 'import', 'retire', 'unretire', 'confirm',
      'janitor', 'approve', 'reject', 'task_file', 'task_update',
      'task_done', 'task_drop', 'task_list', 'handoff_write',
      'handoff_drop', 'secret_set', 'secret_read',
      'secret_grant', 'secret_revoke', 'secret_shred',
      'secret_copy', 'secret_exec', 'secret_leakscan',
      'lease_take', 'lease_release', 'lease_break', 'get',
      'doctor'
    )
  ),
  scope TEXT NOT NULL DEFAULT '',
  detail TEXT NOT NULL DEFAULT '{}',
  session_id TEXT NOT NULL DEFAULT '',
  hash TEXT
);

INSERT INTO events_new
  (id, at, agent_id, kind, scope, detail, session_id, hash)
  SELECT id, at, agent_id, kind, scope, detail, session_id, hash
  FROM events;
DROP TABLE events;
ALTER TABLE events_new RENAME TO events;

CREATE INDEX idx_events_at ON events (at);
CREATE INDEX idx_events_agent ON events (agent_id);
