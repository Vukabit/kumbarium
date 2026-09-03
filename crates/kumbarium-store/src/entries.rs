//! Entry CRUD and FTS5 recall: the mechanical layer. Namespace
//! VALIDATION and chain computation live in kumbarium-librarian;
//! this module trusts its caller on both and enforces only what
//! the storage itself must (registered namespaces, existing
//! entries, non-empty content).

use rusqlite::{Connection, params, params_from_iter};

use crate::StoreError;

/// Entry kind; drives type-aware decay. Mirrors the CHECK
/// constraint in migration 0001.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
  Preference,
  ProjectState,
  Decision,
  Reference,
}

impl Kind {
  pub fn as_str(self) -> &'static str {
    match self {
      Kind::Preference => "preference",
      Kind::ProjectState => "project_state",
      Kind::Decision => "decision",
      Kind::Reference => "reference",
    }
  }

  pub fn parse(s: &str) -> Option<Kind> {
    match s {
      "preference" => Some(Kind::Preference),
      "project_state" => Some(Kind::ProjectState),
      "decision" => Some(Kind::Decision),
      "reference" => Some(Kind::Reference),
      _ => None,
    }
  }
}

/// What a writer supplies; the store mints id, timestamps, and
/// the starting confidence.
#[derive(Debug, Clone)]
pub struct NewEntry {
  pub namespace: String,
  pub kind: Kind,
  pub content: String,
  pub agent_id: String,
  pub source: String,
  pub tags: Vec<String>,
}

/// A stored entry, tags included.
#[derive(Debug, Clone)]
pub struct Entry {
  pub id: String,
  pub namespace: String,
  pub kind: Kind,
  pub content: String,
  pub agent_id: String,
  pub source: String,
  pub confidence: f64,
  pub superseded_by: Option<String>,
  pub created_at: String,
  pub updated_at: String,
  pub last_accessed_at: Option<String>,
  pub last_confirmed_at: Option<String>,
  pub tags: Vec<String>,
}

/// One recall result. `bm25` is the raw FTS5 rank (LOWER is
/// better); the librarian turns it into a 0..=1 relevance score.
#[derive(Debug, Clone)]
pub struct Hit {
  pub entry: Entry,
  pub bm25: f64,
}

/// Register a namespace. Paths are validated upstream (librarian);
/// here only uniqueness is enforced.
pub fn register_namespace(
  conn: &Connection,
  path: &str,
  description: &str,
) -> Result<i64, StoreError> {
  let n = conn.execute(
    "INSERT OR IGNORE INTO namespaces (path, description, created_at)
     VALUES (?1, ?2, ?3)",
    params![path, description, kumbarium_util::now_iso8601()],
  )?;
  if n == 0 {
    return Err(StoreError::NamespaceExists(path.to_string()));
  }
  Ok(conn.last_insert_rowid())
}

/// The namespace's rowid, or None when unregistered.
pub fn namespace_id(
  conn: &Connection,
  path: &str,
) -> Result<Option<i64>, StoreError> {
  let id = conn
    .query_row("SELECT id FROM namespaces WHERE path = ?1", [path], |row| {
      row.get(0)
    })
    .map(Some)
    .or_else(|e| match e {
      rusqlite::Error::QueryReturnedNoRows => Ok(None),
      other => Err(other),
    })?;
  Ok(id)
}

/// Store a new entry. Fails on an unregistered namespace (no
/// auto-create, ever) or empty content.
pub fn remember(
  conn: &mut Connection,
  new: &NewEntry,
) -> Result<Entry, StoreError> {
  let tx = conn.transaction()?;
  let entry = insert_entry(&tx, new)?;
  tx.commit()?;
  Ok(entry)
}

