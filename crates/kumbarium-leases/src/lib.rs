//! The reading room (docs/design/leases.md, D-043): coordination
//! leases, the section's third resource after the docket and
//! the handoffs. A lease is a reservation card: namespace +
//! free resource string + holder. Collisions WARN, never block
//! (identity is self-reported at this tier, so blocking would
//! be theater, and a crashed agent must never padlock the
//! library). A lease lives `ttl_minutes` past its last renewal,
//! and any witnessed activity by the holder renews its cards:
//! the ledger is the heartbeat. Expiry is an absence computed
//! at read time, never a stored deadline or a fired event.

#![forbid(unsafe_code)]

use std::path::Path;

use rusqlite::params;

pub use rusqlite::Connection;

const MIGRATIONS: &[(i64, &str, &str)] = &[
  (1, "0001_init", include_str!("../migrations/0001_init.sql")),
  (
    2,
    "0002_sessions",
    include_str!("../migrations/0002_sessions.sql"),
  ),
];

#[derive(Debug, thiserror::Error)]
pub enum LeaseError {
  #[error("sqlite error: {0}")]
  Sqlite(#[from] rusqlite::Error),
  #[error("migration {0} failed: {1}")]
  Migration(i64, rusqlite::Error),
  #[error("no lease with id {0:?}")]
  IdNotFound(String),
  #[error("id fragment {0:?} matches more than one lease")]
  AmbiguousId(String),
  #[error("no active lease on {0:?} held by {1:?}")]
  NotHeld(String, String),
}

/// Who holds a card: the claimed name plus the minted session
/// (D-044). Two sessions of one agent are two holders.
#[derive(Debug, Clone, Copy)]
pub struct Holder<'a> {
  pub agent_id: &'a str,
  pub session_id: &'a str,
}

/// One card on the table.
#[derive(Debug, Clone)]
pub struct Lease {
  pub id: String,
  pub namespace: String,
  pub resource: String,
  pub agent_id: String,
  /// Librarian-minted per serve process (D-044): two sessions
  /// of the same agent name are different holders.
  pub session_id: String,
  pub note: Option<String>,
  pub taken_at: String,
  pub renewed_at: String,
  pub released_at: Option<String>,
}

pub fn open(path: &Path) -> Result<Connection, LeaseError> {
  let conn = Connection::open(path)?;
  configure(&conn)?;
  migrate(&conn)?;
  Ok(conn)
}

pub fn open_in_memory() -> Result<Connection, LeaseError> {
  let conn = Connection::open_in_memory()?;
  configure(&conn)?;
  migrate(&conn)?;
  Ok(conn)
}

fn configure(conn: &Connection) -> Result<(), LeaseError> {
  conn.pragma_update(None, "journal_mode", "wal")?;
  conn.pragma_update(None, "busy_timeout", 5000)?;
  conn.pragma_update(None, "synchronous", "normal")?;
  Ok(())
}

fn migrate(conn: &Connection) -> Result<(), LeaseError> {
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
      .map_err(|e| LeaseError::Migration(*version, e))?;
    conn.execute(
      "INSERT INTO schema_version (version, name, applied_at)
       VALUES (?1, ?2, ?3)",
      params![version, name, kumbarium_util::now_iso8601()],
    )?;
  }
  Ok(())
}

/// The short display form of an id.
pub fn short_id(id: &str) -> &str {
  id.get(id.len().saturating_sub(8)..).unwrap_or(id)
}

/// The freshness cutoff: a lease renewed at or after this
/// instant is alive. Computed, never stored (D-043).
fn cutoff(now_ms: i64, ttl_minutes: i64) -> String {
  kumbarium_util::format_iso8601_ms(now_ms - ttl_minutes * 60_000)
}

