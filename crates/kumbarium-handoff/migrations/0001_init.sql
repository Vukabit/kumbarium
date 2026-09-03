-- 0001: handoffs. A handoff is the note a departing session
-- leaves for the next one: what is mid-flight on this shelf.
-- Exactly one LIVE head per namespace (writing supersedes it;
-- the chain is the scope's session diary, D-020). The desk's
-- status column is reused verbatim (D-027): a pending briefing
-- is NEVER served. Namespace is the validated PATH, gate-checked
-- against memory.db's registry (D-033).

CREATE TABLE handoffs (
  id TEXT PRIMARY KEY,
  namespace TEXT NOT NULL,
  content TEXT NOT NULL,
  agent_id TEXT NOT NULL,
  source TEXT NOT NULL DEFAULT '',
  superseded_by TEXT REFERENCES handoffs (id),
  note TEXT,
  status TEXT NOT NULL DEFAULT 'live' CHECK (
    status IN ('live', 'pending', 'rejected')
  ),
  created_at TEXT NOT NULL,
  updated_at TEXT NOT NULL
);

CREATE INDEX idx_handoffs_namespace ON handoffs (namespace);
CREATE INDEX idx_handoffs_superseded ON handoffs (superseded_by);
