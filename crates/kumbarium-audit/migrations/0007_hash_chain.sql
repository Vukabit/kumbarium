-- Hash chain (D-029): every event stores
-- sha256(prev_event_hash + its own canonical fields), making the
-- ledger tamper-evident by math: rewrite or remove anything and
-- every later hash breaks. Pre-chain rows are backfilled in Rust
-- on the first open after this migration (deterministic, so
-- concurrent backfills write identical values).
ALTER TABLE events ADD COLUMN hash TEXT;