/// Take a card. Never refuses: the return carries every OTHER
/// active card on the same namespace+resource so the caller
/// can be told, loudly, whose table it is joining. Taking a
/// resource the same agent already actively holds renews that
/// card instead of stacking a duplicate.
pub fn take(
  conn: &Connection,
  namespace: &str,
  resource: &str,
  holder: Holder<'_>,
  note: Option<&str>,
  now_ms: i64,
  ttl_minutes: i64,
) -> Result<(Lease, Vec<Lease>), LeaseError> {
  let now = kumbarium_util::format_iso8601_ms(now_ms);
  let overlapping: Vec<Lease> =
    active_in(conn, Some(namespace), now_ms, ttl_minutes)?
      .into_iter()
      .filter(|l| l.resource == resource)
      .collect();
  // "Own" is (agent, session): the SAME session retaking
  // renews; another session of the same agent name is a
  // different holder and gets the warning, which is the
  // reading room's primary case (D-044).
  let own = overlapping.iter().find(|l| {
    l.agent_id == holder.agent_id && l.session_id == holder.session_id
  });
  if let Some(own) = own {
    conn.execute(
      "UPDATE leases SET renewed_at = ?1 WHERE id = ?2",
      params![now, own.id],
    )?;
    let renewed = get(conn, &own.id)?;
    let others = overlapping
      .into_iter()
      .filter(|l| {
        !(l.agent_id == holder.agent_id && l.session_id == holder.session_id)
      })
      .collect();
    return Ok((renewed, others));
  }
  let id = kumbarium_util::generate_id();
  conn.execute(
    "INSERT INTO leases
       (id, namespace, resource, agent_id, session_id, note,
        taken_at, renewed_at, created_at)
     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?7, ?7)",
    params![
      id,
      namespace,
      resource,
      holder.agent_id,
      holder.session_id,
      note,
      now
    ],
  )?;
  Ok((get(conn, &id)?, overlapping))
}

/// Release the CALLER'S OWN active card on a resource; someone
/// else's card is not yours to release (that is `break_lease`,
/// the human's verb).
pub fn release(
  conn: &Connection,
  namespace: &str,
  resource: &str,
  holder: Holder<'_>,
  now_ms: i64,
  ttl_minutes: i64,
) -> Result<Lease, LeaseError> {
  let held = active_in(conn, Some(namespace), now_ms, ttl_minutes)?
    .into_iter()
    .find(|l| {
      l.resource == resource
        && l.agent_id == holder.agent_id
        && l.session_id == holder.session_id
    })
    .ok_or_else(|| {
      LeaseError::NotHeld(
        format!("{namespace}/{resource}"),
        holder.agent_id.to_string(),
      )
    })?;
  conn.execute(
    "UPDATE leases SET released_at = ?1 WHERE id = ?2",
    params![kumbarium_util::format_iso8601_ms(now_ms), held.id],
  )?;
  get(conn, &held.id)
}

/// The human clears a stuck card by id, whoever holds it.
pub fn break_lease(
  conn: &Connection,
  id: &str,
  now_ms: i64,
) -> Result<Lease, LeaseError> {
  let full = resolve_id(conn, id)?;
  conn.execute(
    "UPDATE leases SET released_at = ?1
     WHERE id = ?2 AND released_at IS NULL",
    params![kumbarium_util::format_iso8601_ms(now_ms), full],
  )?;
  get(conn, &full)
}

/// Any witnessed activity by an agent renews every card it
/// holds: the ledger is the heartbeat (D-043). Returns how
/// many cards were touched.
pub fn renew_for_session(
  conn: &Connection,
  holder: Holder<'_>,
  now_ms: i64,
  ttl_minutes: i64,
) -> Result<usize, LeaseError> {
  // Per SESSION, not per agent name: an abandoned session's
  // card must go stale on schedule even while another session
  // under the same name stays busy (D-044).
  let n = conn.execute(
    "UPDATE leases SET renewed_at = ?1
     WHERE agent_id = ?2 AND session_id = ?3
       AND released_at IS NULL AND renewed_at >= ?4",
    params![
      kumbarium_util::format_iso8601_ms(now_ms),
      holder.agent_id,
      holder.session_id,
      cutoff(now_ms, ttl_minutes)
    ],
  )?;
  Ok(n)
}

/// The active cards (optionally one shelf's): unreleased and
/// renewed within the ttl, oldest first.
pub fn active_in(
  conn: &Connection,
  namespace: Option<&str>,
  now_ms: i64,
  ttl_minutes: i64,
) -> Result<Vec<Lease>, LeaseError> {
  let mut sql = String::from(
    "SELECT id, namespace, resource, agent_id, session_id, note,
            taken_at, renewed_at, released_at
     FROM leases
     WHERE released_at IS NULL AND renewed_at >= ?1",
  );
  let cutoff = cutoff(now_ms, ttl_minutes);
  let mut args: Vec<String> = vec![cutoff];
  if let Some(ns) = namespace {
    sql.push_str(" AND namespace = ?2");
    args.push(ns.to_string());
  }
  sql.push_str(" ORDER BY taken_at ASC");
  let mut stmt = conn.prepare(&sql)?;
  let rows = stmt
    .query_map(rusqlite::params_from_iter(args.iter()), row_to_lease)?
    .collect::<Result<Vec<_>, _>>()?;
  Ok(rows)
}

