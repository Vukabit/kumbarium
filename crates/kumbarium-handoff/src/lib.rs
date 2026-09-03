//! Handoffs (docs/design/handoffs.md, D-036): one standing
//! briefing per namespace, where writing IS superseding and the
//! chain is the scope's session diary. Held to the section
//! inheritance contract (D-033): governed like memory (identity,
//! desk, supersession, id grammar), never recalled like memory
//! (no split, no links, no FTS, no confidence). A handoff is a
//! note, not a matter: no severity, no goal, no state machine.

#![forbid(unsafe_code)]

use std::path::Path;

use rusqlite::params;

pub use rusqlite::Connection;

const MIGRATIONS: &[(i64, &str, &str)] =
  &[(1, "0001_init", include_str!("../migrations/0001_init.sql"))];

/// A briefing that cannot fit is hiding a design document that
/// belongs in memory.
pub const MAX_CONTENT: usize = 4000;

#[derive(Debug, thiserror::Error)]
pub enum HandoffError {
  #[error("sqlite error: {0}")]
  Sqlite(#[from] rusqlite::Error),
  #[error("migration {0} failed: {1}")]
  Migration(i64, rusqlite::Error),
  #[error("no handoff with id {0:?}")]
  HandoffNotFound(String),
  #[error("id fragment {0:?} matches more than one handoff")]
  AmbiguousId(String),
  #[error("handoff {0:?} is not pending")]
  NotPending(String),
  #[error("handoff content is empty")]
  EmptyContent,
  #[error(
    "handoff exceeds {0} bytes; a briefing that cannot fit is \
     hiding a design document that belongs in memory"
  )]
  ContentTooLong(usize),
}

/// Circulation status, D-027 verbatim.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Status {
  Live,
  Pending,
  Rejected,
}

impl Status {
  pub fn as_str(self) -> &'static str {
    match self {
      Status::Live => "live",
      Status::Pending => "pending",
      Status::Rejected => "rejected",
    }
  }

  pub fn parse(s: &str) -> Option<Status> {
    match s {
      "live" => Some(Status::Live),
      "pending" => Some(Status::Pending),
      "rejected" => Some(Status::Rejected),
      _ => None,
    }
  }
}

/// A briefing as left.
#[derive(Debug, Clone)]
pub struct Handoff {
  pub id: String,
  pub namespace: String,
  pub content: String,
  pub agent_id: String,
  pub source: String,
  pub superseded_by: Option<String>,
  pub note: Option<String>,
  pub status: Status,
  pub created_at: String,
  pub updated_at: String,
}

/// Open (creating if absent) the handoff shelf at `path`.
pub fn open(path: &Path) -> Result<Connection, HandoffError> {
  let conn = Connection::open(path)?;
  configure(&conn)?;
  migrate(&conn)?;
  Ok(conn)
}

/// In-memory shelf with the schema applied (tests, dry runs).
pub fn open_in_memory() -> Result<Connection, HandoffError> {
  let conn = Connection::open_in_memory()?;
  configure(&conn)?;
  migrate(&conn)?;
  Ok(conn)
}

fn configure(conn: &Connection) -> Result<(), HandoffError> {
  conn.pragma_update(None, "journal_mode", "wal")?;
  conn.pragma_update(None, "busy_timeout", 5000)?;
  conn.pragma_update(None, "foreign_keys", "on")?;
  conn.pragma_update(None, "synchronous", "normal")?;
  Ok(())
}

fn migrate(conn: &Connection) -> Result<(), HandoffError> {
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
      .map_err(|e| HandoffError::Migration(*version, e))?;
    conn.execute(
      "INSERT INTO schema_version (version, name, applied_at)
       VALUES (?1, ?2, ?3)",
      params![version, name, kumbarium_util::now_iso8601()],
    )?;
  }
  Ok(())
}

/// The short display form of an id: its last 8 hex chars.
pub fn short_id(id: &str) -> &str {
  id.get(id.len().saturating_sub(8)..).unwrap_or(id)
}

