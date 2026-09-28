-- 0003: grants follow actors (D-056). A grant's grantee is now a
-- minted actor id; rows written before this (keyed by a claimed
-- agent name, which reaches every session claiming it) keep
-- working as NAME-WIDE legacy grants, marked here so listings
-- and the doctor can tell them apart. New name-wide grants are
-- never written.
ALTER TABLE grants ADD COLUMN grantee_kind TEXT NOT NULL DEFAULT 'name'
  CHECK (grantee_kind IN ('actor', 'name'));