/// Expired-but-unreleased cards: the crashed-agent shape, the
/// janitor's finding (never served, never reaped here).
pub fn stale_in(
  conn: &Connection,
  now_ms: i64,
  ttl_minutes: i64,
) -> Result<Vec<Lease>, LeaseError> {
  let mut stmt = conn.prepare(
    "SELECT id, namespace, resource, agent_id, session_id, note,
            taken_at, renewed_at, released_at
     FROM leases
     WHERE released_at IS NULL AND renewed_at < ?1
     ORDER BY renewed_at ASC",
  )?;
  let rows = stmt
    .query_map([cutoff(now_ms, ttl_minutes)], row_to_lease)?
    .collect::<Result<Vec<_>, _>>()?;
  Ok(rows)
}

pub fn get(conn: &Connection, id: &str) -> Result<Lease, LeaseError> {
  let mut stmt = conn.prepare(
    "SELECT id, namespace, resource, agent_id, session_id, note,
            taken_at, renewed_at, released_at
     FROM leases WHERE id = ?1",
  )?;
  stmt.query_row([id], row_to_lease).map_err(|e| match e {
    rusqlite::Error::QueryReturnedNoRows => {
      LeaseError::IdNotFound(id.to_string())
    }
    other => other.into(),
  })
}

/// Resolve an id fragment, same grammar as every shelf.
pub fn resolve_id(conn: &Connection, id: &str) -> Result<String, LeaseError> {
  let mut stmt = conn
    .prepare("SELECT id FROM leases WHERE id LIKE ?1 ESCAPE '\\' LIMIT 2")?;
  let pattern = format!(
    "%{}%",
    id.replace('\\', "\\\\")
      .replace('%', "\\%")
      .replace('_', "\\_")
  );
  let matches: Vec<String> = stmt
    .query_map([pattern], |row| row.get(0))?
    .collect::<Result<Vec<_>, _>>()?;
  match matches.as_slice() {
    [] => Err(LeaseError::IdNotFound(id.to_string())),
    [one] => Ok(one.clone()),
    _ => Err(LeaseError::AmbiguousId(id.to_string())),
  }
}

fn row_to_lease(row: &rusqlite::Row<'_>) -> Result<Lease, rusqlite::Error> {
  Ok(Lease {
    id: row.get(0)?,
    namespace: row.get(1)?,
    resource: row.get(2)?,
    agent_id: row.get(3)?,
    session_id: row.get(4)?,
    note: row.get(5)?,
    taken_at: row.get(6)?,
    renewed_at: row.get(7)?,
    released_at: row.get(8)?,
  })
}

#[cfg(test)]
mod tests {
  use super::*;

  const TTL: i64 = 120;
  const HOUR: i64 = 3_600_000;

  fn holder<'a>(agent: &'a str, session: &'a str) -> Holder<'a> {
    Holder {
      agent_id: agent,
      session_id: session,
    }
  }

  #[test]
  fn take_overlap_release_round_trip() {
    let conn = open_in_memory().unwrap();
    let now = kumbarium_util::now_ms();
    let (a, others) = take(
      &conn,
      "project/x",
      "store",
      holder("agent-a", "s1"),
      None,
      now,
      TTL,
    )
    .unwrap();
    assert!(others.is_empty(), "empty room, no warning");
    // B joins the same table: granted, warned.
    let (b, others) = take(
      &conn,
      "project/x",
      "store",
      holder("agent-b", "s2"),
      None,
      now,
      TTL,
    )
    .unwrap();
    assert_eq!(others.len(), 1);
    assert_eq!(others[0].agent_id, "agent-a");
    assert_ne!(a.id, b.id, "both cards stand");
    // A releases its own; B's card survives.
    release(
      &conn,
      "project/x",
      "store",
      holder("agent-a", "s1"),
      now,
      TTL,
    )
    .unwrap();
    let active = active_in(&conn, Some("project/x"), now, TTL).unwrap();
    assert_eq!(active.len(), 1);
    assert_eq!(active[0].agent_id, "agent-b");
    // A cannot release what it no longer holds.
    assert!(
      release(
        &conn,
        "project/x",
        "store",
        holder("agent-a", "s1"),
        now,
        TTL
      )
      .is_err()
    );
  }

