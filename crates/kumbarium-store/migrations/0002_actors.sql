-- 0002: minted actors (D-056). Identity's third leg beside the
-- claimed name and the minted session: an actor the librarian
-- mints and a human can name, merge, and retire. The ledger
-- attributes events through a hashed actor_bind per session;
-- this table is only the catalog of names.
--
-- key: the auto-mint key (agent: claimed name + workspace;
-- human: identity source + value). NULL for actors a human
-- registered by name, which bind only when asked for
-- explicitly. merged_into: a registry fact, never a rewrite;
-- reads follow it to the surviving actor.

CREATE TABLE actors (
  id TEXT PRIMARY KEY,
  name TEXT NOT NULL UNIQUE,
  kind TEXT NOT NULL CHECK (kind IN ('agent', 'human')),
  claimed TEXT NOT NULL DEFAULT '',
  workspace TEXT NOT NULL DEFAULT '',
  key TEXT UNIQUE,
  created_at TEXT NOT NULL,
  merged_into TEXT REFERENCES actors (id),
  retired_at TEXT,
  note TEXT
);

-- Writes carry the actor that made them (NULL before D-056 and
-- for imported entries: actor ids are local, like confidence).
ALTER TABLE entries ADD COLUMN actor_id TEXT;
CREATE INDEX idx_entries_actor ON entries (actor_id);