/// Fetch one entry by id, tags included.
pub fn get(conn: &Connection, id: &str) -> Result<Entry, StoreError> {
  let mut stmt = conn.prepare(
    "SELECT e.id, ns.path, e.kind, e.content, e.agent_id, e.source,
            e.confidence, e.superseded_by, e.created_at,
            e.updated_at, e.last_accessed_at, e.last_confirmed_at
     FROM entries e JOIN namespaces ns ON ns.id = e.namespace_id
     WHERE e.id = ?1",
  )?;
  let entry = stmt.query_row([id], row_to_entry).map_err(|e| match e {
    rusqlite::Error::QueryReturnedNoRows => {
      StoreError::EntryNotFound(id.to_string())
    }
    other => other.into(),
  })?;
  with_tags(conn, entry)
}

/// FTS recall over `namespaces` (the caller passes the already
/// computed chain). Superseded entries never surface. Matching
/// is OR across sanitized tokens: bm25 still ranks entries
/// hitting more terms first, but one missing word cannot blank a
/// result. Returned entries get `last_accessed_at` touched.
pub fn recall(
  conn: &Connection,
  query: &str,
  namespaces: &[String],
  limit: usize,
) -> Result<Vec<Hit>, StoreError> {
  let Some(fts) = fts_query(query) else {
    return Ok(Vec::new());
  };
  if namespaces.is_empty() {
    return Ok(Vec::new());
  }
  let ns_marks = (0..namespaces.len())
    .map(|i| format!("?{}", i + 3))
    .collect::<Vec<_>>()
    .join(", ");
  let sql = format!(
    "SELECT e.id, ns.path, e.kind, e.content, e.agent_id, e.source,
            e.confidence, e.superseded_by, e.created_at,
            e.updated_at, e.last_accessed_at, e.last_confirmed_at,
            bm25(entries_fts) AS rank
     FROM entries_fts
     JOIN entries e ON e.rowid = entries_fts.rowid
     JOIN namespaces ns ON ns.id = e.namespace_id
     WHERE entries_fts MATCH ?1
       AND e.superseded_by IS NULL
       AND ns.path IN ({ns_marks})
     ORDER BY rank
     LIMIT ?2"
  );
  let mut args: Vec<String> = vec![fts, (limit as i64).to_string()];
  args.extend(namespaces.iter().cloned());
  let mut stmt = conn.prepare(&sql)?;
  let rows = stmt.query_map(params_from_iter(args.iter()), |row| {
    Ok((row_to_entry(row)?, row.get::<_, f64>(12)?))
  })?;
  let mut hits = Vec::new();
  for row in rows {
    let (entry, bm25) = row?;
    let entry = with_tags(conn, entry)?;
    hits.push(Hit { entry, bm25 });
  }
  touch_accessed(conn, &hits)?;
  Ok(hits)
}

/// Store `new` as the replacement for `old_id`: the old entry is
/// chained forward via `superseded_by`, never deleted. Fails when
/// the old entry is missing or already superseded (chains stay
/// linear; supersede the chain head instead).
pub fn supersede(
  conn: &mut Connection,
  old_id: &str,
  new: &NewEntry,
) -> Result<Entry, StoreError> {
  let tx = conn.transaction()?;
  let prior: Option<Option<String>> = tx
    .query_row(
      "SELECT superseded_by FROM entries WHERE id = ?1",
      [old_id],
      |row| row.get(0),
    )
    .map(Some)
    .or_else(|e| match e {
      rusqlite::Error::QueryReturnedNoRows => Ok(None),
      other => Err(other),
    })?;
  match prior {
    None => {
      return Err(StoreError::EntryNotFound(old_id.to_string()));
    }
    Some(Some(_)) => {
      return Err(StoreError::AlreadySuperseded(old_id.to_string()));
    }
    Some(None) => {}
  }
  let entry = insert_entry(&tx, new)?;
  tx.execute(
    "UPDATE entries SET superseded_by = ?1, updated_at = ?2
     WHERE id = ?3",
    params![entry.id, kumbarium_util::now_iso8601(), old_id],
  )?;
  tx.commit()?;
  Ok(entry)
}

