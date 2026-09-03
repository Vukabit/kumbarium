//! The docket commands: file, list, judge, regrade, and the
//! roadmap pivot (D-032). Goals are watched here: the timeline
//! marks creep as it renders, and a passed goal outranks its
//! severity peers.

use std::process::ExitCode;

use super::super::{open_stores, style, tools};
use super::term::*;

fn docket_conn(
  state: &mut tools::ServerState,
) -> Result<&kumbarium_docket::Connection, String> {
  state.docket()
}

/// Days from today to the goal date (negative = overdue), or
/// None for a goalless matter.
fn days_to_goal(goal: Option<&str>, now_ms: i64) -> Option<i64> {
  let goal = goal?;
  let ms = kumbarium_util::parse_iso8601_ms(&format!("{goal}T00:00:00.000Z"))?;
  Some((ms - now_ms).div_euclid(86_400_000))
}

fn severity_paint(
  sty: &style::Style,
  sev: kumbarium_docket::Severity,
) -> String {
  let padded = format!("{:<6}", sev.as_str());
  match sev {
    kumbarium_docket::Severity::Urgent => sty.red(&padded),
    kumbarium_docket::Severity::High => sty.yellow(&padded),
    kumbarium_docket::Severity::Normal => padded,
    kumbarium_docket::Severity::Low => sty.dim(&padded),
  }
}

/// The goal cell: date plus creep mark, painted by proximity.
fn goal_paint(
  sty: &style::Style,
  days: Option<i64>,
  goal: Option<&str>,
) -> String {
  match (goal, days) {
    (Some(g), Some(d)) if d < 0 => sty.red(&format!("{g} over {}d", -d)),
    (Some(g), Some(d)) if d <= 7 => sty.yellow(&format!("{g} in {d}d")),
    (Some(g), _) => g.to_string(),
    _ => sty.dim("-").to_string(),
  }
}

fn age_of(created_at: &str, now_ms: i64) -> String {
  match kumbarium_util::parse_iso8601_ms(created_at) {
    Some(ms) => format!("{}d", ((now_ms - ms) / 86_400_000).max(0)),
    None => "?".into(),
  }
}

/// Sort for the timeline: most-overdue first, then severity,
/// then oldest.
fn timeline_order(tasks: &mut [kumbarium_docket::Task], now_ms: i64) {
  tasks.sort_by(|a, b| {
    let over = |t: &kumbarium_docket::Task| {
      days_to_goal(t.goal.as_deref(), now_ms)
        .filter(|d| *d < 0)
        .unwrap_or(0)
    };
    over(a)
      .cmp(&over(b))
      .then(b.severity.cmp(&a.severity))
      .then(a.created_at.cmp(&b.created_at))
  });
}

type TaskFlags = (
  Vec<String>,
  Option<kumbarium_docket::Severity>,
  Option<String>,
);

fn parse_task_flags(rest: &[&str]) -> Result<TaskFlags, String> {
  let mut words = Vec::new();
  let mut severity = None;
  let mut goal = None;
  let mut it = rest.iter();
  while let Some(arg) = it.next() {
    match *arg {
      "--severity" => {
        let raw = it.next().ok_or("--severity needs a value")?;
        severity = Some(
          kumbarium_docket::Severity::parse(raw)
            .ok_or_else(|| format!("unknown severity {raw:?}"))?,
        );
      }
      "--goal" => {
        let raw = it.next().ok_or("--goal needs YYYY-MM-DD")?;
        validate_goal_cli(raw)?;
        goal = Some((*raw).to_string());
      }
      word => words.push(word.to_string()),
    }
  }
  Ok((words, severity, goal))
}

fn validate_goal_cli(goal: &str) -> Result<(), String> {
  let ok = goal.len() == 10
    && kumbarium_util::parse_iso8601_ms(&format!("{goal}T00:00:00.000Z"))
      .is_some();
  if ok {
    Ok(())
  } else {
    Err(format!("invalid goal {goal:?}; use YYYY-MM-DD"))
  }
}

