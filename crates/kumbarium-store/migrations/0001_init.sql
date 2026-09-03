-- 0001: the Library's founding schema.
--
-- Timestamps are strict ISO-8601 UTC TEXT with ms precision
-- (kumbarium-util's canonical format): lexicographic order equals
-- chronological order, and the rows stay human-readable.
-- Entry ids are UUIDv7 TEXT (time-ordered). A TEXT primary key
-- keeps SQLite's implicit rowid, which the FTS5 external-content
-- index is keyed on.

CREATE TABLE namespaces (
  id INTEGER PRIMARY KEY,
  path TEXT NOT NULL UNIQUE,
  description TEXT NOT NULL DEFAULT '',
  created_at TEXT NOT NULL
);

CREATE TABLE entries (
  id TEXT PRIMARY KEY,
  namespace_id INTEGER NOT NULL REFERENCES namespaces (id),
  -- Type-aware decay hangs off `kind`; the CHECK is the enum.
  kind TEXT NOT NULL CHECK (
    kind IN ('preference', 'project_state', 'decision', 'reference')
  ),
  content TEXT NOT NULL,
  -- Provenance: which agent wrote it, from what source/session.
  agent_id TEXT NOT NULL,
  source TEXT NOT NULL DEFAULT '',
  -- Trustworthiness of the fact itself (not query relevance);
  -- updated by the janitor, surfaced with a confidence_basis.
  confidence REAL NOT NULL DEFAULT 0.5,
  -- Supersede, never delete: contradictions chain forward.
  superseded_by TEXT REFERENCES entries (id),
  created_at TEXT NOT NULL,
  updated_at TEXT NOT NULL,
  last_accessed_at TEXT,
  -- Age WITHOUT recent confirmation is the strong staleness signal.
  last_confirmed_at TEXT
);

CREATE INDEX idx_entries_namespace ON entries (namespace_id);
CREATE INDEX idx_entries_superseded ON entries (superseded_by);

-- Tags are a filter facet, not the primary retrieval mechanism.
CREATE TABLE entry_tags (
  entry_id TEXT NOT NULL REFERENCES entries (id),
  tag TEXT NOT NULL,
  PRIMARY KEY (entry_id, tag)
) WITHOUT ROWID;

-- Full-text index over entry content (external content table);
-- the triggers below keep it in lockstep with `entries`.
CREATE VIRTUAL TABLE entries_fts USING fts5 (
  content,
  content = 'entries',
  content_rowid = 'rowid'
);

CREATE TRIGGER entries_fts_insert AFTER INSERT ON entries BEGIN
  INSERT INTO entries_fts (rowid, content)
  VALUES (new.rowid, new.content);
END;

CREATE TRIGGER entries_fts_delete AFTER DELETE ON entries BEGIN
  INSERT INTO entries_fts (entries_fts, rowid, content)
  VALUES ('delete', old.rowid, old.content);
END;

CREATE TRIGGER entries_fts_update
AFTER UPDATE OF content ON entries BEGIN
  INSERT INTO entries_fts (entries_fts, rowid, content)
  VALUES ('delete', old.rowid, old.content);
  INSERT INTO entries_fts (rowid, content)
  VALUES (new.rowid, new.content);
END;

-- The one namespace every query chain ends at.
INSERT INTO namespaces (path, description, created_at)
VALUES (
  'global',
  'Cross-project memories: preferences, style rules, standing facts.',
  strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
);
