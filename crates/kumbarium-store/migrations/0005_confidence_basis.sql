-- The janitor stores WHY the confidence number is what it is,
-- so the read path can explain it without opening the audit db.
-- NULL means the neutral prior with no janitor pass yet.
ALTER TABLE entries ADD COLUMN confidence_basis TEXT;
