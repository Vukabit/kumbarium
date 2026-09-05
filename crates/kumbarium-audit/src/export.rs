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
  pub session_id: String,
  pub kind: String,
  pub scope: String,
  pub detail: String,
}

/// The most recent `n` events, newest first; optionally only
/// one scope's.
pub fn tail(
  conn: &Connection,
  n: usize,
  scope: Option<&str>,
) -> Result<Vec<StoredEvent>, AuditError> {
  let mut stmt = match scope {
    Some(_) => conn.prepare(
      "SELECT id, at, agent_id, session_id, kind, scope, detail
       FROM events
       WHERE scope = ?1 ORDER BY at DESC, id DESC LIMIT ?2",
    )?,
    None => conn.prepare(
      "SELECT id, at, agent_id, session_id, kind, scope, detail
       FROM events
       ORDER BY at DESC, id DESC LIMIT ?1",
    )?,
  };
  let map = |row: &rusqlite::Row<'_>| {
    Ok(StoredEvent {
      id: row.get(0)?,
      at: row.get(1)?,
      agent_id: row.get(2)?,
      session_id: row.get(3)?,
      kind: row.get(4)?,
      scope: row.get(5)?,
      detail: row.get(6)?,
    })
  };
  let rows = match scope {
    Some(sc) => stmt.query_map(rusqlite::params![sc, n as i64], map)?,
    None => stmt.query_map([n as i64], map)?,
  }
  .collect::<Result<Vec<_>, _>>()?;
  Ok(rows)
}

/// (event count, newest event timestamp) for `kum status`.
pub fn summary(conn: &Connection) -> Result<(i64, Option<String>), AuditError> {
  let count =
    conn.query_row("SELECT count(*) FROM events", [], |row| row.get(0))?;
  let latest = conn
    .query_row(
      "SELECT at FROM events ORDER BY at DESC LIMIT 1",
      [],
      |row| row.get(0),
    )
    .map(Some)
    .or_else(|e| match e {
      rusqlite::Error::QueryReturnedNoRows => Ok(None),
      other => Err(other),
    })?;
  Ok((count, latest))
}

