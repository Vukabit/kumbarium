-- 0002: rebuild the FTS index with porter stemming.
--
-- Recall queries phrase things differently than stored content
-- ("formatting" vs "formatted"); the porter tokenizer stems both
-- to one root so they match. FTS5 tokenizers are fixed at CREATE
-- time and 0001 already shipped, so this drops and recreates the
-- index (external content: the entries table itself is untouched)
-- and reindexes existing rows.

DROP TRIGGER entries_fts_insert;
DROP TRIGGER entries_fts_delete;
DROP TRIGGER entries_fts_update;
DROP TABLE entries_fts;

CREATE VIRTUAL TABLE entries_fts USING fts5 (
  content,
  content = 'entries',
  content_rowid = 'rowid',
  tokenize = 'porter unicode61'
);

INSERT INTO entries_fts (rowid, content)
SELECT rowid, content FROM entries;

CREATE TRIGGER entries_fts_insert AFTER INSERT ON entries BEGIN
  INSERT INTO entries_fts (rowid, content)
  VALUES (new.rowid, new.content);
END;

CREATE TRIGGER entries_fts_delete AFTER DELETE ON entries BEGIN
  INSERT INTO entries_fts (entries_fts, rowid, content)
  VALUES ('delete', old.rowid, old.content);
END;

CREATE TRIGGER entries_fts_update
AFTER UPDATE OF content ON entries BEGIN
  INSERT INTO entries_fts (entries_fts, rowid, content)
  VALUES ('delete', old.rowid, old.content);
  INSERT INTO entries_fts (rowid, content)
  VALUES (new.rowid, new.content);
END;
