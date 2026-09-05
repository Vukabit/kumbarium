-- 0002: the 'dropped' status. A dead project's standing
-- briefing would otherwise be served to every future session,
-- forever; dropping keeps the row (the chain is the diary) and
-- removes it from circulation. Same rebuild dance as the audit
-- shelf: SQLite cannot widen a CHECK in place.

CREATE TABLE handoffs_new (
  id TEXT PRIMARY KEY,
  namespace TEXT NOT NULL,
  content TEXT NOT NULL,
  agent_id TEXT NOT NULL,
  source TEXT NOT NULL DEFAULT '',
  superseded_by TEXT REFERENCES handoffs_new (id),
  note TEXT,
  status TEXT NOT NULL DEFAULT 'live' CHECK (
    status IN ('live', 'pending', 'rejected', 'dropped')
  ),
  created_at TEXT NOT NULL,
  updated_at TEXT NOT NULL
);

INSERT INTO handoffs_new
  (id, namespace, content, agent_id, source, superseded_by,
   note, status, created_at, updated_at)
  SELECT id, namespace, content, agent_id, source,
         superseded_by, note, status, created_at, updated_at
  FROM handoffs;
DROP TABLE handoffs;
ALTER TABLE handoffs_new RENAME TO handoffs;

CREATE INDEX idx_handoffs_namespace ON handoffs (namespace);
CREATE INDEX idx_handoffs_superseded ON handoffs (superseded_by);