/// Every event, oldest first (the minutes ordering).
pub fn events_asc(conn: &Connection) -> Result<Vec<StoredEvent>, AuditError> {
  query(
    conn,
    "SELECT id, at, agent_id, session_id, kind, scope, detail
       FROM events
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
      session_id: row.get(3)?,
      kind: row.get(4)?,
      scope: row.get(5)?,
      detail: row.get(6)?,
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
/// one day per section, one line per event. Pure given its
/// inputs: `localize` converts a stored UTC ISO timestamp into
/// the display form "YYYY-MM-DD HH:MM:SS" (the CLI passes local
/// time; tests pass a fixed converter, keeping the renderer
/// deterministic).
pub fn render_minutes(
  events: &[StoredEvent],
  localize: &dyn Fn(&str) -> String,
  times_note: &str,
) -> String {
  let mut out = format!("# Kumbarium minutes\n\n{times_note}\n");
  if events.is_empty() {
    out.push_str("\nNo events recorded.\n");
    return out;
  }
  let mut day = String::new();
  for e in events {
    let local = localize(&e.at);
    let (d, t) = split_at(&local);
    if d != day {
      if !day.is_empty() {
        out.push_str("```\n");
      }
      day = d.to_string();
      out.push_str(&format!(
        "\n## {day}\n\n```\ntime      kind            \
agent                scope                detail\n"
      ));
    }
    out.push_str(&format!(
      "{t:<8}  {:<15} {:<20} {:<20} {}\n",
      e.kind,
      e.agent_id,
      e.scope,
      describe_event(&e.kind, &e.detail)
    ));
  }
  out.push_str("```\n");
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
      "approve" => Some(format!(
        "approved {} (submitted by {})",
        short(s("id")?),
        s("submitter")?
      )),
      "reject" => {
        let mut line = format!(
          "rejected {} (submitted by {})",
          short(s("id")?),
          s("submitter")?
        );
        if let Some(reason) = s("reason") {
          line.push_str(&format!(" {reason:?}"));
        }
        Some(line)
      }
      "confirm" => Some(format!("confirmed {}", short(s("id")?))),
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
      "handoff_write" => Some(format!("left a briefing {}", short(s("id")?))),
      "handoff_drop" => Some(format!(
        "dropped the standing briefing {} (kept on record, no \
         longer served)",
        short(s("id")?)
      )),
      "get" => Some(format!("fetched {} in full by id", short(s("id")?))),
      "task_list" => Some(format!(
        "surveyed the open docket ({} matters served)",
        n("returned").unwrap_or(0)
      )),
      "secret_set" => Some(format!("stocked secret {:?}", s("name")?)),
      "secret_read" => {
        // found:false = the name was not on the shelf; nothing
        // moved, and the line must never read as a disclosure.
        if v.get("found").and_then(|x| x.as_bool()) == Some(false) {
          return Some(format!(
            "sought secret {:?} (not on the shelf; nothing moved)",
            s("name")?
          ));
        }
        let granted =
          v.get("granted").and_then(|x| x.as_bool()).unwrap_or(false);
        Some(if granted {
          format!("read secret {:?}", s("name")?)
        } else {
          format!("REFUSED secret {:?} (no grant)", s("name")?)
        })
      }
      "secret_grant" => {
        Some(format!("granted {:?} to {}", s("name")?, s("grantee")?))
      }
      "secret_revoke" => {
        Some(format!("revoked {:?} from {}", s("name")?, s("grantee")?))
      }
      "secret_shred" => Some(format!("shredded secret {:?}", s("name")?)),
      "secret_copy" => Some(format!(
        "concealed-copied secret {:?} (auto-clear)",
        s("name")?
      )),
      "secret_exec" => Some(format!(
        "ran {} with secret {:?} injected (output redacted)",
        s("command")?,
        s("name")?
      )),
      "lease_take" => {
        let mut line = format!("took a lease on {:?}", s("resource")?);
        if let Some(n) = v.get("overlapping").and_then(|x| x.as_u64())
          && n > 0
        {
          line.push_str(&format!(" (WARNED: {n} other holder(s))"));
        }
        Some(line)
      }
      "lease_release" => {
        Some(format!("released the lease on {:?}", s("resource")?))
      }
      "lease_break" => Some(format!(
        "broke {}'s lease on {:?}",
        s("holder")?,
        s("resource")?
      )),
      "secret_leakscan" => {
        let hits = v.get("hits").and_then(|x| x.as_i64()).unwrap_or(0);
        let scanned = v.get("scanned").and_then(|x| x.as_i64()).unwrap_or(0);
        Some(if hits == 0 {
          format!("leak scan clean ({scanned} secrets swept)")
        } else {
          format!(
            "leak scan found {hits} EXPOSURE(S) across \
             {scanned} secrets"
          )
        })
      }
      "task_file" => {
        let mut line =
          format!("filed {} task {}", s("severity")?, short(s("id")?));
        if let Some(goal) = s("goal") {
          line.push_str(&format!(" (goal {goal})"));
        }
        Some(line)
      }
      "task_update" => {
        let mut line = format!(
          "regraded task {} to {}",
          short(s("old_id")?),
          short(s("new_id")?)
        );
        if let Some(goal) = s("goal") {
          line.push_str(&format!(" (goal {goal})"));
        }
        if let Some(sev) = s("severity") {
          line.push_str(&format!(" ({sev})"));
        }
        Some(line)
      }
      "task_done" => Some(format!("completed task {}", short(s("id")?))),
      "task_drop" => {
        let mut line = format!("dropped task {}", short(s("id")?));
        if let Some(note) = s("note") {
          line.push_str(&format!(" {note:?}"));
        }
        Some(line)
      }
      "janitor" => Some(format!(
        "janitor adjusted {}, {} dormant flagged",
        plural(n("changed")? as usize, "confidence", "confidences"),
        n("dormant")?
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

/// (day, hh:mm:ss) halves of a "YYYY-MM-DD HH:MM:SS"-shaped
/// display timestamp; total for any input (malformed rows
/// render verbatim, never panic).
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
          session_id: "test-session".into(),
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
    let events = tail(&conn, 1, None).unwrap();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].kind, "recall");
    assert_eq!(tail(&conn, 10, None).unwrap().len(), 2);
    assert_eq!(tail(&conn, 10, Some("project/demo")).unwrap().len(), 2);
    assert_eq!(tail(&conn, 10, Some("global")).unwrap().len(), 0);
  }

  fn utc(at: &str) -> String {
    let day = at.get(..10).unwrap_or(at);
    let time = at.get(11..19).unwrap_or("");
    format!("{day} {time}")
  }

  #[test]
  fn minutes_render_deterministically_grouped_by_day() {
    let conn = seeded();
    let events = events_asc(&conn).unwrap();
    let a = render_minutes(&events, &utc, "All times UTC.");
    let b = render_minutes(&events, &utc, "All times UTC.");
    assert_eq!(a, b, "same events, same minutes");
    assert!(a.starts_with("# Kumbarium minutes\n"));
    assert!(a.contains("## 20"), "day section header");
    assert!(a.contains("time      kind"), "tabular header per day");
    assert!(a.contains("remember        test-agent"));
    assert!(a.ends_with("```\n"), "day table fence closed");
    let remember_pos = a.find("remember        test-agent").unwrap();
    let recall_pos = a.find("recall          test-agent").unwrap();
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
    let text =
      render_minutes(&events_asc(&conn).unwrap(), &utc, "All times UTC.");
    assert!(text.contains("No events recorded."));
  }
}