/// `kum task <ns> <content...> [--severity S] [--goal D]`
pub(crate) fn task_file_cmd(ns: &str, rest: &[&str]) -> ExitCode {
  let ns = kumbarium_librarian::normalize_namespace(ns);
  if let Err(e) = kumbarium_librarian::validate_namespace(&ns) {
    return fail(&format!("invalid namespace: {e}"));
  }
  let (words, severity, goal) = match parse_task_flags(rest) {
    Ok(v) => v,
    Err(e) => return fail(&e),
  };
  let content = words.join(" ");
  if content.trim().is_empty() {
    return fail("a matter needs content");
  }
  let (_, mut state) = match open_stores() {
    Ok(v) => v,
    Err(e) => return fail(&e),
  };
  match kumbarium_store::namespace_id(&state.library, &ns) {
    Ok(Some(_)) => {}
    Ok(None) => {
      return fail(&format!(
        "namespace {ns:?} is not registered; kumbarium namespace \
         add {ns}"
      ));
    }
    Err(e) => return fail(&e.to_string()),
  }
  let sty = style::Style::detect();
  let new = kumbarium_docket::NewTask {
    namespace: ns.clone(),
    content,
    agent_id: "kumbarium-cli".into(),
    source: String::new(),
    severity: severity.unwrap_or(kumbarium_docket::Severity::Normal),
    goal,
    status: kumbarium_docket::Status::Live,
  };
  let conn = match docket_conn(&mut state) {
    Ok(c) => c,
    Err(e) => return fail(&e),
  };
  let task = match kumbarium_docket::file_task(conn, &new) {
    Ok(t) => t,
    Err(e) => return fail(&e.to_string()),
  };
  let event = kumbarium_audit::Event {
    agent_id: "kumbarium-cli".into(),
    kind: kumbarium_audit::EventKind::TaskFile,
    scope: ns,
    detail: serde_json::json!({
      "id": task.id,
      "severity": task.severity.as_str(),
      "goal": task.goal,
    }),
  };
  if let Err(e) = kumbarium_audit::append(&state.audit, &event) {
    return fail(&format!("filed, but audit append failed: {e}"));
  }
  let goal_str = task
    .goal
    .as_deref()
    .map(|g| format!(" goal={g}"))
    .unwrap_or_default();
  println!(
    "filed {} [{}]{}",
    sty.id(kumbarium_docket::short_id(&task.id)),
    task.severity.as_str(),
    goal_str
  );
  ExitCode::SUCCESS
}

/// `kum task done|drop <id> [note...]`
pub(crate) fn task_judge_cmd(id: &str, to_done: bool, note: &str) -> ExitCode {
  let (_, mut state) = match open_stores() {
    Ok(v) => v,
    Err(e) => return fail(&e),
  };
  let sty = style::Style::detect();
  let conn = match docket_conn(&mut state) {
    Ok(c) => c,
    Err(e) => return fail(&e),
  };
  let full = match kumbarium_docket::resolve_id(conn, id) {
    Ok(f) => f,
    Err(e) => return fail(&e.to_string()),
  };
  let task = match kumbarium_docket::get(conn, &full) {
    Ok(t) => t,
    Err(e) => return fail(&e.to_string()),
  };
  let to = if to_done {
    kumbarium_docket::TaskState::Done
  } else {
    kumbarium_docket::TaskState::Dropped
  };
  let note = (!note.trim().is_empty()).then(|| note.trim().to_string());
  if let Err(e) = kumbarium_docket::set_state(conn, &full, to, note.as_deref())
  {
    return fail(&e.to_string());
  }
  let kind = if to_done {
    kumbarium_audit::EventKind::TaskDone
  } else {
    kumbarium_audit::EventKind::TaskDrop
  };
  let event = kumbarium_audit::Event {
    agent_id: "kumbarium-cli".into(),
    kind,
    scope: task.namespace.clone(),
    detail: serde_json::json!({ "id": full, "note": note }),
  };
  if let Err(e) = kumbarium_audit::append(&state.audit, &event) {
    return fail(&format!("judged, but audit append failed: {e}"));
  }
  println!(
    "{} {}",
    to.as_str(),
    sty.id(kumbarium_docket::short_id(&full))
  );
  ExitCode::SUCCESS
}