/// Leave a briefing. A LIVE write supersedes the shelf's
/// standing head (writing IS superseding, one live head per
/// namespace by construction); a PENDING write chains onto the
/// same writer's pending head for the shelf, or stands alone
/// awaiting the desk, and supersedes nothing live.
pub fn write_handoff(
  conn: &Connection,
  namespace: &str,
  content: &str,
  agent_id: &str,
  source: &str,
  status: Status,
) -> Result<Handoff, HandoffError> {
  if content.trim().is_empty() {
    return Err(HandoffError::EmptyContent);
  }
  if content.len() > MAX_CONTENT {
    return Err(HandoffError::ContentTooLong(MAX_CONTENT));
  }
  conn.execute_batch("BEGIN IMMEDIATE")?;
  let result = write_locked(conn, namespace, content, agent_id, source, status);
  match &result {
    Ok(_) => conn.execute_batch("COMMIT")?,
    Err(_) => {
      let _ = conn.execute_batch("ROLLBACK");
    }
  }
  result
}

fn write_locked(
  conn: &Connection,
  namespace: &str,
  content: &str,
  agent_id: &str,
  source: &str,
  status: Status,
) -> Result<Handoff, HandoffError> {
  let prior: Option<String> = match status {
    Status::Live => conn
      .query_row(
        "SELECT id FROM handoffs
         WHERE namespace = ?1 AND status = 'live'
           AND superseded_by IS NULL",
        [namespace],
        |row| row.get(0),
      )
      .map(Some)
      .or_else(none_on_empty)?,
    Status::Pending => conn
      .query_row(
        "SELECT id FROM handoffs
         WHERE namespace = ?1 AND status = 'pending'
           AND superseded_by IS NULL AND agent_id = ?2",
        params![namespace, agent_id],
        |row| row.get(0),
      )
      .map(Some)
      .or_else(none_on_empty)?,
    Status::Rejected => None,
  };
  let id = kumbarium_util::generate_id();
  let now = kumbarium_util::now_iso8601();
  conn.execute(
    "INSERT INTO handoffs
       (id, namespace, content, agent_id, source, status,
        created_at, updated_at)
     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?7)",
    params![
      id,
      namespace,
      content,
      agent_id,
      source,
      status.as_str(),
      now
    ],
  )?;
  if let Some(prev) = prior {
    conn.execute(
      "UPDATE handoffs SET superseded_by = ?1, updated_at = ?2
       WHERE id = ?3",
      params![id, now, prev],
    )?;
  }
  get(conn, &id)
}

fn none_on_empty<T>(e: rusqlite::Error) -> Result<Option<T>, rusqlite::Error> {
  match e {
    rusqlite::Error::QueryReturnedNoRows => Ok(None),
    other => Err(other),
  }
}

/// The shelf's standing briefing (live head), if any.
pub fn standing(
  conn: &Connection,
  namespace: &str,
) -> Result<Option<Handoff>, HandoffError> {
  let mut stmt = conn.prepare(
    "SELECT id, namespace, content, agent_id, source,
            superseded_by, note, status, created_at, updated_at
     FROM handoffs
     WHERE namespace = ?1 AND status = 'live'
       AND superseded_by IS NULL",
  )?;
  stmt
    .query_row([namespace], row_to_handoff)
    .map(Some)
    .or_else(|e| match e {
      rusqlite::Error::QueryReturnedNoRows => Ok(None),
      other => Err(other.into()),
    })
}

/// Every shelf's standing briefing, by namespace.
pub fn standings(conn: &Connection) -> Result<Vec<Handoff>, HandoffError> {
  let mut stmt = conn.prepare(
    "SELECT id, namespace, content, agent_id, source,
            superseded_by, note, status, created_at, updated_at
     FROM handoffs
     WHERE status = 'live' AND superseded_by IS NULL
     ORDER BY namespace",
  )?;
  let rows = stmt
    .query_map([], row_to_handoff)?
    .collect::<Result<Vec<_>, _>>()?;
  Ok(rows)
}

/// Pending briefings, oldest first: the desk's queue.
pub fn pending_handoffs(
  conn: &Connection,
) -> Result<Vec<Handoff>, HandoffError> {
  let mut stmt = conn.prepare(
    "SELECT id, namespace, content, agent_id, source,
            superseded_by, note, status, created_at, updated_at
     FROM handoffs
     WHERE status = 'pending' AND superseded_by IS NULL
     ORDER BY created_at ASC",
  )?;
  let rows = stmt
    .query_map([], row_to_handoff)?
    .collect::<Result<Vec<_>, _>>()?;
  Ok(rows)
}

