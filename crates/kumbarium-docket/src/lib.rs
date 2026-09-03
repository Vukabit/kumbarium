//! The docket: tasks and the roadmap (docs/design/docket.md,
//! D-032). A task is a matter before the house; edits are
//! supersessions, done and dropped keep the row, the desk's
//! status gate is reused verbatim, and goals are targets the
//! read side watches, never alarms. Held to the section
//! inheritance contract (D-033): governed like memory, never
//! recalled like memory (no split, no links, no FTS, no
//! confidence).

#![forbid(unsafe_code)]

use std::path::Path;

use rusqlite::params;

pub use rusqlite::Connection;

const MIGRATIONS: &[(i64, &str, &str)] =
  &[(1, "0001_init", include_str!("../migrations/0001_init.sql"))];

#[derive(Debug, thiserror::Error)]
pub enum DocketError {
  #[error("sqlite error: {0}")]
  Sqlite(#[from] rusqlite::Error),
  #[error("migration {0} failed: {1}")]
  Migration(i64, rusqlite::Error),
  #[error("no task with id {0:?}")]
  TaskNotFound(String),
  #[error("id fragment {0:?} matches more than one task")]
  AmbiguousId(String),
  #[error("task {0:?} is already superseded")]
  AlreadySuperseded(String),
  #[error("task {0:?} is not open")]
  NotOpen(String),
  #[error("task {0:?} is not pending")]
  NotPending(String),
  #[error("task content is empty")]
  EmptyContent,
  #[error(
    "task content exceeds {1} bytes; a matter too big to state \
     is two matters, or a design document that belongs in memory"
  )]
  ContentTooLong(String, usize),
}

/// A matter too big to state is two matters (the inheritance
/// contract: no auto-split on this shelf).
pub const MAX_CONTENT: usize = 1500;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Severity {
  Low,
  Normal,
  High,
  Urgent,
}

impl Severity {
  pub fn as_str(self) -> &'static str {
    match self {
      Severity::Low => "low",
      Severity::Normal => "normal",
      Severity::High => "high",
      Severity::Urgent => "urgent",
    }
  }

  pub fn parse(s: &str) -> Option<Severity> {
    match s {
      "low" => Some(Severity::Low),
      "normal" => Some(Severity::Normal),
      "high" => Some(Severity::High),
      "urgent" => Some(Severity::Urgent),
      _ => None,
    }
  }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TaskState {
  Open,
  Done,
  Dropped,
}

impl TaskState {
  pub fn as_str(self) -> &'static str {
    match self {
      TaskState::Open => "open",
      TaskState::Done => "done",
      TaskState::Dropped => "dropped",
    }
  }

  pub fn parse(s: &str) -> Option<TaskState> {
    match s {
      "open" => Some(TaskState::Open),
      "done" => Some(TaskState::Done),
      "dropped" => Some(TaskState::Dropped),
      _ => None,
    }
  }
}

/// Circulation status, D-027 verbatim: the desk judges tasks
/// with the same three verbs it judges memories with.
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

/// What a filer supplies; the docket mints id and timestamps.
/// `status` is set by the GATE from write policy (D-027), never
/// chosen by the writer; `namespace` arrives validated and
/// registry-checked by the librarian (D-033).
#[derive(Debug, Clone)]
pub struct NewTask {
  pub namespace: String,
  pub content: String,
  pub agent_id: String,
  pub source: String,
  pub severity: Severity,
  pub goal: Option<String>,
  pub status: Status,
}

/// A matter as filed.
#[derive(Debug, Clone)]
pub struct Task {
  pub id: String,
  pub namespace: String,
  pub content: String,
  pub agent_id: String,
  pub source: String,
  pub severity: Severity,
  pub goal: Option<String>,
  pub state: TaskState,
  pub done_at: Option<String>,
  pub superseded_by: Option<String>,
  pub note: Option<String>,
  pub status: Status,
  pub created_at: String,
  pub updated_at: String,
}

/// Open (creating if absent) the docket shelf at `path`.
pub fn open(path: &Path) -> Result<Connection, DocketError> {
  let conn = Connection::open(path)?;
  configure(&conn)?;
  migrate(&conn)?;
  Ok(conn)
}

/// In-memory docket with the schema applied (tests, dry runs).
pub fn open_in_memory() -> Result<Connection, DocketError> {
  let conn = Connection::open_in_memory()?;
  configure(&conn)?;
  migrate(&conn)?;
  Ok(conn)
}

