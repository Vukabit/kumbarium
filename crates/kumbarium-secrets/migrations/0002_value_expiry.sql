-- Value expiry (v1.5): metadata the broker records and
-- surfaces, never enforces. The credential expires UPSTREAM on
-- this date; the docket does the reminding (a rotation matter
-- with a goal date), the listing marks it, and nothing here
-- blocks a read: an expired-but-working credential is the
-- human's judgment call, not the broker's.
ALTER TABLE secrets ADD COLUMN expires_at TEXT;
