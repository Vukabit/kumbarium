-- 0002: typed edges between entries.
--
-- One mechanism for every relation that is not supersession:
-- 'continues' chains the parts of a split memory (the linked
-- list), 'relates_to' carries the association graph (imported
-- [[wiki-links]] included), 'duplicates' / 'contradicts' are
-- reserved for janitor findings. `superseded_by` stays a column
-- on entries: it is load-bearing for recall filtering and
-- enforced-linear, so it earns its special place.

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
