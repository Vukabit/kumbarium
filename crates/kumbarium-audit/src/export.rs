//! Reading the log back, and the deterministic meeting-minutes
//! rendering: a pure template over events in time order. No LLM
//! anywhere; the same events always render the same minutes.

use rusqlite::Connection;

use crate::AuditError;

/// An event as stored (kind stays a string here: the reader
/// must render rows written by FUTURE schema versions too).
#[derive(Debug, Clone)]
pub struct StoredEvent {
  pub id: String,
  pub at: String,
  pub agent_id: String,
  pub kind: String,
  pub scope: String,
  pub detail: String,
}

/// The most recent `n` events, newest first.
pub fn tail(
  conn: &Connection,
  n: usize,
) -> Result<Vec<StoredEvent>, AuditError> {
  query(
    conn,
    "SELECT id, at, agent_id, kind, scope, detail FROM events
     ORDER BY at DESC, id DESC LIMIT ?1",
    Some(n),
  )
}

/// Every event, oldest first (the minutes ordering).
pub fn events_asc(conn: &Connection) -> Result<Vec<StoredEvent>, AuditError> {
  query(
    conn,
    "SELECT id, at, agent_id, kind, scope, detail FROM events
     ORDER BY at ASC, id ASC",
    None,
  )
}

fn query(
  conn: &Connection,
  sql: &str,
  limit: Option<usize>,
) -> Result<Vec<StoredEvent>, AuditError> {
  let mut stmt = conn.prepare(sql)?;
  let map = |row: &rusqlite::Row<'_>| {
    Ok(StoredEvent {
      id: row.get(0)?,
      at: row.get(1)?,
      agent_id: row.get(2)?,
      kind: row.get(3)?,
      scope: row.get(4)?,
      detail: row.get(5)?,
    })
  };
  let rows = match limit {
    Some(n) => stmt.query_map([n as i64], map)?,
    None => stmt.query_map([], map)?,
  }
  .collect::<Result<Vec<_>, _>>()?;
  Ok(rows)
}

/// Render events (oldest first) as meeting-minutes markdown:
/// one day per section, one line per event. Pure and
/// deterministic; identical input always yields identical
/// output.
pub fn render_minutes(events: &[StoredEvent]) -> String {
  let mut out = String::from("# Kumbarium minutes\n");
  if events.is_empty() {
    out.push_str("\nNo events recorded.\n");
    return out;
  }
  let mut day = "";
  for e in events {
    let (d, t) = split_at(&e.at);
    if d != day {
      day = d;
      out.push_str(&format!("\n## {day}\n\n"));
    }
    let scope = if e.scope.is_empty() {
      String::new()
    } else {
      format!(" in {}", e.scope)
    };
    out.push_str(&format!(
      "- {t} {} by {}{}: {}\n",
      e.kind, e.agent_id, scope, e.detail
    ));
  }
  out
}

/// (day, hh:mm:ss) halves of a strict ISO timestamp; total for
/// any input (malformed rows render verbatim, never panic).
fn split_at(at: &str) -> (&str, &str) {
  match (at.get(..10), at.get(11..19)) {
    (Some(d), Some(t)) => (d, t),
    _ => (at, ""),
  }
}

#[cfg(test)]
mod tests {
  use super::*;
  use crate::{Event, EventKind, append, open_in_memory};

  fn seeded() -> Connection {
    let conn = open_in_memory().unwrap();
    for kind in [EventKind::Remember, EventKind::Recall] {
      append(
        &conn,
        &Event {
          agent_id: "test-agent".into(),
          kind,
          scope: "project/demo".into(),
          detail: serde_json::json!({ "n": 1 }),
        },
      )
      .unwrap();
    }
    conn
  }

  #[test]
  fn tail_returns_newest_first_and_respects_limit() {
    let conn = seeded();
    let events = tail(&conn, 1).unwrap();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].kind, "recall");
    assert_eq!(tail(&conn, 10).unwrap().len(), 2);
  }

  #[test]
  fn minutes_render_deterministically_grouped_by_day() {
    let conn = seeded();
    let events = events_asc(&conn).unwrap();
    let a = render_minutes(&events);
    let b = render_minutes(&events);
    assert_eq!(a, b, "same events, same minutes");
    assert!(a.starts_with("# Kumbarium minutes\n"));
    assert!(a.contains("## 20"), "day section header");
    assert!(a.contains("remember by test-agent in project/demo"));
    let remember_pos = a.find("remember by").unwrap();
    let recall_pos = a.find("recall by").unwrap();
    assert!(remember_pos < recall_pos, "oldest first");
  }

  #[test]
  fn empty_log_renders_a_stub() {
    let conn = open_in_memory().unwrap();
    let text = render_minutes(&events_asc(&conn).unwrap());
    assert!(text.contains("No events recorded."));
  }
}