/// `kum task grade <id> [--severity S] [--goal D] [note...]`
pub(crate) fn task_grade_cmd(id: &str, rest: &[&str]) -> ExitCode {
  let (words, severity, goal) = match parse_task_flags(rest) {
    Ok(v) => v,
    Err(e) => return fail(&e),
  };
  if severity.is_none() && goal.is_none() {
    return fail("grade needs --severity and/or --goal");
  }
  let (_, mut state) = match open_stores() {
    Ok(v) => v,
    Err(e) => return fail(&e),
  };
  let sty = style::Style::detect();
  let conn = match docket_conn(&mut state) {
    Ok(c) => c,
    Err(e) => return fail(&e),
  };
  let full = match kumbarium_docket::resolve_id(conn, id) {
    Ok(f) => f,
    Err(e) => return fail(&e.to_string()),
  };
  let note = words.join(" ");
  let edit = kumbarium_docket::TaskEdit {
    severity,
    goal: goal.map(Some),
    note: (!note.trim().is_empty()).then(|| note.trim().to_string()),
    content: None,
    namespace: None,
  };
  let task =
    match kumbarium_docket::supersede_task(conn, &full, &edit, "kumbarium-cli")
    {
      Ok(t) => t,
      Err(e) => return fail(&e.to_string()),
    };
  let event = kumbarium_audit::Event {
    agent_id: "kumbarium-cli".into(),
    kind: kumbarium_audit::EventKind::TaskUpdate,
    scope: task.namespace.clone(),
    detail: serde_json::json!({
      "old_id": full,
      "new_id": task.id,
      "severity": task.severity.as_str(),
      "goal": task.goal,
      "note": edit.note,
    }),
  };
  if let Err(e) = kumbarium_audit::append(&state.audit, &event) {
    return fail(&format!("regraded, but audit append failed: {e}"));
  }
  println!(
    "regraded {} -> {} [{}] goal={}",
    sty.id(kumbarium_docket::short_id(&full)),
    sty.id(kumbarium_docket::short_id(&task.id)),
    task.severity.as_str(),
    task.goal.as_deref().unwrap_or("none")
  );
  ExitCode::SUCCESS
}

/// `kum task history <id>`: the chain, oldest first, goals and
/// grades visible so creep reads at a glance.
pub(crate) fn task_history_cmd(id: &str) -> ExitCode {
  let (_, mut state) = match open_stores() {
    Ok(v) => v,
    Err(e) => return fail(&e),
  };
  let sty = style::Style::detect();
  let conn = match docket_conn(&mut state) {
    Ok(c) => c,
    Err(e) => return fail(&e),
  };
  let full = match kumbarium_docket::resolve_id(conn, id) {
    Ok(f) => f,
    Err(e) => return fail(&e.to_string()),
  };
  let chain = match kumbarium_docket::history(conn, &full) {
    Ok(c) => c,
    Err(e) => return fail(&e.to_string()),
  };
  for (i, t) in chain.iter().enumerate() {
    let head = t.superseded_by.is_none();
    let marker = if head { "live" } else { "    " };
    let note = t
      .note
      .as_deref()
      .map(|n| format!(" {n:?}"))
      .unwrap_or_default();
    println!(
      "v{} {} {} [{}] goal={} {}{}",
      i + 1,
      sty.dim(marker),
      sty.id(kumbarium_docket::short_id(&t.id)),
      t.severity.as_str(),
      t.goal.as_deref().unwrap_or("none"),
      sty.dim(&t.created_at[..10]),
      note
    );
  }
  ExitCode::SUCCESS
}

/// `kum tasks [ns] [--all] [--severity S]`: the timeline.
pub(crate) fn tasks_cmd(rest: &[&str]) -> ExitCode {
  let mut ns: Option<String> = None;
  let mut all = false;
  let mut severity = None;
  let mut it = rest.iter();
  while let Some(arg) = it.next() {
    match *arg {
      "--all" => all = true,
      "--severity" => match it.next() {
        Some(raw) => {
          severity = Some(match kumbarium_docket::Severity::parse(raw) {
            Some(s) => s,
            None => return fail(&format!("unknown severity {raw:?}")),
          })
        }
        None => return fail("--severity needs a value"),
      },
      word => ns = Some(kumbarium_librarian::normalize_namespace(word)),
    }
  }
  let (_, mut state) = match open_stores() {
    Ok(v) => v,
    Err(e) => return fail(&e),
  };
  let sty = style::Style::detect();
  let conn = match docket_conn(&mut state) {
    Ok(c) => c,
    Err(e) => return fail(&e),
  };
  let scoped = ns.as_ref().map(|n| vec![n.clone()]);
  let mut tasks = match kumbarium_docket::tasks_in(conn, scoped.as_deref(), all)
  {
    Ok(t) => t,
    Err(e) => return fail(&e.to_string()),
  };
  if let Some(sev) = severity {
    tasks.retain(|t| t.severity == sev);
  }
  if tasks.is_empty() {
    println!("docket clear: no open matters");
    return ExitCode::SUCCESS;
  }
  let now = kumbarium_util::now_ms();
  timeline_order(&mut tasks, now);
  println!(
    "{}",
    sty.dim(
      "id        sev    goal                age  namespace            \
matter"
    )
  );
  // Matter column start: 8+2 + 6+1 + 19+2 + 3+2 + 20+1 = 64.
  // Overflow hanging-wraps there on a terminal, exactly like
  // `audit tail`; piped output stays single-line for grep.
  const MATTER_COL: usize = 64;
  let wrap_width = term_width()
    .filter(|w| *w > MATTER_COL + 16)
    .map(|w| w - MATTER_COL);
  for t in &tasks {
    let days = days_to_goal(t.goal.as_deref(), now);
    let goal_cell = goal_paint(&sty, days, t.goal.as_deref());
    let plain_goal_len = t
      .goal
      .as_deref()
      .map(|g| match days {
        Some(d) if d < 0 => g.len() + format!(" over {}d", -d).len(),
        Some(d) if d <= 7 => g.len() + format!(" in {d}d").len(),
        _ => g.len(),
      })
      .unwrap_or(1);
    let pad = 19usize.saturating_sub(plain_goal_len);
    let mark = match t.state {
      kumbarium_docket::TaskState::Done => " [done]",
      kumbarium_docket::TaskState::Dropped => " [dropped]",
      kumbarium_docket::TaskState::Open => "",
    };
    let matter = t.content.lines().next().unwrap_or("");
    let chunks = match wrap_width {
      Some(width) => wrap_words(matter, width),
      None => vec![matter.to_string()],
    };
    println!(
      "{}  {} {}{:pad$}  {:>3}  {:<20} {}{}",
      sty.id(kumbarium_docket::short_id(&t.id)),
      severity_paint(&sty, t.severity),
      goal_cell,
      "",
      age_of(&t.created_at, now),
      t.namespace,
      chunks.first().map(String::as_str).unwrap_or(""),
      sty.dim(mark),
    );
    for chunk in chunks.iter().skip(1) {
      println!("{:MATTER_COL$}{chunk}", "");
    }
  }
  ExitCode::SUCCESS
}