/// Hard-delete an entry (the explicit-removal escape hatch for
/// wrong or sensitive content; routine correction is `supersede`).
/// Any chain pointing at it is unlinked first.
pub fn forget(conn: &mut Connection, id: &str) -> Result<(), StoreError> {
  let tx = conn.transaction()?;
  tx.execute(
    "UPDATE entries SET superseded_by = NULL WHERE superseded_by = ?1",
    [id],
  )?;
  tx.execute("DELETE FROM entry_tags WHERE entry_id = ?1", [id])?;
  let n = tx.execute("DELETE FROM entries WHERE id = ?1", [id])?;
  if n == 0 {
    return Err(StoreError::EntryNotFound(id.to_string()));
  }
  tx.commit()?;
  Ok(())
}

/// Mark an entry as re-confirmed now (it was re-asserted or acted
/// on successfully); feeds the staleness signal.
pub fn confirm(conn: &Connection, id: &str) -> Result<(), StoreError> {
  let n = conn.execute(
    "UPDATE entries SET last_confirmed_at = ?1 WHERE id = ?2",
    params![kumbarium_util::now_iso8601(), id],
  )?;
  if n == 0 {
    return Err(StoreError::EntryNotFound(id.to_string()));
  }
  Ok(())
}

fn insert_entry(
  conn: &Connection,
  new: &NewEntry,
) -> Result<Entry, StoreError> {
  if new.content.trim().is_empty() {
    return Err(StoreError::EmptyContent);
  }
  let ns = namespace_id(conn, &new.namespace)?
    .ok_or_else(|| StoreError::NamespaceNotRegistered(new.namespace.clone()))?;
  let id = kumbarium_util::generate_id();
  let now = kumbarium_util::now_iso8601();
  conn.execute(
    "INSERT INTO entries
       (id, namespace_id, kind, content, agent_id, source,
        created_at, updated_at)
     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?7)",
    params![
      id,
      ns,
      new.kind.as_str(),
      new.content,
      new.agent_id,
      new.source,
      now,
    ],
  )?;
  for tag in &new.tags {
    conn.execute(
      "INSERT OR IGNORE INTO entry_tags (entry_id, tag)
       VALUES (?1, ?2)",
      params![id, tag],
    )?;
  }
  get(conn, &id)
}

fn row_to_entry(row: &rusqlite::Row<'_>) -> Result<Entry, rusqlite::Error> {
  let kind_raw: String = row.get(2)?;
  let kind = Kind::parse(&kind_raw).ok_or_else(|| {
    rusqlite::Error::FromSqlConversionFailure(
      2,
      rusqlite::types::Type::Text,
      format!("unknown kind {kind_raw:?}").into(),
    )
  })?;
  Ok(Entry {
    id: row.get(0)?,
    namespace: row.get(1)?,
    kind,
    content: row.get(3)?,
    agent_id: row.get(4)?,
    source: row.get(5)?,
    confidence: row.get(6)?,
    superseded_by: row.get(7)?,
    created_at: row.get(8)?,
    updated_at: row.get(9)?,
    last_accessed_at: row.get(10)?,
    last_confirmed_at: row.get(11)?,
    tags: Vec::new(),
  })
}

fn with_tags(conn: &Connection, mut entry: Entry) -> Result<Entry, StoreError> {
  let mut stmt = conn
    .prepare("SELECT tag FROM entry_tags WHERE entry_id = ?1 ORDER BY tag")?;
  let tags = stmt
    .query_map([&entry.id], |row| row.get(0))?
    .collect::<Result<Vec<String>, _>>()?;
  entry.tags = tags;
  Ok(entry)
}

fn touch_accessed(conn: &Connection, hits: &[Hit]) -> Result<(), StoreError> {
  let now = kumbarium_util::now_iso8601();
  for hit in hits {
    conn.execute(
      "UPDATE entries SET last_accessed_at = ?1 WHERE id = ?2",
      params![now, hit.entry.id],
    )?;
  }
  Ok(())
}

