-- The reading room (D-043). One table: cards on the table.
-- ACTIVE is computed at read time (released_at IS NULL and now
-- within ttl of renewed_at); expiry is an absence, not an
-- event, so there is no reaper and no stored deadline to go
-- stale when config changes. Released and broken cards keep
-- their rows: the drawer, not the bin.
CREATE TABLE leases (
  id TEXT PRIMARY KEY,
  namespace TEXT NOT NULL,
  resource TEXT NOT NULL,
  agent_id TEXT NOT NULL,
  note TEXT,
  taken_at TEXT NOT NULL,
  renewed_at TEXT NOT NULL,
  released_at TEXT,
  created_at TEXT NOT NULL
);

CREATE INDEX idx_leases_ns ON leases (namespace);
CREATE INDEX idx_leases_agent ON leases (agent_id);
