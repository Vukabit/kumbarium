//! The witness: Kumbarium's append-only audit log. Every
//! agent-to-librarian transaction lands here as a structured
//! event in a SEPARATE SQLite file (audit volume never contends
//! with the Library's WAL; this crate cannot even see the
//! Library, by dependency graph).
//!
//! v0.1 appends synchronously. The bounded buffered writer with
//! halt/resume watermarks (docs/design) replaces the direct
//! append when traffic warrants it; the event schema is already
//! shaped for that.

#![forbid(unsafe_code)]

mod export;

use std::path::Path;

use rusqlite::Connection;

pub use export::{StoredEvent, events_asc, render_minutes, tail};

const MIGRATIONS: &[(i64, &str, &str)] = &[
  (1, "0001_init", include_str!("../migrations/0001_init.sql")),
  (
    2,
    "0002_event_kinds",
    include_str!("../migrations/0002_event_kinds.sql"),
  ),
];

#[derive(Debug, thiserror::Error)]
pub enum AuditError {
  #[error("sqlite error: {0}")]
  Sqlite(#[from] rusqlite::Error),
  #[error("migration {0} failed: {1}")]
  Migration(i64, rusqlite::Error),
  #[error("detail is not serializable: {0}")]
  Detail(#[from] serde_json::Error),
}

/// What happened. One row per librarian transaction.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EventKind {
  Recall,
  Remember,
  Supersede,
  Forget,
  EvalRun,
  Link,
  Import,
}

impl EventKind {
  fn as_str(self) -> &'static str {
    match self {
      EventKind::Recall => "recall",
      EventKind::Remember => "remember",
      EventKind::Supersede => "supersede",
      EventKind::Forget => "forget",
      EventKind::EvalRun => "eval_run",
      EventKind::Link => "link",
      EventKind::Import => "import",
    }
  }
}

/// A recorded transaction: who, when, what, in which scope, with
/// a kind-specific JSON `detail` payload.
#[derive(Debug, Clone)]
pub struct Event {
  pub agent_id: String,
  pub kind: EventKind,
  pub scope: String,
  pub detail: serde_json::Value,
}

/// Open (creating if absent) the audit log at `path`.
pub fn open(path: &Path) -> Result<Connection, AuditError> {
  let conn = Connection::open(path)?;
  configure(&conn)?;
  migrate(&conn)?;
  Ok(conn)
}

/// In-memory audit log with the schema applied (tests, dry runs).
pub fn open_in_memory() -> Result<Connection, AuditError> {
  let conn = Connection::open_in_memory()?;
  configure(&conn)?;
  migrate(&conn)?;
  Ok(conn)
}

/// Append one event. Returns the event's id (UUIDv7, so event ids
/// sort chronologically).
pub fn append(conn: &Connection, event: &Event) -> Result<String, AuditError> {
  let id = kumbarium_util::generate_id();
  conn.execute(
    "INSERT INTO events (id, at, agent_id, kind, scope, detail)
     VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
    rusqlite::params![
      id,
      kumbarium_util::now_iso8601(),
      event.agent_id,
      event.kind.as_str(),
      event.scope,
      serde_json::to_string(&event.detail)?,
    ],
  )?;
  Ok(id)
}

fn configure(conn: &Connection) -> Result<(), AuditError> {
  conn.pragma_update(None, "journal_mode", "wal")?;
  conn.pragma_update(None, "synchronous", "normal")?;
  Ok(())
}

fn migrate(conn: &Connection) -> Result<(), AuditError> {
  conn.execute(
    "CREATE TABLE IF NOT EXISTS schema_version (
       version INTEGER PRIMARY KEY,
       name TEXT NOT NULL,
       applied_at TEXT NOT NULL
     )",
    [],
  )?;
  let current: i64 = conn.query_row(
    "SELECT COALESCE(MAX(version), 0) FROM schema_version",
    [],
    |row| row.get(0),
  )?;
  for (version, name, sql) in MIGRATIONS {
    if *version <= current {
      continue;
    }
    conn
      .execute_batch(&format!("BEGIN;\n{sql}\nCOMMIT;"))
      .map_err(|e| AuditError::Migration(*version, e))?;
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

  fn event(kind: EventKind) -> Event {
    Event {
      agent_id: "test-agent".into(),
      kind,
      scope: "project/demo".into(),
      detail: serde_json::json!({ "query": "commit style" }),
    }
  }

  #[test]
  fn appends_and_reads_back() {
    let conn = open_in_memory().unwrap();
    let id = append(&conn, &event(EventKind::Recall)).unwrap();
    assert!(kumbarium_util::is_valid_id(&id));
    let (kind, scope): (String, String) = conn
      .query_row(
        "SELECT kind, scope FROM events WHERE id = ?1",
        [&id],
        |row| Ok((row.get(0)?, row.get(1)?)),
      )
      .unwrap();
    assert_eq!(kind, "recall");
    assert_eq!(scope, "project/demo");
  }

  #[test]
  fn event_ids_sort_chronologically() {
    let conn = open_in_memory().unwrap();
    let a = append(&conn, &event(EventKind::Remember)).unwrap();
    let b = append(&conn, &event(EventKind::Recall)).unwrap();
    assert!(a < b, "UUIDv7 ids order by mint time");
  }

  #[test]
  fn detail_round_trips_as_json() {
    let conn = open_in_memory().unwrap();
    let id = append(&conn, &event(EventKind::EvalRun)).unwrap();
    let raw: String = conn
      .query_row("SELECT detail FROM events WHERE id = ?1", [&id], |row| {
        row.get(0)
      })
      .unwrap();
    let parsed: serde_json::Value = serde_json::from_str(&raw).unwrap();
    assert_eq!(parsed["query"], "commit style");
  }
}