/// Sanitize a raw query into FTS5 MATCH syntax: each whitespace
/// token becomes a quoted phrase (embedded quotes doubled), all
/// joined with OR. Quoting makes ANY input valid MATCH syntax, so
/// agent-supplied text can never raise an FTS parse error. None
/// when no tokens survive.
fn fts_query(raw: &str) -> Option<String> {
  let tokens: Vec<String> = raw
    .split_whitespace()
    .map(|t| format!("\"{}\"", t.replace('"', "\"\"")))
    .collect();
  if tokens.is_empty() {
    None
  } else {
    Some(tokens.join(" OR "))
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  fn store() -> Connection {
    let conn = crate::open_in_memory().unwrap();
    register_namespace(&conn, "project/demo-app", "demo").unwrap();
    register_namespace(&conn, "project/other-app", "other").unwrap();
    conn
  }

  fn entry_in(ns: &str, content: &str) -> NewEntry {
    NewEntry {
      namespace: ns.into(),
      kind: Kind::Decision,
      content: content.into(),
      agent_id: "test-agent".into(),
      source: "unit-test".into(),
      tags: vec!["alpha".into(), "beta".into()],
    }
  }

  const CHAIN: &[&str] = &["project/demo-app", "project", "global"];

  fn chain() -> Vec<String> {
    CHAIN.iter().map(|s| s.to_string()).collect()
  }

  #[test]
  fn remember_get_round_trips_with_tags() {
    let mut conn = store();
    let e = remember(
      &mut conn,
      &entry_in("project/demo-app", "demo-app uses SQLite, WAL mode"),
    )
    .unwrap();
    assert!(kumbarium_util::is_valid_id(&e.id));
    let back = get(&conn, &e.id).unwrap();
    assert_eq!(back.content, "demo-app uses SQLite, WAL mode");
    assert_eq!(back.namespace, "project/demo-app");
    assert_eq!(back.kind, Kind::Decision);
    assert_eq!(back.tags, ["alpha", "beta"]);
    assert_eq!(back.superseded_by, None);
  }

  #[test]
  fn remember_refuses_unregistered_namespace() {
    let mut conn = store();
    let err = remember(&mut conn, &entry_in("project/nope", "x"));
    assert!(matches!(err, Err(StoreError::NamespaceNotRegistered(_))));
  }

  #[test]
  fn remember_refuses_empty_content() {
    let mut conn = store();
    let err = remember(&mut conn, &entry_in("project/demo-app", "  "));
    assert!(matches!(err, Err(StoreError::EmptyContent)));
  }

  #[test]
  fn recall_ranks_the_relevant_entry_first() {
    let mut conn = store();
    remember(
      &mut conn,
      &entry_in(
        "project/demo-app",
        "demo-app persists everything in SQLite, WAL mode",
      ),
    )
    .unwrap();
    remember(
      &mut conn,
      &entry_in("project/demo-app", "the release cadence is weekly"),
    )
    .unwrap();
    let hits = recall(
      &conn,
      "what does demo-app use for persisting data",
      &chain(),
      10,
    )
    .unwrap();
    assert!(!hits.is_empty());
    assert!(hits[0].entry.content.contains("SQLite"));
  }

  #[test]
  fn recall_respects_the_namespace_firewall() {
    let mut conn = store();
    remember(
      &mut conn,
      &entry_in("project/other-app", "other-app uses Postgres 16"),
    )
    .unwrap();
    let hits = recall(&conn, "postgres database", &chain(), 10).unwrap();
    assert!(hits.is_empty(), "sibling namespace must not leak");
  }

  #[test]
  fn recall_stems_query_terms() {
    let mut conn = store();
    remember(
      &mut conn,
      &entry_in("project/demo-app", "commits are formatted as subjects"),
    )
    .unwrap();
    let hits = recall(&conn, "commit formatting", &chain(), 10).unwrap();
    assert_eq!(hits.len(), 1, "porter stems formatting/formatted");
  }

  #[test]
  fn recall_touches_last_accessed() {
    let mut conn = store();
    let e = remember(
      &mut conn,
      &entry_in("project/demo-app", "the queue drains nightly"),
    )
    .unwrap();
    assert_eq!(e.last_accessed_at, None);
    recall(&conn, "queue", &chain(), 10).unwrap();
    let back = get(&conn, &e.id).unwrap();
    assert!(back.last_accessed_at.is_some());
  }

  #[test]
  fn hostile_query_strings_never_error() {
    let conn = store();
    for q in ["\"unbalanced", "a AND (", "NEAR/3 x", "col:val", "*", "  "] {
      let hits = recall(&conn, q, &chain(), 10).unwrap();
      assert!(hits.is_empty());
    }
  }

  #[test]
  fn supersede_chains_and_hides_the_old_entry() {
    let mut conn = store();
    let old = remember(
      &mut conn,
      &entry_in("project/demo-app", "the user edits in VS Code"),
    )
    .unwrap();
    let new = supersede(
      &mut conn,
      &old.id,
      &entry_in("project/demo-app", "the user edits in Neovim"),
    )
    .unwrap();
    let old_back = get(&conn, &old.id).unwrap();
    assert_eq!(old_back.superseded_by, Some(new.id.clone()));
    let hits = recall(&conn, "editor edits", &chain(), 10).unwrap();
    assert_eq!(hits.len(), 1, "superseded entry never surfaces");
    assert_eq!(hits[0].entry.id, new.id);
  }

  #[test]
  fn supersede_refuses_a_dead_chain_link() {
    let mut conn = store();
    let old =
      remember(&mut conn, &entry_in("project/demo-app", "first fact")).unwrap();
    supersede(
      &mut conn,
      &old.id,
      &entry_in("project/demo-app", "second fact"),
    )
    .unwrap();
    let err = supersede(
      &mut conn,
      &old.id,
      &entry_in("project/demo-app", "third fact"),
    );
    assert!(matches!(err, Err(StoreError::AlreadySuperseded(_))));
    let err = supersede(
      &mut conn,
      "01912d68-783e-7cde-8f1a-4b2c9e0f3a71",
      &entry_in("project/demo-app", "orphan"),
    );
    assert!(matches!(err, Err(StoreError::EntryNotFound(_))));
  }

  #[test]
  fn forget_removes_entry_index_and_unlinks_chains() {
    let mut conn = store();
    let old = remember(
      &mut conn,
      &entry_in("project/demo-app", "obsolete secret fact"),
    )
    .unwrap();
    let new = supersede(
      &mut conn,
      &old.id,
      &entry_in("project/demo-app", "replacement fact"),
    )
    .unwrap();
    forget(&mut conn, &new.id).unwrap();
    assert!(matches!(
      get(&conn, &new.id),
      Err(StoreError::EntryNotFound(_))
    ));
    let hits = recall(&conn, "replacement", &chain(), 10).unwrap();
    assert!(hits.is_empty(), "forgotten entry left the index");
    let old_back = get(&conn, &old.id).unwrap();
    assert_eq!(old_back.superseded_by, None, "chain unlinked");
  }

  #[test]
  fn confirm_stamps_last_confirmed() {
    let mut conn = store();
    let e =
      remember(&mut conn, &entry_in("project/demo-app", "a standing fact"))
        .unwrap();
    assert_eq!(e.last_confirmed_at, None);
    confirm(&conn, &e.id).unwrap();
    let back = get(&conn, &e.id).unwrap();
    assert!(back.last_confirmed_at.is_some());
    assert!(matches!(
      confirm(&conn, "missing"),
      Err(StoreError::EntryNotFound(_))
    ));
  }

  #[test]
  fn duplicate_namespace_registration_errors() {
    let conn = store();
    let err = register_namespace(&conn, "project/demo-app", "again");
    assert!(matches!(err, Err(StoreError::NamespaceExists(_))));
  }
}
