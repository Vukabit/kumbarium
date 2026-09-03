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
  let mut out = String::from("# Kumbarium minutes\n\nAll times UTC.\n");
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
      e.kind,
      e.agent_id,
      scope,
      describe_event(&e.kind, &e.detail)
    ));
  }
  out
}

/// One deterministic prose line for an event's detail payload;
/// unknown kinds or shapes fall back to the raw JSON, so rows
/// written by future schema versions still render.
pub fn describe_event(kind: &str, detail: &str) -> String {
  let Ok(v) = serde_json::from_str::<serde_json::Value>(detail) else {
    return detail.to_string();
  };
  let s = |key: &str| v.get(key).and_then(|x| x.as_str());
  let n = |key: &str| v.get(key).and_then(|x| x.as_u64());
  let described = (|| -> Option<String> {
    match kind {
      "recall" => {
        let query = s("query")?;
        let ids: Vec<&str> = v
          .get("returned")?
          .as_array()?
          .iter()
          .filter_map(|x| x.as_str())
          .map(short)
          .collect();
        if ids.is_empty() {
          Some(format!("recalled nothing for {query:?}"))
        } else {
          Some(format!(
            "recalled {} for {query:?}: {}",
            plural(ids.len(), "memory", "memories"),
            ids.join(", ")
          ))
        }
      }
      "remember" => {
        let mut line = format!("remembered {}", short(s("id")?));
        if let Some(kind) = s("kind") {
          line.push_str(&format!(" ({kind}"));
          if let Some(parts) = n("parts").filter(|p| *p > 1) {
            line.push_str(&format!(", {parts} parts"));
          }
          if let Some(links) = n("links").filter(|l| *l > 0) {
            line.push_str(&format!(", {links} links"));
          }
          line.push(')');
        }
        Some(line)
      }
      "supersede" => {
        let mut line = format!(
          "superseded {} with {}",
          short(s("old_id")?),
          short(s("new_id")?)
        );
        if let Some(to) = s("revert_to") {
          line.push_str(&format!(" (revert to {})", short(to)));
        }
        if let Some(note) = s("note") {
          line.push_str(&format!(" {note:?}"));
        }
        if let Some(parts) = n("parts").filter(|p| *p > 1) {
          line.push_str(&format!(" ({parts} parts)"));
        }
        Some(line)
      }
      "forget" => Some(format!("forgot {}", short(s("id")?))),
      "retire" => Some(format!("retired {}", short(s("id")?))),
      "unretire" => Some(format!("restored {}", short(s("id")?))),
      "link" => Some(format!(
        "linked {} {} {}",
        short(s("from_id")?),
        s("rel")?,
        short(s("to_id")?)
      )),
      "import" => Some(format!(
        "imported {} of {} planned memories, {} edges",
        n("imported")?,
        n("planned")?,
        n("edges")?
      )),
      _ => None,
    }
  })();
  described.unwrap_or_else(|| detail.to_string())
}

/// Last 8 hex chars: the display short form. Kept local; this
/// crate must not depend on the store.
fn short(id: &str) -> &str {
  id.get(id.len().saturating_sub(8)..).unwrap_or(id)
}

fn plural(n: usize, one: &str, many: &str) -> String {
  if n == 1 {
    format!("1 {one}")
  } else {
    format!("{n} {many}")
  }
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
  fn described_events_read_as_prose() {
    let id_a = "01a06550-ec55-7470-88d9-e009dfeb4d7c";
    let id_b = "01a0652e-7e50-7661-a7f6-94d4d0b96f52";
    let recall = format!(
      "{{\"query\":\"commit style\",\"returned\":[\"{id_a}\",\
       \"{id_b}\"]}}"
    );
    assert_eq!(
      describe_event("recall", &recall),
      "recalled 2 memories for \"commit style\": dfeb4d7c, \
       d0b96f52"
    );
    assert_eq!(
      describe_event("recall", "{\"query\":\"x\",\"returned\":[]}"),
      "recalled nothing for \"x\""
    );
    let sup = format!(
      "{{\"old_id\":\"{id_a}\",\"new_id\":\"{id_b}\",\
       \"parts\":4}}"
    );
    assert_eq!(
      describe_event("supersede", &sup),
      "superseded dfeb4d7c with d0b96f52 (4 parts)"
    );
    assert_eq!(
      describe_event("import", "{\"planned\":5,\"imported\":5,\"edges\":10}"),
      "imported 5 of 5 planned memories, 10 edges"
    );
    // Unknown kind or shape: raw JSON survives untouched.
    assert_eq!(describe_event("eval_run", "{\"n\":1}"), "{\"n\":1}");
    assert_eq!(
      describe_event("recall", "{\"nope\":true}"),
      "{\"nope\":true}"
    );
  }

  #[test]
  fn empty_log_renders_a_stub() {
    let conn = open_in_memory().unwrap();
    let text = render_minutes(&events_asc(&conn).unwrap());
    assert!(text.contains("No events recorded."));
  }
}
