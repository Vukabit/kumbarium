-- 0001: the docket. A task is a matter before the house: filed
-- on a registered shelf (the namespace PATH is stored as text
-- and gate-checked against memory.db's registry; shelves meet
-- in the librarian, never in SQL, D-033), carrying a severity
-- and an optional goal date, awaiting a judgment. Edits are
-- supersessions (D-020); done and dropped KEEP the row; the
-- desk's status column is reused verbatim (D-027).

CREATE TABLE tasks (
  id TEXT PRIMARY KEY,
  namespace TEXT NOT NULL,
  content TEXT NOT NULL,
  agent_id TEXT NOT NULL,
  source TEXT NOT NULL DEFAULT '',
  severity TEXT NOT NULL DEFAULT 'normal' CHECK (
    severity IN ('low', 'normal', 'high', 'urgent')
  ),
  -- Optional target date (ISO day). A goal, never an alarm;
  -- the derived roadmap horizon and the creep marks come from
  -- it at read time.
  goal TEXT,
  state TEXT NOT NULL DEFAULT 'open' CHECK (
    state IN ('open', 'done', 'dropped')
  ),
  done_at TEXT,
  superseded_by TEXT REFERENCES tasks (id),
  note TEXT,
  status TEXT NOT NULL DEFAULT 'live' CHECK (
    status IN ('live', 'pending', 'rejected')
  ),
  created_at TEXT NOT NULL,
  updated_at TEXT NOT NULL
);

CREATE INDEX idx_tasks_namespace ON tasks (namespace);
CREATE INDEX idx_tasks_state ON tasks (state);
CREATE INDEX idx_tasks_superseded ON tasks (superseded_by);