fn configure(conn: &Connection) -> Result<(), DocketError> {
  conn.pragma_update(None, "journal_mode", "wal")?;
  // Multi-process by design (D-015), same as every shelf.
  conn.pragma_update(None, "busy_timeout", 5000)?;
  conn.pragma_update(None, "foreign_keys", "on")?;
  conn.pragma_update(None, "synchronous", "normal")?;
  Ok(())
}

fn migrate(conn: &Connection) -> Result<(), DocketError> {
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
      .map_err(|e| DocketError::Migration(*version, e))?;
    conn.execute(
      "INSERT INTO schema_version (version, name, applied_at)
       VALUES (?1, ?2, ?3)",
      params![version, name, kumbarium_util::now_iso8601()],
    )?;
  }
  Ok(())
}

/// File a matter. Content must be one statement (MAX_CONTENT).
pub fn file_task(
  conn: &Connection,
  new: &NewTask,
) -> Result<Task, DocketError> {
  validate_content(&new.content)?;
  let id = kumbarium_util::generate_id();
  let now = kumbarium_util::now_iso8601();
  conn.execute(
    "INSERT INTO tasks
       (id, namespace, content, agent_id, source, severity,
        goal, status, created_at, updated_at)
     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?9)",
    params![
      id,
      new.namespace,
      new.content,
      new.agent_id,
      new.source,
      new.severity.as_str(),
      new.goal,
      new.status.as_str(),
      now,
    ],
  )?;
  get(conn, &id)
}

fn validate_content(content: &str) -> Result<(), DocketError> {
  if content.trim().is_empty() {
    return Err(DocketError::EmptyContent);
  }
  if content.len() > MAX_CONTENT {
    return Err(DocketError::ContentTooLong(
      content.chars().take(40).collect(),
      MAX_CONTENT,
    ));
  }
  Ok(())
}

/// The short display form of an id: its last 8 hex chars.
pub fn short_id(id: &str) -> &str {
  id.get(id.len().saturating_sub(8)..).unwrap_or(id)
}

/// Fetch one task by full id.
pub fn get(conn: &Connection, id: &str) -> Result<Task, DocketError> {
  let mut stmt = conn.prepare(
    "SELECT id, namespace, content, agent_id, source, severity,
            goal, state, done_at, superseded_by, note, status,
            created_at, updated_at
     FROM tasks WHERE id = ?1",
  )?;
  stmt.query_row([id], row_to_task).map_err(|e| match e {
    rusqlite::Error::QueryReturnedNoRows => {
      DocketError::TaskNotFound(id.to_string())
    }
    other => other.into(),
  })
}

/// Resolve an id fragment git-style, same grammar as memory:
/// full id passes through; any unique hex-ish fragment of 4+
/// chars matches; ambiguity errors, never guesses.
pub fn resolve_id(
  conn: &Connection,
  fragment: &str,
) -> Result<String, DocketError> {
  if kumbarium_util::is_valid_id(fragment) {
    return Ok(fragment.to_string());
  }
  let hexish = fragment.bytes().all(|b| b.is_ascii_hexdigit() || b == b'-');
  if fragment.len() < 4 || !hexish {
    return Err(DocketError::TaskNotFound(fragment.to_string()));
  }
  let mut stmt =
    conn.prepare("SELECT id FROM tasks WHERE id LIKE ?1 LIMIT 2")?;
  let matches = stmt
    .query_map([format!("%{fragment}%")], |row| row.get::<_, String>(0))?
    .collect::<Result<Vec<_>, _>>()?;
  match matches.as_slice() {
    [] => Err(DocketError::TaskNotFound(fragment.to_string())),
    [id] => Ok(id.clone()),
    _ => Err(DocketError::AmbiguousId(fragment.to_string())),
  }
}

/// Open matters on the given shelves (a namespace chain, or None
/// for every shelf), oldest first; live only. `include_all`
/// adds done, dropped, superseded, and non-live rows (the
/// forensics view).
pub fn tasks_in(
  conn: &Connection,
  namespaces: Option<&[String]>,
  include_all: bool,
) -> Result<Vec<Task>, DocketError> {
  let mut sql = String::from(
    "SELECT id, namespace, content, agent_id, source, severity,
            goal, state, done_at, superseded_by, note, status,
            created_at, updated_at
     FROM tasks WHERE 1=1",
  );
  if !include_all {
    sql.push_str(
      " AND state = 'open' AND status = 'live' \
       AND superseded_by IS NULL",
    );
  }
  let mut args: Vec<String> = Vec::new();
  if let Some(chain) = namespaces {
    let marks = (0..chain.len())
      .map(|i| format!("?{}", i + 1))
      .collect::<Vec<_>>()
      .join(", ");
    sql.push_str(&format!(" AND namespace IN ({marks})"));
    args.extend(chain.iter().cloned());
  }
  sql.push_str(" ORDER BY created_at ASC");
  let mut stmt = conn.prepare(&sql)?;
  let rows = stmt
    .query_map(rusqlite::params_from_iter(args.iter()), row_to_task)?
    .collect::<Result<Vec<_>, _>>()?;
  Ok(rows)
}