/// `kum roadmap [ns]`: the same matters pivoted by derived
/// horizon.
pub(crate) fn roadmap_cmd(ns: Option<&str>) -> ExitCode {
  let (_, mut state) = match open_stores() {
    Ok(v) => v,
    Err(e) => return fail(&e),
  };
  let sty = style::Style::detect();
  let conn = match docket_conn(&mut state) {
    Ok(c) => c,
    Err(e) => return fail(&e),
  };
  let ns = ns.map(kumbarium_librarian::normalize_namespace);
  let scoped = ns.as_ref().map(|n| vec![n.clone()]);
  let mut tasks =
    match kumbarium_docket::tasks_in(conn, scoped.as_deref(), false) {
      Ok(t) => t,
      Err(e) => return fail(&e.to_string()),
    };
  if tasks.is_empty() {
    println!("docket clear: no open matters");
    return ExitCode::SUCCESS;
  }
  let now = kumbarium_util::now_ms();
  timeline_order(&mut tasks, now);
  let bucket = |t: &kumbarium_docket::Task| -> usize {
    match days_to_goal(t.goal.as_deref(), now) {
      Some(d) if d < 0 => 0,
      Some(d) if d <= 7 => 1,
      Some(d) if d <= 30 => 2,
      Some(_) => 3,
      None => 4,
    }
  };
  let names = ["overdue", "now", "next", "later", "someday"];
  // Unscoped, matters group per shelf inside each horizon
  // (global first, then alphabetical); a scoped roadmap skips
  // the redundant group headers.
  let grouped = ns.is_none();
  for (i, name) in names.iter().enumerate() {
    let in_bucket: Vec<_> = tasks.iter().filter(|t| bucket(t) == i).collect();
    if in_bucket.is_empty() {
      continue;
    }
    println!("{}", sty.bold(name));
    let mut shelves: Vec<&str> =
      in_bucket.iter().map(|t| t.namespace.as_str()).collect();
    shelves.sort_unstable();
    shelves.dedup();
    shelves.sort_by_key(|s| (*s != "global", *s));
    let indent = if grouped { 4 } else { 2 };
    // Content column start: indent + 8+2 + 6+1.
    let content_col = indent + 17;
    let wrap_width = term_width()
      .filter(|w| *w > content_col + 16)
      .map(|w| w - content_col);
    for shelf in shelves {
      if grouped {
        println!("  {}", sty.dim(shelf));
      }
      for t in in_bucket.iter().filter(|t| t.namespace == shelf) {
        let goal = t
          .goal
          .as_deref()
          .map(|g| format!("  {g}"))
          .unwrap_or_default();
        let content = t.content.lines().next().unwrap_or("");
        let chunks = match wrap_width {
          Some(width) => wrap_words(content, width),
          None => vec![content.to_string()],
        };
        println!(
          "{:indent$}{}  {} {}",
          "",
          sty.id(kumbarium_docket::short_id(&t.id)),
          severity_paint(&sty, t.severity),
          chunks.first().map(String::as_str).unwrap_or(""),
        );
        for chunk in chunks.iter().skip(1) {
          println!("{:content_col$}{chunk}", "");
        }
        if !goal.is_empty() {
          println!("{:content_col$}{}", "", sty.dim(goal.trim_start()));
        }
      }
    }
  }
  ExitCode::SUCCESS
}

