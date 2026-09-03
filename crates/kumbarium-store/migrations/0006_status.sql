-- Circulation status (D-027): quarantine is a status, not a
-- place. An entry keeps its target namespace from day one;
-- pending and rejected entries never surface in recall, list,
-- grep, or chain search. Approval flips pending -> live;
-- rejection keeps the entry as evidence of the judgment.
ALTER TABLE entries ADD COLUMN status TEXT NOT NULL DEFAULT 'live'
  CHECK (status IN ('live', 'pending', 'rejected'));