/// Pending matters, oldest first: the desk's docket queue.
pub fn pending_tasks(conn: &Connection) -> Result<Vec<Task>, DocketError> {
  let mut stmt = conn.prepare(
    "SELECT id, namespace, content, agent_id, source, severity,
            goal, state, done_at, superseded_by, note, status,
            created_at, updated_at
     FROM tasks
     WHERE status = 'pending' AND superseded_by IS NULL
     ORDER BY created_at ASC",
  )?;
  let rows = stmt
    .query_map([], row_to_task)?
    .collect::<Result<Vec<_>, _>>()?;
  Ok(rows)
}

/// What a regrade may change; None fields carry forward.
#[derive(Debug, Clone, Default)]
pub struct TaskEdit {
  pub content: Option<String>,
  pub severity: Option<Severity>,
  /// Some(None) clears the goal; None leaves it alone.
  pub goal: Option<Option<String>>,
  pub note: Option<String>,
}

/// Edit a matter by supersession (D-020): the old version chains
/// forward, the new head carries the changes. Chain heads only;
/// a pending head stays pending (judgment happens at the desk).
pub fn supersede_task(
  conn: &Connection,
  old_id: &str,
  edit: &TaskEdit,
  agent_id: &str,
) -> Result<Task, DocketError> {
  conn.execute_batch("BEGIN IMMEDIATE")?;
  let result = supersede_locked(conn, old_id, edit, agent_id);
  match &result {
    Ok(_) => conn.execute_batch("COMMIT")?,
    Err(_) => {
      let _ = conn.execute_batch("ROLLBACK");
    }
  }
  result
}

fn supersede_locked(
  conn: &Connection,
  old_id: &str,
  edit: &TaskEdit,
  agent_id: &str,
) -> Result<Task, DocketError> {
  let old = get(conn, old_id)?;
  if old.superseded_by.is_some() {
    return Err(DocketError::AlreadySuperseded(old_id.to_string()));
  }
  if old.state != TaskState::Open {
    return Err(DocketError::NotOpen(old_id.to_string()));
  }
  let content = edit.content.clone().unwrap_or_else(|| old.content.clone());
  validate_content(&content)?;
  let severity = edit.severity.unwrap_or(old.severity);
  let goal = match &edit.goal {
    Some(g) => g.clone(),
    None => old.goal.clone(),
  };
  let id = kumbarium_util::generate_id();
  let now = kumbarium_util::now_iso8601();
  conn.execute(
    "INSERT INTO tasks
       (id, namespace, content, agent_id, source, severity,
        goal, status, note, created_at, updated_at)
     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?10)",
    params![
      id,
      old.namespace,
      content,
      agent_id,
      old.source,
      severity.as_str(),
      goal,
      old.status.as_str(),
      edit.note,
      now,
    ],
  )?;
  conn.execute(
    "UPDATE tasks SET superseded_by = ?1, updated_at = ?2
     WHERE id = ?3",
    params![id, now, old_id],
  )?;
  get(conn, &id)
}

/// Judge a matter done or dropped: the row is KEPT (the docket
/// records judgments), done_at stamps when, an optional note
/// says why. Open, live chain heads only.
pub fn set_state(
  conn: &Connection,
  id: &str,
  to: TaskState,
  note: Option<&str>,
) -> Result<(), DocketError> {
  let task = get(conn, id)?;
  if task.superseded_by.is_some() {
    return Err(DocketError::AlreadySuperseded(id.to_string()));
  }
  if task.state != TaskState::Open || to == TaskState::Open {
    return Err(DocketError::NotOpen(id.to_string()));
  }
  let now = kumbarium_util::now_iso8601();
  conn.execute(
    "UPDATE tasks
     SET state = ?1, done_at = ?2, updated_at = ?2,
         note = COALESCE(?3, note)
     WHERE id = ?4",
    params![to.as_str(), now, note, id],
  )?;
  Ok(())
}

