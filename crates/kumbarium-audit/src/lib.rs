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

pub use export::{
  StoredEvent, describe_event, events_asc, render_minutes, summary, tail,
};

/// What `verify_chain` concluded: either the whole ledger checks
/// out (event count + head hash) or the first break, located.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ChainStatus {
  Intact {
    events: usize,
    head: Option<String>,
  },
  Broken {
    index: usize,
    id: String,
    at: String,
  },
}

const MIGRATIONS: &[(i64, &str, &str)] = &[
  (1, "0001_init", include_str!("../migrations/0001_init.sql")),
  (
    2,
    "0002_event_kinds",
    include_str!("../migrations/0002_event_kinds.sql"),
  ),
  (
    3,
    "0003_retire_kinds",
    include_str!("../migrations/0003_retire_kinds.sql"),
  ),
  (
    4,
    "0004_confirm_kind",
    include_str!("../migrations/0004_confirm_kind.sql"),
  ),
  (
    5,
    "0005_janitor_kind",
    include_str!("../migrations/0005_janitor_kind.sql"),
  ),
  (
    6,
    "0006_desk_kinds",
    include_str!("../migrations/0006_desk_kinds.sql"),
  ),
  (
    7,
    "0007_hash_chain",
    include_str!("../migrations/0007_hash_chain.sql"),
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
  Retire,
  Unretire,
  Confirm,
  Janitor,
  Approve,
  Reject,
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
      EventKind::Retire => "retire",
      EventKind::Unretire => "unretire",
      EventKind::Confirm => "confirm",
      EventKind::Janitor => "janitor",
      EventKind::Approve => "approve",
      EventKind::Reject => "reject",
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
  backfill_chain(&conn)?;
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
/// sort chronologically). The whole read-prev/insert runs inside
/// one IMMEDIATE transaction: chain order is id order, and the
/// write lock serializes concurrent appenders so the chain stays
/// linear under multi-process WAL (D-015).
pub fn append(conn: &Connection, event: &Event) -> Result<String, AuditError> {
  conn.execute_batch("BEGIN IMMEDIATE")?;
  let result = append_locked(conn, event);
  match &result {
    Ok(_) => conn.execute_batch("COMMIT")?,
    Err(_) => {
      let _ = conn.execute_batch("ROLLBACK");
    }
  }
  result
}

fn append_locked(
  conn: &Connection,
  event: &Event,
) -> Result<String, AuditError> {
  let id = kumbarium_util::generate_id();
  let at = kumbarium_util::now_iso8601();
  let detail = serde_json::to_string(&event.detail)?;
  let prev = head_hash(conn)?;
  let hash = event_hash(
    &prev,
    &id,
    &at,
    &event.agent_id,
    event.kind.as_str(),
    &event.scope,
    &detail,
  );
  conn.execute(
    "INSERT INTO events (id, at, agent_id, kind, scope, detail, hash)
     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
    rusqlite::params![
      id,
      at,
      event.agent_id,
      event.kind.as_str(),
      event.scope,
      detail,
      hash,
    ],
  )?;
  Ok(id)
}

/// The chain head's hash; the empty string before any event (the
/// documented genesis value).
fn head_hash(conn: &Connection) -> Result<String, AuditError> {
  conn
    .query_row(
      "SELECT COALESCE(hash, '') FROM events
       ORDER BY id DESC LIMIT 1",
      [],
      |row| row.get(0),
    )
    .or_else(|e| match e {
      rusqlite::Error::QueryReturnedNoRows => Ok(String::new()),
      other => Err(other.into()),
    })
}

/// The canonical hash of one event given its predecessor's hash.
/// Every field is length-prefixed, so no delimiter inside any
/// field can forge a boundary.
fn event_hash(
  prev: &str,
  id: &str,
  at: &str,
  agent_id: &str,
  kind: &str,
  scope: &str,
  detail: &str,
) -> String {
  let mut input = Vec::new();
  for field in [prev, id, at, agent_id, kind, scope, detail] {
    input.extend_from_slice(field.len().to_string().as_bytes());
    input.push(b':');
    input.extend_from_slice(field.as_bytes());
  }
  kumbarium_util::sha256_hex(&input)
}

/// One-time (and idempotent) chaining of rows written before
/// migration 0007. Deterministic: concurrent backfills compute
/// identical values, so the IMMEDIATE lock only orders them.
pub fn backfill_chain(conn: &Connection) -> Result<(), AuditError> {
  let unhashed: i64 = conn.query_row(
    "SELECT count(*) FROM events WHERE hash IS NULL",
    [],
    |row| row.get(0),
  )?;
  if unhashed == 0 {
    return Ok(());
  }
  conn.execute_batch("BEGIN IMMEDIATE")?;
  let result = (|| -> Result<(), AuditError> {
    let rows: Vec<(String, String, String, String, String, String)> = {
      let mut stmt = conn.prepare(
        "SELECT id, at, agent_id, kind, scope, detail
         FROM events ORDER BY id ASC",
      )?;
      stmt
        .query_map([], |row| {
          Ok((
            row.get(0)?,
            row.get(1)?,
            row.get(2)?,
            row.get(3)?,
            row.get(4)?,
            row.get(5)?,
          ))
        })?
        .collect::<Result<Vec<_>, _>>()?
    };
    let mut prev = String::new();
    for (id, at, agent_id, kind, scope, detail) in rows {
      let hash = event_hash(&prev, &id, &at, &agent_id, &kind, &scope, &detail);
      conn.execute(
        "UPDATE events SET hash = ?1 WHERE id = ?2",
        rusqlite::params![hash, id],
      )?;
      prev = hash;
    }
    Ok(())
  })();
  match &result {
    Ok(_) => conn.execute_batch("COMMIT")?,
    Err(_) => {
      let _ = conn.execute_batch("ROLLBACK");
    }
  }
  result
}

/// Recompute the whole chain and compare: anyone holding the file
/// can prove the ledger unaltered, or name the first broken link.
pub fn verify_chain(conn: &Connection) -> Result<ChainStatus, AuditError> {
  let mut stmt = conn.prepare(
    "SELECT id, at, agent_id, kind, scope, detail,
            COALESCE(hash, '') FROM events ORDER BY id ASC",
  )?;
  let rows = stmt
    .query_map([], |row| {
      Ok((
        row.get::<_, String>(0)?,
        row.get::<_, String>(1)?,
        row.get::<_, String>(2)?,
        row.get::<_, String>(3)?,
        row.get::<_, String>(4)?,
        row.get::<_, String>(5)?,
        row.get::<_, String>(6)?,
      ))
    })?
    .collect::<Result<Vec<_>, _>>()?;
  let mut prev = String::new();
  for (index, (id, at, agent_id, kind, scope, detail, stored)) in
    rows.iter().enumerate()
  {
    let expected = event_hash(&prev, id, at, agent_id, kind, scope, detail);
    if *stored != expected {
      return Ok(ChainStatus::Broken {
        index: index + 1,
        id: id.clone(),
        at: at.clone(),
      });
    }
    prev = expected;
  }
  Ok(ChainStatus::Intact {
    events: rows.len(),
    head: rows.last().map(|r| r.6.clone()),
  })
}

fn configure(conn: &Connection) -> Result<(), AuditError> {
  conn.pragma_update(None, "journal_mode", "wal")?;
  // Multi-process by design (D-015): a writer briefly holding
  // the db must make peers wait, not error with SQLITE_BUSY.
  conn.pragma_update(None, "busy_timeout", 5000)?;
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
  fn chain_is_intact_after_appends() {
    let conn = open_in_memory().unwrap();
    for kind in [EventKind::Remember, EventKind::Recall, EventKind::Confirm] {
      append(&conn, &event(kind)).unwrap();
    }
    match verify_chain(&conn).unwrap() {
      ChainStatus::Intact { events, head } => {
        assert_eq!(events, 3);
        assert_eq!(head.unwrap().len(), 64);
      }
      broken => panic!("chain should verify: {broken:?}"),
    }
  }

  #[test]
  fn rewriting_an_event_breaks_the_chain_there() {
    let conn = open_in_memory().unwrap();
    let ids: Vec<String> = (0..4)
      .map(|_| append(&conn, &event(EventKind::Remember)).unwrap())
      .collect();
    conn
      .execute(
        "UPDATE events SET detail = '{\"query\":\"forged\"}'
         WHERE id = ?1",
        [&ids[1]],
      )
      .unwrap();
    match verify_chain(&conn).unwrap() {
      ChainStatus::Broken { index, id, .. } => {
        assert_eq!(index, 2, "break located at the rewrite");
        assert_eq!(id, ids[1]);
      }
      intact => panic!("tamper must be detected: {intact:?}"),
    }
  }

  #[test]
  fn removing_an_event_breaks_the_chain() {
    let conn = open_in_memory().unwrap();
    let ids: Vec<String> = (0..3)
      .map(|_| append(&conn, &event(EventKind::Recall)).unwrap())
      .collect();
    conn
      .execute("DELETE FROM events WHERE id = ?1", [&ids[1]])
      .unwrap();
    assert!(matches!(
      verify_chain(&conn).unwrap(),
      ChainStatus::Broken { index: 2, .. }
    ));
  }

  #[test]
  fn backfill_chains_pre_migration_rows() {
    let conn = open_in_memory().unwrap();
    // Simulate the pre-0007 era: rows inserted without hashes.
    for i in 0..3 {
      conn
        .execute(
          "INSERT INTO events (id, at, agent_id, kind, scope, detail)
           VALUES (?1, ?2, 'old-agent', 'recall', 'global', '{}')",
          rusqlite::params![
            kumbarium_util::generate_id(),
            format!("2026-09-01T00:0{i}:00.000Z"),
          ],
        )
        .unwrap();
    }
    backfill_chain(&conn).unwrap();
    // Idempotent, and appends continue the backfilled chain.
    backfill_chain(&conn).unwrap();
    append(&conn, &event(EventKind::Confirm)).unwrap();
    assert!(matches!(
      verify_chain(&conn).unwrap(),
      ChainStatus::Intact { events: 4, .. }
    ));
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
