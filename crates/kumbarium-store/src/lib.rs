//! The Library: Kumbarium's memory store. Owns the SQLite schema,
//! the numbered-migration runner, and (as v0.1 lands) entry CRUD,
//! FTS5 search plumbing, supersession, and backups. The librarian
//! crate ranks; this crate stores.

#![forbid(unsafe_code)]

mod entries;
mod links;

use std::path::Path;

// Part of this crate's public API: open()/open_in_memory()
// return it, so callers get the type without a rusqlite dep.
pub use rusqlite::Connection;

pub use entries::{
  Entry, Hit, Kind, NewEntry, confirm, find_by_source, forget, get,
  namespace_id, namespaces, recall, register_namespace, remember, supersede,
};
pub use links::{Link, Rel, link, links_of, unlink};

/// Numbered migrations, applied in order inside one transaction
/// each. Append-only: a shipped migration is never edited; schema
/// changes are a new numbered file.
const MIGRATIONS: &[(i64, &str, &str)] = &[
  (1, "0001_init", include_str!("../migrations/0001_init.sql")),
  (
    2,
    "0002_entry_links",
    include_str!("../migrations/0002_entry_links.sql"),
  ),
];

#[derive(Debug, thiserror::Error)]
pub enum StoreError {
  #[error("sqlite error: {0}")]
  Sqlite(#[from] rusqlite::Error),
  #[error("migration {0} failed: {1}")]
  Migration(i64, rusqlite::Error),
  #[error("namespace {0:?} is not registered")]
  NamespaceNotRegistered(String),
  #[error("namespace {0:?} is already registered")]
  NamespaceExists(String),
  #[error("no entry with id {0:?}")]
  EntryNotFound(String),
  #[error("entry {0:?} is already superseded")]
  AlreadySuperseded(String),
  #[error("entry content is empty")]
  EmptyContent,
  #[error("cannot link entry {0:?} to itself")]
  SelfLink(String),
}

/// Open (creating if absent) the Library at `path`, applying WAL
/// mode, foreign keys, and any pending migrations.
pub fn open(path: &Path) -> Result<Connection, StoreError> {
  let conn = Connection::open(path)?;
  configure(&conn)?;
  migrate(&conn)?;
  Ok(conn)
}

/// In-memory Library with the full schema applied. Test-focused,
/// but public: an ephemeral store is legitimate (e.g. a dry run).
pub fn open_in_memory() -> Result<Connection, StoreError> {
  let conn = Connection::open_in_memory()?;
  configure(&conn)?;
  migrate(&conn)?;
  Ok(conn)
}

/// The schema version the store is at (0 = brand new).
pub fn schema_version(conn: &Connection) -> Result<i64, StoreError> {
  let v = conn.query_row(
    "SELECT COALESCE(MAX(version), 0) FROM schema_version",
    [],
    |row| row.get(0),
  )?;
  Ok(v)
}

fn configure(conn: &Connection) -> Result<(), StoreError> {
  // WAL for concurrent-reader friendliness on file databases; an
  // in-memory db reports its own mode and that is fine.
  let _mode: String =
    conn.pragma_query_value(None, "journal_mode", |row| row.get(0))?;
  conn.pragma_update(None, "journal_mode", "wal")?;
  conn.pragma_update(None, "foreign_keys", "on")?;
  conn.pragma_update(None, "synchronous", "normal")?;
  Ok(())
}

fn migrate(conn: &Connection) -> Result<(), StoreError> {
  conn.execute(
    "CREATE TABLE IF NOT EXISTS schema_version (
       version INTEGER PRIMARY KEY,
       name TEXT NOT NULL,
       applied_at TEXT NOT NULL
     )",
    [],
  )?;
  let current = schema_version(conn)?;
  for (version, name, sql) in MIGRATIONS {
    if *version <= current {
      continue;
    }
    conn
      .execute_batch(&format!("BEGIN;\n{sql}\nCOMMIT;"))
      .map_err(|e| StoreError::Migration(*version, e))?;
    conn.execute(
      "INSERT INTO schema_version (version, name, applied_at)
       VALUES (?1, ?2, ?3)",
      rusqlite::params![version, name, kumbarium_util::now_iso8601()],
    )?;
  }
  Ok(())
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn fresh_store_reaches_latest_schema() {
    let conn = open_in_memory().unwrap();
    assert_eq!(schema_version(&conn).unwrap(), 2);
  }

  #[test]
  fn migrations_are_idempotent() {
    let conn = open_in_memory().unwrap();
    // A second pass sees itself at latest and applies nothing.
    migrate(&conn).unwrap();
    assert_eq!(schema_version(&conn).unwrap(), 2);
  }

  #[test]
  fn global_namespace_is_seeded() {
    let conn = open_in_memory().unwrap();
    let path: String = conn
      .query_row(
        "SELECT path FROM namespaces WHERE path = 'global'",
        [],
        |row| row.get(0),
      )
      .unwrap();
    assert_eq!(path, "global");
  }

  #[test]
  fn fts5_indexes_and_matches_entry_content() {
    let conn = open_in_memory().unwrap();
    conn
      .execute(
        "INSERT INTO entries
           (id, namespace_id, kind, content, agent_id,
            created_at, updated_at)
         VALUES (?1, 1, 'preference', ?2, 'test-agent', ?3, ?3)",
        rusqlite::params![
          kumbarium_util::generate_id(),
          "commits are subject-only, no trailers",
          kumbarium_util::now_iso8601(),
        ],
      )
      .unwrap();
    let hits: i64 = conn
      .query_row(
        "SELECT count(*) FROM entries_fts
         WHERE entries_fts MATCH 'trailers'",
        [],
        |row| row.get(0),
      )
      .unwrap();
    assert_eq!(hits, 1, "FTS5 trigger indexed the insert");
  }

  #[test]
  fn entry_kind_check_rejects_unknown_kinds() {
    let conn = open_in_memory().unwrap();
    let err = conn.execute(
      "INSERT INTO entries
         (id, namespace_id, kind, content, agent_id,
          created_at, updated_at)
       VALUES (?1, 1, 'vibe', 'x', 'test-agent', ?2, ?2)",
      rusqlite::params![
        kumbarium_util::generate_id(),
        kumbarium_util::now_iso8601(),
      ],
    );
    assert!(err.is_err(), "CHECK constraint rejects unknown kind");
  }
}