/// Fetch one briefing by full id.
pub fn get(conn: &Connection, id: &str) -> Result<Handoff, HandoffError> {
  let mut stmt = conn.prepare(
    "SELECT id, namespace, content, agent_id, source,
            superseded_by, note, status, created_at, updated_at
     FROM handoffs WHERE id = ?1",
  )?;
  stmt.query_row([id], row_to_handoff).map_err(|e| match e {
    rusqlite::Error::QueryReturnedNoRows => {
      HandoffError::HandoffNotFound(id.to_string())
    }
    other => other.into(),
  })
}

/// Resolve an id fragment, same grammar as every shelf.
pub fn resolve_id(
  conn: &Connection,
  fragment: &str,
) -> Result<String, HandoffError> {
  if kumbarium_util::is_valid_id(fragment) {
    return Ok(fragment.to_string());
  }
  let hexish = fragment.bytes().all(|b| b.is_ascii_hexdigit() || b == b'-');
  if fragment.len() < 4 || !hexish {
    return Err(HandoffError::HandoffNotFound(fragment.to_string()));
  }
  let mut stmt =
    conn.prepare("SELECT id FROM handoffs WHERE id LIKE ?1 LIMIT 2")?;
  let matches = stmt
    .query_map([format!("%{fragment}%")], |row| row.get::<_, String>(0))?
    .collect::<Result<Vec<_>, _>>()?;
  match matches.as_slice() {
    [] => Err(HandoffError::HandoffNotFound(fragment.to_string())),
    [id] => Ok(id.clone()),
    _ => Err(HandoffError::AmbiguousId(fragment.to_string())),
  }
}

/// Promote a pending briefing: it becomes the shelf's standing
/// head, superseding the current live one so the one-live-head
/// invariant survives the desk (approval is itself a "writing
/// is superseding" act, witnessed by the caller).
pub fn approve(conn: &Connection, id: &str) -> Result<(), HandoffError> {
  conn.execute_batch("BEGIN IMMEDIATE")?;
  let result = approve_locked(conn, id);
  match &result {
    Ok(_) => conn.execute_batch("COMMIT")?,
    Err(_) => {
      let _ = conn.execute_batch("ROLLBACK");
    }
  }
  result
}

fn approve_locked(conn: &Connection, id: &str) -> Result<(), HandoffError> {
  let h = get(conn, id)?;
  if h.status != Status::Pending || h.superseded_by.is_some() {
    return Err(HandoffError::NotPending(id.to_string()));
  }
  let now = kumbarium_util::now_iso8601();
  let live: Option<String> = conn
    .query_row(
      "SELECT id FROM handoffs
       WHERE namespace = ?1 AND status = 'live'
         AND superseded_by IS NULL",
      [&h.namespace],
      |row| row.get(0),
    )
    .map(Some)
    .or_else(none_on_empty)?;
  if let Some(prev) = live {
    conn.execute(
      "UPDATE handoffs SET superseded_by = ?1, updated_at = ?2
       WHERE id = ?3",
      params![id, now, prev],
    )?;
  }
  conn.execute(
    "UPDATE handoffs SET status = 'live', updated_at = ?1
     WHERE id = ?2",
    params![now, id],
  )?;
  Ok(())
}

/// Decline a pending briefing, kept as evidence of the judgment.
pub fn reject(conn: &Connection, id: &str) -> Result<(), HandoffError> {
  let n = conn.execute(
    "UPDATE handoffs SET status = 'rejected', updated_at = ?1
     WHERE id = ?2 AND status = 'pending'",
    params![kumbarium_util::now_iso8601(), id],
  )?;
  if n == 0 {
    return Err(HandoffError::NotPending(id.to_string()));
  }
  Ok(())
}

/// A briefing's chain, oldest first: the scope's session diary.
pub fn history(
  conn: &Connection,
  id: &str,
) -> Result<Vec<Handoff>, HandoffError> {
  let mut chain = vec![get(conn, id)?];
  loop {
    let cur = &chain[0];
    let prev: Option<String> = conn
      .query_row(
        "SELECT id FROM handoffs WHERE superseded_by = ?1",
        [&cur.id],
        |row| row.get(0),
      )
      .map(Some)
      .or_else(none_on_empty)?;
    match prev {
      Some(p) if chain.len() < 1000 => chain.insert(0, get(conn, &p)?),
      _ => break,
    }
  }
  loop {
    let cur = chain.last().expect("nonempty");
    match &cur.superseded_by {
      Some(next) if chain.len() < 1000 => {
        let next = next.clone();
        chain.push(get(conn, &next)?);
      }
      _ => break,
    }
  }
  Ok(chain)
}