/// Promote a pending matter into circulation (human-only at the
/// desk; the caller witnesses).
pub fn approve(conn: &Connection, id: &str) -> Result<(), DocketError> {
  set_status(conn, id, Status::Live)
}

/// Decline a pending matter, kept as evidence of the judgment.
pub fn reject(conn: &Connection, id: &str) -> Result<(), DocketError> {
  set_status(conn, id, Status::Rejected)
}

fn set_status(
  conn: &Connection,
  id: &str,
  to: Status,
) -> Result<(), DocketError> {
  let n = conn.execute(
    "UPDATE tasks SET status = ?1, updated_at = ?2
     WHERE id = ?3 AND status = 'pending'",
    params![to.as_str(), kumbarium_util::now_iso8601(), id],
  )?;
  if n == 0 {
    let exists: bool = conn
      .query_row("SELECT 1 FROM tasks WHERE id = ?1", [id], |_| Ok(true))
      .or_else(|e| match e {
        rusqlite::Error::QueryReturnedNoRows => Ok(false),
        other => Err(other),
      })?;
    return Err(if exists {
      DocketError::NotPending(id.to_string())
    } else {
      DocketError::TaskNotFound(id.to_string())
    });
  }
  Ok(())
}

/// A matter's version chain, oldest first (regrades and edits;
/// the creep math reads goals off this).
pub fn history(conn: &Connection, id: &str) -> Result<Vec<Task>, DocketError> {
  let mut chain = vec![get(conn, id)?];
  // Walk backward.
  loop {
    let cur = &chain[0];
    let prev: Option<String> = conn
      .query_row(
        "SELECT id FROM tasks WHERE superseded_by = ?1",
        [&cur.id],
        |row| row.get(0),
      )
      .map(Some)
      .or_else(|e| match e {
        rusqlite::Error::QueryReturnedNoRows => Ok(None),
        other => Err(other),
      })?;
    match prev {
      Some(p) if chain.len() < 1000 => chain.insert(0, get(conn, &p)?),
      _ => break,
    }
  }
  // Walk forward.
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

/// (open, urgent-open, pending) counts for `kum status`.
pub fn counts(conn: &Connection) -> Result<(i64, i64, i64), DocketError> {
  let one = |sql: &str| -> Result<i64, DocketError> {
    Ok(conn.query_row(sql, [], |row| row.get(0))?)
  };
  Ok((
    one(
      "SELECT count(*) FROM tasks WHERE state = 'open'
       AND status = 'live' AND superseded_by IS NULL",
    )?,
    one(
      "SELECT count(*) FROM tasks WHERE state = 'open'
       AND status = 'live' AND superseded_by IS NULL
       AND severity = 'urgent'",
    )?,
    one(
      "SELECT count(*) FROM tasks WHERE status = 'pending'
       AND superseded_by IS NULL",
    )?,
  ))
}

fn row_to_task(row: &rusqlite::Row<'_>) -> Result<Task, rusqlite::Error> {
  let bad = |idx: usize, what: &str, raw: &str| {
    rusqlite::Error::FromSqlConversionFailure(
      idx,
      rusqlite::types::Type::Text,
      format!("unknown {what} {raw:?}").into(),
    )
  };
  let sev_raw: String = row.get(5)?;
  let severity =
    Severity::parse(&sev_raw).ok_or_else(|| bad(5, "severity", &sev_raw))?;
  let state_raw: String = row.get(7)?;
  let state =
    TaskState::parse(&state_raw).ok_or_else(|| bad(7, "state", &state_raw))?;
  let status_raw: String = row.get(11)?;
  let status =
    Status::parse(&status_raw).ok_or_else(|| bad(11, "status", &status_raw))?;
  Ok(Task {
    id: row.get(0)?,
    namespace: row.get(1)?,
    content: row.get(2)?,
    agent_id: row.get(3)?,
    source: row.get(4)?,
    severity,
    goal: row.get(6)?,
    state,
    done_at: row.get(8)?,
    superseded_by: row.get(9)?,
    note: row.get(10)?,
    status,
    created_at: row.get(12)?,
    updated_at: row.get(13)?,
  })
}

#[cfg(test)]
mod tests {
  use super::*;

  fn task_in(ns: &str, content: &str) -> NewTask {
    NewTask {
      namespace: ns.into(),
      content: content.into(),
      agent_id: "test-agent".into(),
      source: "unit-test".into(),
      severity: Severity::Normal,
      goal: None,
      status: Status::Live,
    }
  }

  #[test]
  fn file_and_list_scoped_by_chain() {
    let conn = open_in_memory().unwrap();
    file_task(&conn, &task_in("project/a", "fix the grelvix")).unwrap();
    file_task(&conn, &task_in("project/b", "sibling matter")).unwrap();
    file_task(&conn, &task_in("global", "org-wide matter")).unwrap();
    let chain = ["project/a".to_string(), "global".to_string()];
    let listed = tasks_in(&conn, Some(&chain), false).unwrap();
    assert_eq!(listed.len(), 2, "sibling shelf never leaks");
    assert!(listed.iter().all(|t| t.namespace != "project/b"));
  }

  #[test]
  fn pending_never_lists_and_desk_judges() {
    let conn = open_in_memory().unwrap();
    let mut new = task_in("global", "vendored matter");
    new.status = Status::Pending;
    let t = file_task(&conn, &new).unwrap();
    assert!(tasks_in(&conn, None, false).unwrap().is_empty());
    assert_eq!(pending_tasks(&conn).unwrap().len(), 1);
    approve(&conn, &t.id).unwrap();
    assert_eq!(tasks_in(&conn, None, false).unwrap().len(), 1);
    let err = approve(&conn, &t.id);
    assert!(matches!(err, Err(DocketError::NotPending(_))));
  }

  #[test]
  fn regrade_chains_and_creep_is_on_the_chain() {
    let conn = open_in_memory().unwrap();
    let mut new = task_in("global", "ship the docket");
    new.goal = Some("2026-09-10".into());
    let v1 = file_task(&conn, &new).unwrap();
    let v2 = supersede_task(
      &conn,
      &v1.id,
      &TaskEdit {
        goal: Some(Some("2026-09-20".into())),
        note: Some("slipped for the launch".into()),
        ..TaskEdit::default()
      },
      "test-agent",
    )
    .unwrap();
    assert_eq!(v2.goal.as_deref(), Some("2026-09-20"));
    assert_eq!(v2.severity, Severity::Normal, "carried forward");
    let chain = history(&conn, &v1.id).unwrap();
    assert_eq!(chain.len(), 2);
    assert_eq!(chain[0].goal.as_deref(), Some("2026-09-10"));
    assert_eq!(chain[1].goal.as_deref(), Some("2026-09-20"));
    // Only the head lists.
    assert_eq!(tasks_in(&conn, None, false).unwrap().len(), 1);
    // Superseded versions are frozen.
    let err = set_state(&conn, &v1.id, TaskState::Done, None);
    assert!(matches!(err, Err(DocketError::AlreadySuperseded(_))));
  }

  #[test]
  fn done_and_dropped_keep_the_row() {
    let conn = open_in_memory().unwrap();
    let a = file_task(&conn, &task_in("global", "matter a")).unwrap();
    let b = file_task(&conn, &task_in("global", "matter b")).unwrap();
    set_state(&conn, &a.id, TaskState::Done, None).unwrap();
    set_state(&conn, &b.id, TaskState::Dropped, Some("overtaken")).unwrap();
    assert!(tasks_in(&conn, None, false).unwrap().is_empty());
    let all = tasks_in(&conn, None, true).unwrap();
    assert_eq!(all.len(), 2, "judgments are kept");
    let done = get(&conn, &a.id).unwrap();
    assert!(done.done_at.is_some());
    let dropped = get(&conn, &b.id).unwrap();
    assert_eq!(dropped.note.as_deref(), Some("overtaken"));
    // A judged matter cannot be re-judged.
    let err = set_state(&conn, &a.id, TaskState::Dropped, None);
    assert!(matches!(err, Err(DocketError::NotOpen(_))));
  }

  #[test]
  fn oversized_matters_are_two_matters() {
    let conn = open_in_memory().unwrap();
    let long = "do the thing ".repeat(200);
    let err = file_task(&conn, &task_in("global", &long));
    assert!(matches!(err, Err(DocketError::ContentTooLong(_, _))));
  }

  #[test]
  fn fragments_resolve_uniquely_or_error() {
    let conn = open_in_memory().unwrap();
    let t = file_task(&conn, &task_in("global", "findable")).unwrap();
    let tail = &t.id[t.id.len() - 8..];
    assert_eq!(resolve_id(&conn, tail).unwrap(), t.id);
    assert!(matches!(
      resolve_id(&conn, "zzzz"),
      Err(DocketError::TaskNotFound(_))
    ));
  }
}
