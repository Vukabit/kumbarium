-- 0004: supersession notes.
--
-- An optional one-line label on a version describing how it
-- came to be ("typo fix", "revert to dfeb4d7c"). Display
-- metadata ONLY: collapse decisions in history are gated on the
-- measured diff, never on the note, so a note can inform but
-- can never hide a change. Content stays immutable (D-020);
-- notes exist so trivial corrections are legible, not so edits
-- are possible.

ALTER TABLE entries ADD COLUMN note TEXT;