/// Render one task in full (`kum show` falling through to the
/// docket: ids are building-wide names).
pub(crate) fn show_task(
  state: &mut tools::ServerState,
  id: &str,
) -> Result<ExitCode, String> {
  let conn = state.docket()?;
  let full = match kumbarium_docket::resolve_id(conn, id) {
    Ok(f) => f,
    Err(kumbarium_docket::DocketError::TaskNotFound(_)) => {
      // Third shelf in the chain (D-034).
      return super::handoff::show_handoff(state, id);
    }
    Err(e) => return Err(e.to_string()),
  };
  let t = kumbarium_docket::get(conn, &full).map_err(|e| e.to_string())?;
  let sty = style::Style::detect();
  println!("{}", sty.bold("task (the docket)"));
  println!(
    "id:         {} (short: {})",
    t.id,
    kumbarium_docket::short_id(&t.id)
  );
  println!("namespace:  {}", t.namespace);
  println!("severity:   {}", t.severity.as_str());
  println!("goal:       {}", t.goal.as_deref().unwrap_or("none"));
  println!("state:      {}", t.state.as_str());
  if t.status != kumbarium_docket::Status::Live {
    println!("status:     {}", sty.yellow(t.status.as_str()));
  }
  println!(
    "filed:      {} by {}",
    local_display(&t.created_at),
    t.agent_id
  );
  if let Some(done) = &t.done_at {
    println!("judged:     {}", local_display(done));
  }
  if let Some(note) = &t.note {
    println!("note:       {note}");
  }
  if let Some(next) = &t.superseded_by {
    println!(
      "superseded: by {} (kum task history {})",
      kumbarium_docket::short_id(next),
      kumbarium_docket::short_id(&t.id)
    );
  }
  println!("\n{}", t.content);
  Ok(ExitCode::SUCCESS)
}

/// Relocate a task (`kum move` falling through to the docket):
/// a supersession into the target shelf, the move noted, same
/// discipline as memory (D-034: ids are building-wide names).
pub(crate) fn move_task_cmd(
  state: &mut tools::ServerState,
  id: &str,
  namespace: &str,
) -> ExitCode {
  match kumbarium_store::namespace_id(&state.library, namespace) {
    Ok(Some(_)) => {}
    Ok(None) => {
      return fail(&format!(
        "namespace {namespace:?} is not registered; kumbarium \
         namespace add {namespace}"
      ));
    }
    Err(e) => return fail(&e.to_string()),
  }
  let sty = style::Style::detect();
  let conn = match state.docket() {
    Ok(c) => c,
    Err(e) => return fail(&e),
  };
  let full = match kumbarium_docket::resolve_id(conn, id) {
    Ok(f) => f,
    Err(kumbarium_docket::DocketError::TaskNotFound(_)) => {
      return fail(&format!("no entry or task with id {id:?}"));
    }
    Err(e) => return fail(&e.to_string()),
  };
  let old = match kumbarium_docket::get(conn, &full) {
    Ok(t) => t,
    Err(e) => return fail(&e.to_string()),
  };
  if old.namespace == namespace {
    return fail("task is already on that shelf");
  }
  let note = format!("moved from {}", old.namespace);
  let edit = kumbarium_docket::TaskEdit {
    namespace: Some(namespace.to_string()),
    note: Some(note.clone()),
    ..Default::default()
  };
  let task =
    match kumbarium_docket::supersede_task(conn, &full, &edit, "kumbarium-cli")
    {
      Ok(t) => t,
      Err(e) => return fail(&e.to_string()),
    };
  let event = kumbarium_audit::Event {
    agent_id: "kumbarium-cli".into(),
    kind: kumbarium_audit::EventKind::TaskUpdate,
    scope: namespace.to_string(),
    detail: serde_json::json!({
      "old_id": full,
      "new_id": task.id,
      "severity": task.severity.as_str(),
      "goal": task.goal,
      "note": note,
    }),
  };
  if let Err(e) = kumbarium_audit::append(&state.audit, &event) {
    return fail(&format!("moved, but audit append failed: {e}"));
  }
  println!(
    "moved task {} -> {} (now on {})",
    sty.id(kumbarium_docket::short_id(&full)),
    sty.id(kumbarium_docket::short_id(&task.id)),
    namespace
  );
  ExitCode::SUCCESS
}
