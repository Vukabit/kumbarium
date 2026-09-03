-- 0003: retirement, the third lifecycle door.
--
-- An entry can be true, worth keeping in history, and no longer
-- worth SUGGESTING: retired. Distinct from supersession (which
-- requires a successor) and forget (which destroys). Deliberately
-- NOT a confidence change: relevance and trustworthiness stay
-- separate judgments (D-004). Recall filters retired entries;
-- history and continues-sets keep them; unretire reverses.

ALTER TABLE entries ADD COLUMN retired_at TEXT;
