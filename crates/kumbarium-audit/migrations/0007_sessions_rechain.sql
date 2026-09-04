-- 0007: the session column joins the ledger (D-045): agents
-- are claimed, sessions are minted, and session attribution is
-- HASHED like every other field, one recipe for the whole
-- chain. Historical rows carry the empty session. Nulling every
-- hash makes the standing backfill machinery re-chain the
-- entire ledger under the new recipe on next open: the
-- pre-adoption door (no external ledgers exist to keep
-- compatible), walked through deliberately and then closed;
-- after v1.0 a recipe change means a hash-version marker, never
-- a re-chain.
ALTER TABLE events ADD COLUMN session_id TEXT NOT NULL DEFAULT '';
UPDATE events SET hash = NULL;
