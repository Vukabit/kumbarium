-- 0001: the restricted stacks. A secret is sealed at rest
-- (XChaCha20-Poly1305 under the keystore-held master key,
-- versioned envelope, AAD binds namespace+name so ciphertext
-- cannot be re-shelved); rotation supersedes and SHREDS the
-- ancestor's value (the one deliberate bend of
-- supersede-never-delete: an old key is a liability, not a
-- memory). Grants are deny-by-default, mode 'reveal' now with
-- 'use' reserved (the egress-broker seam), expires_at is the
-- lease column checked at read time. Human-only writes in v1.

CREATE TABLE secrets (
  id TEXT PRIMARY KEY,
  namespace TEXT NOT NULL,
  name TEXT NOT NULL,
  -- The sealed envelope, or NULL once shredded (the row stays:
  -- history keeps the skeleton, never the material).
  sealed BLOB,
  agent_id TEXT NOT NULL,
  superseded_by TEXT REFERENCES secrets (id),
  note TEXT,
  shredded_at TEXT,
  created_at TEXT NOT NULL,
  updated_at TEXT NOT NULL
);

CREATE INDEX idx_secrets_name ON secrets (namespace, name);
CREATE INDEX idx_secrets_superseded ON secrets (superseded_by);

CREATE TABLE grants (
  namespace TEXT NOT NULL,
  name TEXT NOT NULL,
  agent_id TEXT NOT NULL,
  mode TEXT NOT NULL DEFAULT 'reveal' CHECK (
    mode IN ('reveal', 'use')
  ),
  expires_at TEXT,
  created_at TEXT NOT NULL,
  PRIMARY KEY (namespace, name, agent_id)
) WITHOUT ROWID;

-- How this shelf seals: 'keystore' or (explicit human choice
-- on an absent substrate) 'plaintext'. One row, written at
-- shelf creation, never silently changed.
CREATE TABLE sealing (
  id INTEGER PRIMARY KEY CHECK (id = 1),
  mode TEXT NOT NULL CHECK (mode IN ('keystore', 'plaintext')),
  created_at TEXT NOT NULL
);
