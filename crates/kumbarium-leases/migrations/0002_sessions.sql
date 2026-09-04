-- Sessions are minted, agents are claimed (D-044). A holder is
-- (agent_id, session_id): the librarian mints a session id per
-- serve process at initialize, so two sessions of the SAME
-- agent name are different holders and warn each other, which
-- is the reading room's primary case (self-reported names made
-- same-name sessions invisible to each other). Pre-migration
-- cards get the empty session: distinguishable, never
-- renewable by a real session.
ALTER TABLE leases ADD COLUMN session_id TEXT NOT NULL DEFAULT '';