fn row_to_handoff(row: &rusqlite::Row<'_>) -> Result<Handoff, rusqlite::Error> {
  let status_raw: String = row.get(7)?;
  let status = Status::parse(&status_raw).ok_or_else(|| {
    rusqlite::Error::FromSqlConversionFailure(
      7,
      rusqlite::types::Type::Text,
      format!("unknown status {status_raw:?}").into(),
    )
  })?;
  Ok(Handoff {
    id: row.get(0)?,
    namespace: row.get(1)?,
    content: row.get(2)?,
    agent_id: row.get(3)?,
    source: row.get(4)?,
    superseded_by: row.get(5)?,
    note: row.get(6)?,
    status,
    created_at: row.get(8)?,
    updated_at: row.get(9)?,
  })
}

#[cfg(test)]
mod tests {
  use super::*;

  fn live_write(conn: &Connection, ns: &str, content: &str) -> Handoff {
    write_handoff(conn, ns, content, "agent-a", "test", Status::Live).unwrap()
  }

  #[test]
  fn writing_is_superseding_one_live_head() {
    let conn = open_in_memory().unwrap();
    let v1 = live_write(&conn, "project/x", "s1: wired the adapter");
    let v2 = live_write(&conn, "project/x", "s2: adapter shipped");
    let head = standing(&conn, "project/x").unwrap().unwrap();
    assert_eq!(head.id, v2.id);
    let chain = history(&conn, &v1.id).unwrap();
    assert_eq!(chain.len(), 2, "the diary chains");
    // A sibling shelf is untouched.
    assert!(standing(&conn, "project/y").unwrap().is_none());
  }

  #[test]
  fn pending_never_stands_and_approval_supersedes() {
    let conn = open_in_memory().unwrap();
    let live = live_write(&conn, "project/x", "trusted state");
    let pend = write_handoff(
      &conn,
      "project/x",
      "vendored claim of state",
      "vendor-bot",
      "",
      Status::Pending,
    )
    .unwrap();
    // The standing head is still the trusted one.
    assert_eq!(standing(&conn, "project/x").unwrap().unwrap().id, live.id);
    assert_eq!(pending_handoffs(&conn).unwrap().len(), 1);
    // Approval makes it THE head and chains the old one under it.
    approve(&conn, &pend.id).unwrap();
    let head = standing(&conn, "project/x").unwrap().unwrap();
    assert_eq!(head.id, pend.id);
    assert_eq!(
      get(&conn, &live.id).unwrap().superseded_by.as_deref(),
      Some(pend.id.as_str()),
      "one live head survives the desk"
    );
  }

  #[test]
  fn rejection_keeps_the_row_out_of_standing() {
    let conn = open_in_memory().unwrap();
    let pend = write_handoff(
      &conn,
      "project/x",
      "poison frame",
      "vendor-bot",
      "",
      Status::Pending,
    )
    .unwrap();
    reject(&conn, &pend.id).unwrap();
    assert!(standing(&conn, "project/x").unwrap().is_none());
    assert_eq!(get(&conn, &pend.id).unwrap().status, Status::Rejected);
    assert!(matches!(
      approve(&conn, &pend.id),
      Err(HandoffError::NotPending(_))
    ));
  }

  #[test]
  fn a_pending_writer_chains_their_own_drafts() {
    let conn = open_in_memory().unwrap();
    let p1 = write_handoff(
      &conn,
      "project/x",
      "draft one",
      "vendor-bot",
      "",
      Status::Pending,
    )
    .unwrap();
    let p2 = write_handoff(
      &conn,
      "project/x",
      "draft two",
      "vendor-bot",
      "",
      Status::Pending,
    )
    .unwrap();
    assert_eq!(pending_handoffs(&conn).unwrap().len(), 1, "one per writer");
    assert_eq!(
      get(&conn, &p1.id).unwrap().superseded_by.as_deref(),
      Some(p2.id.as_str())
    );
  }

  #[test]
  fn oversized_briefings_are_refused() {
    let conn = open_in_memory().unwrap();
    let long = "state ".repeat(1000);
    assert!(matches!(
      write_handoff(&conn, "project/x", &long, "a", "", Status::Live),
      Err(HandoffError::ContentTooLong(_))
    ));
  }
}