  #[test]
  fn retake_renews_instead_of_stacking() {
    let conn = open_in_memory().unwrap();
    let now = kumbarium_util::now_ms();
    let (a1, _) = take(
      &conn,
      "project/x",
      "store",
      holder("agent-a", "s1"),
      None,
      now,
      TTL,
    )
    .unwrap();
    let (a2, _) = take(
      &conn,
      "project/x",
      "store",
      holder("agent-a", "s1"),
      None,
      now + HOUR,
      TTL,
    )
    .unwrap();
    assert_eq!(a1.id, a2.id, "same card, renewed");
    assert!(a2.renewed_at > a1.renewed_at);
    assert_eq!(
      active_in(&conn, Some("project/x"), now + HOUR, TTL)
        .unwrap()
        .len(),
      1
    );
  }

  #[test]
  fn activity_renews_and_ttl_expires() {
    let conn = open_in_memory().unwrap();
    let now = kumbarium_util::now_ms();
    take(
      &conn,
      "project/x",
      "store",
      holder("agent-a", "s1"),
      None,
      now,
      TTL,
    )
    .unwrap();
    // 90 minutes later, activity renews the card.
    let later = now + 90 * 60_000;
    assert_eq!(
      renew_for_session(&conn, holder("agent-a", "s1"), later, TTL).unwrap(),
      1
    );
    // 3 hours after THAT with no activity: expired, not served.
    let stale_time = later + 3 * HOUR;
    assert!(
      active_in(&conn, Some("project/x"), stale_time, TTL)
        .unwrap()
        .is_empty()
    );
    let stale = stale_in(&conn, stale_time, TTL).unwrap();
    assert_eq!(stale.len(), 1);
    assert_eq!(stale[0].agent_id, "agent-a");
    // A dead card does not spring back from late activity.
    assert_eq!(
      renew_for_session(&conn, holder("agent-a", "s1"), stale_time, TTL)
        .unwrap(),
      0
    );
  }

  #[test]
  fn the_human_breaks_anyones_card() {
    let conn = open_in_memory().unwrap();
    let now = kumbarium_util::now_ms();
    let (a, _) = take(
      &conn,
      "project/x",
      "store",
      holder("agent-a", "s1"),
      None,
      now,
      TTL,
    )
    .unwrap();
    let broken = break_lease(&conn, short_id(&a.id), now).unwrap();
    assert!(broken.released_at.is_some());
    assert!(active_in(&conn, None, now, TTL).unwrap().is_empty());
  }

  #[test]
  fn same_agent_different_sessions_warn_each_other() {
    let conn = open_in_memory().unwrap();
    let now = kumbarium_util::now_ms();
    let (a, _) = take(
      &conn,
      "project/x",
      "store",
      holder("claude-code", "s1"),
      None,
      now,
      TTL,
    )
    .unwrap();
    // The SAME agent name from a DIFFERENT session is a
    // different holder: warned, not silently renewed (D-044).
    let (b, others) = take(
      &conn,
      "project/x",
      "store",
      holder("claude-code", "s2"),
      None,
      now,
      TTL,
    )
    .unwrap();
    assert_ne!(a.id, b.id, "two cards, not one renewed");
    assert_eq!(others.len(), 1);
    assert_eq!(others[0].session_id, "s1");
    // Session 2's activity renews only its own card.
    let later = now + HOUR;
    assert_eq!(
      renew_for_session(&conn, holder("claude-code", "s2"), later, TTL)
        .unwrap(),
      1
    );
    // Session 1 goes idle past ttl while s2 stays busy: s1's
    // card is stale, s2's is not (no zombie via shared name).
    let much_later = now + 3 * HOUR;
    renew_for_session(&conn, holder("claude-code", "s2"), much_later, TTL)
      .unwrap();
    let stale = stale_in(&conn, much_later, TTL).unwrap();
    assert_eq!(stale.len(), 1);
    assert_eq!(stale[0].session_id, "s1");
    // Session 2 cannot release session 1's card.
    assert!(
      release(
        &conn,
        "project/x",
        "store",
        holder("claude-code", "s2"),
        much_later,
        TTL
      )
      .is_ok(),
      "releases its OWN card"
    );
    assert!(
      release(
        &conn,
        "project/x",
        "store",
        holder("claude-code", "s2"),
        much_later,
        TTL
      )
      .is_err(),
      "nothing left that s2 holds"
    );
  }
}
