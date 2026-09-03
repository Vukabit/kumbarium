-- 0001: the Library's founding schema (squashed at the public
-- threshold from the six pre-release migrations, D-030; from
-- here on migrations are append-only forever).
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
  -- moved only by the janitor (D-004, D-025), explained by the
  -- stored confidence_basis.
  confidence REAL NOT NULL DEFAULT 0.5,
  -- Supersede, never delete: contradictions chain forward.
  superseded_by TEXT REFERENCES entries (id),
  created_at TEXT NOT NULL,
  updated_at TEXT NOT NULL,
  last_accessed_at TEXT,
  -- Age WITHOUT recent confirmation is the strong staleness signal.
  last_confirmed_at TEXT,
  -- Retirement, the third lifecycle door: true, kept, no longer
  -- SUGGESTED. Distinct from supersession (needs a successor)
  -- and forget (destroys); deliberately not a confidence change.
  retired_at TEXT,
  -- Optional one-line label on a version ("typo fix"). Display
  -- metadata ONLY: history collapse is gated on the measured
  -- diff, never the note (D-020 keeps content immutable).
  note TEXT,
  -- Why confidence is what it is; written only by the janitor
  -- (NULL = neutral prior, no pass yet).
  confidence_basis TEXT,
  -- Circulation status (D-027): quarantine is a status, not a
  -- place. Pending and rejected entries never surface in
  -- recall, list, grep, or chain search; approval flips pending
  -- to live; rejection keeps the entry as evidence.
  status TEXT NOT NULL DEFAULT 'live'
    CHECK (status IN ('live', 'pending', 'rejected'))
);

CREATE INDEX idx_entries_namespace ON entries (namespace_id);
CREATE INDEX idx_entries_superseded ON entries (superseded_by);

-- Tags are a filter facet, not the primary retrieval mechanism.
CREATE TABLE entry_tags (
  entry_id TEXT NOT NULL REFERENCES entries (id),
  tag TEXT NOT NULL,
  PRIMARY KEY (entry_id, tag)
) WITHOUT ROWID;

-- Typed edges: one mechanism for every relation that is not
-- supersession. 'continues' chains the parts of a split memory,
-- 'relates_to' carries the association graph, 'duplicates' /
-- 'contradicts' name janitor and merge findings. `superseded_by`
-- stays a column on entries: load-bearing for recall filtering
-- and enforced-linear, so it earns its special place.
CREATE TABLE entry_links (
  from_id TEXT NOT NULL REFERENCES entries (id),
  to_id TEXT NOT NULL REFERENCES entries (id),
  rel TEXT NOT NULL CHECK (
    rel IN (
      'continues', 'relates_to', 'duplicates', 'contradicts'
    )
  ),
  created_at TEXT NOT NULL,
  PRIMARY KEY (from_id, to_id, rel)
) WITHOUT ROWID;

CREATE INDEX idx_links_to ON entry_links (to_id);

-- Full-text index over entry content (external content table);
-- the triggers below keep it in lockstep with `entries`. Porter
-- stemming: queries phrase things differently than stored
-- content ("formatting" vs "formatted"), and both stem to one
-- root.
CREATE VIRTUAL TABLE entries_fts USING fts5 (
  content,
  content = 'entries',
  content_rowid = 'rowid',
  tokenize = 'porter unicode61'
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
