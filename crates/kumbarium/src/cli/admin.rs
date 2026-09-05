//! Upkeep: namespaces, status, the witness readers, config.

use std::process::ExitCode;

use super::super::{config, open_stores, paths, style};
use super::term::*;

pub(crate) fn namespace_add(path: &str, description: &str) -> ExitCode {
  let path = &kumbarium_librarian::normalize_namespace(path);
  if let Err(e) = kumbarium_librarian::validate_namespace(path) {
    return fail(&format!("invalid namespace {path:?}: {e}"));
  }
  if let Some(word) = kumbarium_librarian::reserved_word(path) {
    return fail(&format!(
      "{word:?} is reserved by the CLI grammar (current or \
       roadmap command); use a multi-segment path like \
       project/{word}"
    ));
  }
  let (_, state) = match open_stores() {
    Ok(v) => v,
    Err(e) => return fail(&e),
  };
  match kumbarium_store::register_namespace(&state.library, path, description) {
    Ok(_) => {
      println!("registered {path}");
      ExitCode::SUCCESS
    }
    Err(e) => fail(&e.to_string()),
  }
}

pub(crate) fn namespace_describe(path: &str, description: &str) -> ExitCode {
  let path = &kumbarium_librarian::normalize_namespace(path);
  if description.trim().is_empty() {
    return fail("describe needs the new description text");
  }
  let (_, state) = match open_stores() {
    Ok(v) => v,
    Err(e) => return fail(&e),
  };
  match kumbarium_store::describe_namespace(&state.library, path, description) {
    Ok(()) => {
      println!("described {path}: {description}");
      ExitCode::SUCCESS
    }
    Err(e) => fail(&e.to_string()),
  }
}

pub(crate) fn namespace_list() -> ExitCode {
  let (_, state) = match open_stores() {
    Ok(v) => v,
    Err(e) => return fail(&e),
  };
  match kumbarium_store::namespaces(&state.library) {
    Ok(rows) => {
      let sty = style::Style::detect();
      println!(
        "{}",
        sty.dim("namespace             [created]     description")
      );
      for (path, description, created_at) in rows {
        let day = created_at.get(..10).unwrap_or(&created_at);
        println!(
          "{}  [{day}]  {description}",
          sty.bold(&format!("{path:<20}"))
        );
      }
      ExitCode::SUCCESS
    }
    Err(e) => fail(&e.to_string()),
  }
}

const AUDIT_COLS: &[Col] = &[
  Col {
    title: "at (local)",
    width: 19,
  },
  Col {
    title: "kind",
    width: 15,
  },
  Col {
    title: "agent",
    width: 20,
  },
  Col {
    title: "scope",
    width: 20,
  },
  Col {
    title: "detail",
    width: 0,
  },
];

/// One event as an aligned, wrapped row (shared by tail and
/// follow, so the two renderings cannot drift).
fn print_event_row(e: &kumbarium_audit::StoredEvent, sty: &style::Style) {
  let detail = kumbarium_audit::describe_event(&e.kind, &e.detail);
  let lines = hang(body_col(AUDIT_COLS), &detail);
  println!(
    "{} {} {} {} {}",
    sty.dim(&cell(AUDIT_COLS, 0, &local_display(&e.at))),
    sty.event(&cell(AUDIT_COLS, 1, &e.kind)),
    cell(AUDIT_COLS, 2, &e.agent_id),
    cell(AUDIT_COLS, 3, &e.scope),
    lines[0]
  );
  for line in &lines[1..] {
    println!("{line}");
  }
}

pub(crate) fn audit_tail(n: usize, scope: Option<&str>) -> ExitCode {
  let (_, state) = match open_stores() {
    Ok(v) => v,
    Err(e) => return fail(&e),
  };
  match kumbarium_audit::tail(&state.audit, n, scope) {
    Ok(events) => {
      let sty = style::Style::detect();
      println!("{}", sty.dim(&table_header(AUDIT_COLS)));
      for e in events {
        print_event_row(&e, &sty);
      }
      ExitCode::SUCCESS
    }
    Err(e) => fail(&e.to_string()),
  }
}

/// `kum audit follow`: stream witnessed events as they land.
/// The one real-time diagnostic the OS cannot give (a growing
/// log, not a re-run command: this is what `watch` cannot do).
/// Prints a short backlog, then polls for new rows and streams
/// them oldest-first; Ctrl-C to stop.
pub(crate) fn audit_follow(n: usize, scope: Option<&str>) -> ExitCode {
  let (_, state) = match open_stores() {
    Ok(v) => v,
    Err(e) => return fail(&e),
  };
  let sty = style::Style::detect();
  println!("{}", sty.dim(&table_header(AUDIT_COLS)));
  // The backlog, oldest-first, so the stream reads as one
  // continuous chronological tail.
  match kumbarium_audit::tail(&state.audit, n, scope) {
    Ok(mut events) => {
      events.reverse();
      for e in &events {
        print_event_row(e, &sty);
      }
    }
    Err(e) => return fail(&e.to_string()),
  }
  // Stream from now on. Seed the cursor at the current head so
  // the backlog is not reprinted; each poll is a fresh WAL read
  // snapshot, so commits from live serve processes appear.
  let mut cursor = match kumbarium_audit::max_rowid(&state.audit) {
    Ok(c) => c,
    Err(e) => return fail(&e.to_string()),
  };
  loop {
    std::thread::sleep(std::time::Duration::from_millis(1000));
    match kumbarium_audit::events_after(&state.audit, cursor, scope) {
      Ok(rows) => {
        for (rowid, e) in rows {
          print_event_row(&e, &sty);
          cursor = cursor.max(rowid);
        }
      }
      // A transient read error (e.g. mid-checkpoint) must not
      // kill a long-running follow; report once and keep going.
      Err(e) => eprintln!("kumbarium: follow read hiccup: {e}"),
    }
  }
}

/// Recompute the ledger's hash chain (D-029): tamper-evidence
/// verifiable by anyone holding the file, no trust in us needed.
pub(crate) fn audit_verify() -> ExitCode {
  let (_, state) = match open_stores() {
    Ok(v) => v,
    Err(e) => return fail(&e),
  };
  let sty = style::Style::detect();
  match kumbarium_audit::verify_chain(&state.audit) {
    Ok(kumbarium_audit::ChainStatus::Intact { events, head }) => {
      let head = head.unwrap_or_default();
      let short = head.get(..12).unwrap_or(&head);
      println!(
        "{} {} events, head {}",
        sty.green("chain intact:"),
        events,
        short
      );
      ExitCode::SUCCESS
    }
    Ok(kumbarium_audit::ChainStatus::Broken { index, id, at }) => {
      eprintln!(
        "{} first break at event {} (id {}, at {}); every\n\
         event from there on is untrustworthy",
        sty.red("chain BROKEN:"),
        index,
        id,
        at
      );
      ExitCode::FAILURE
    }
    Err(e) => fail(&e.to_string()),
  }
}

/// Open config.toml in the editor (the most-edited file in the
/// system deserves a door).
pub(crate) fn config_open() -> ExitCode {
  let p = match paths::resolve() {
    Ok(p) => p,
    Err(e) => return fail(&e.to_string()),
  };
  if !p.config_file.exists() {
    return fail(
      "no config file yet; write the template first: \
       kumbarium config --init",
    );
  }
  match open_in_editor(&p.config_file) {
    Ok(()) => ExitCode::SUCCESS,
    Err(e) => fail(&e),
  }
}

pub(crate) fn status_cmd() -> ExitCode {
  let (p, state) = match open_stores() {
    Ok(v) => v,
    Err(e) => return fail(&e),
  };
  let sty = style::Style::detect();
  let stats = match kumbarium_store::stats(&state.library) {
    Ok(stats) => stats,
    Err(e) => return fail(&e.to_string()),
  };
  println!("{}", sty.bold("library"));
  println!(
    "  entries:   {} live, {} superseded, {} retired",
    stats.live, stats.superseded, stats.retired
  );
  if stats.pending > 0 || stats.rejected > 0 {
    println!(
      "  desk:      {} pending (kum inbox), {} rejected",
      stats.pending, stats.rejected
    );
  }
  println!(
    "  sets:      {} ({} parts)",
    stats.set_heads, stats.set_parts
  );
  if p.docket_db.exists()
    && let Ok(conn) = kumbarium_docket::open(&p.docket_db)
    && let Ok((open, urgent, pending)) = kumbarium_docket::counts(&conn)
  {
    // Known ill-health belongs in the health summary: overdue
    // is computed here exactly as the timeline computes it.
    let overdue = kumbarium_docket::tasks_in(&conn, None, false)
      .map(|tasks| {
        let now = kumbarium_util::now_ms();
        tasks
          .iter()
          .filter(|t| {
            super::docket::days_to_goal(t.goal.as_deref(), now)
              .is_some_and(|d| d < 0)
          })
          .count()
      })
      .unwrap_or(0);
    let mut line = format!("  docket:    {open} open");
    let mut flags = Vec::new();
    if urgent > 0 {
      flags.push(format!("{urgent} urgent"));
    }
    if overdue > 0 {
      flags.push(format!("{overdue} overdue"));
    }
    if !flags.is_empty() {
      line.push_str(&format!(" ({})", flags.join(", ")));
    }
    if pending > 0 {
      line.push_str(&format!(", {pending} pending (kum inbox)"));
    }
    println!("{line}");
  }
  if p.handoff_db.exists()
    && let Ok(conn) = kumbarium_handoff::open(&p.handoff_db)
    && let Ok(pending) = kumbarium_handoff::pending_handoffs(&conn)
    && !pending.is_empty()
  {
    // A pending briefing is the sharpest injection surface in
    // the building; the health page must never be silent on it.
    println!(
      "  desk:      {} pending {} awaiting judgment (kum inbox)",
      pending.len(),
      if pending.len() == 1 {
        "briefing"
      } else {
        "briefings"
      }
    );
  }
  if p.secrets_db.exists()
    && let Ok(conn) = kumbarium_secrets::open(&p.secrets_db)
    && let Ok((live, grants)) = kumbarium_secrets::counts(&conn)
  {
    let sealing = match kumbarium_secrets::sealing_mode(&conn) {
      Ok(Some(kumbarium_secrets::Sealing::Plaintext)) => ", PLAINTEXT",
      _ => "",
    };
    let today = kumbarium_util::now_iso8601();
    let expired = kumbarium_secrets::list(&conn, None)
      .map(|rows| {
        rows
          .iter()
          .filter(|m| m.expires_at.as_deref().is_some_and(|d| &today[..10] > d))
          .count()
      })
      .unwrap_or(0);
    let exp = if expired > 0 {
      format!(" ({expired} EXPIRED)")
    } else {
      String::new()
    };
    println!("  secrets:   {live} stocked{exp}, {grants} grants{sealing}");
  }
  if p.leases_db.exists()
    && let Ok(conn) = kumbarium_leases::open(&p.leases_db)
    && let Ok(active) = kumbarium_leases::active_in(
      &conn,
      None,
      kumbarium_util::now_ms(),
      state.cfg.leases_ttl_minutes,
    )
  {
    let stale = kumbarium_leases::stale_in(
      &conn,
      kumbarium_util::now_ms(),
      state.cfg.leases_ttl_minutes,
    )
    .map(|v| v.len())
    .unwrap_or(0);
    if !active.is_empty() || stale > 0 {
      let stale_note = if stale > 0 {
        format!(", {stale} stale (kum leases)")
      } else {
        String::new()
      };
      println!("  reading room: {} active{stale_note}", active.len());
    }
  }
  {
    let live = super::super::procs::live(&p.procs_dir);
    if !live.is_empty() {
      let old = live
        .iter()
        .filter(|r| r.version != env!("CARGO_PKG_VERSION"))
        .count();
      let note = if old > 0 {
        format!(" ({old} on an older binary; kum serve reload)")
      } else {
        String::new()
      };
      println!("  processes: {} live{note}", live.len());
    }
  }
  match kumbarium_store::namespaces(&state.library) {
    Ok(rows) => {
      for (path, _, _) in rows {
        let n: i64 = state
          .library
          .query_row(
            "SELECT count(*) FROM entries e
             JOIN namespaces ns ON ns.id = e.namespace_id
             WHERE ns.path = ?1 AND e.superseded_by IS NULL
               AND e.retired_at IS NULL AND e.status = 'live'",
            [&path],
            |row| row.get(0),
          )
          .unwrap_or(0);
        println!("  {path:<22} {n}");
      }
    }
    Err(e) => return fail(&e.to_string()),
  }
  match kumbarium_audit::summary(&state.audit) {
    Ok((count, latest)) => {
      println!("{}", sty.bold("audit"));
      let last = latest
        .map(|at| local_display(&at))
        .unwrap_or_else(|| "never".into());
      println!("  events:    {count} (latest {last})");
    }
    Err(e) => return fail(&e.to_string()),
  }
  println!("{}", sty.bold("maintenance"));
  let mut shelves = vec![
    ("memory", p.backups_dir.join("memory")),
    ("audit", p.backups_dir.join("audit")),
  ];
  if p.docket_db.exists() {
    shelves.push(("docket", p.backups_dir.join("docket")));
  }
  if p.handoff_db.exists() {
    shelves.push(("handoff", p.backups_dir.join("handoff")));
  }
  for (name, dir) in shelves {
    let line = match kumbarium_store::latest_backup_ms(&dir) {
      Some(ms) => {
        let age_h = (kumbarium_util::now_ms() - ms).max(0) / 3_600_000;
        format!("last backup {age_h}h ago")
      }
      None => "no backups yet".into(),
    };
    println!("  {name:<10} {line}");
  }
  for (name, path) in [
    ("memory.db", &p.memory_db),
    ("docket.db", &p.docket_db),
    ("handoff.db", &p.handoff_db),
    ("audit.db", &p.audit_db),
  ] {
    if let Ok(meta) = std::fs::metadata(path) {
      println!("  {name:<10} {} KB", meta.len() / 1024);
    }
  }
  ExitCode::SUCCESS
}

/// `kum status --json`: the same health figures as the page,
/// as one machine-readable object (the porcelain the peers
/// ship; kumbarium's own agents are the natural consumers).
pub(crate) fn status_json() -> ExitCode {
  let (p, state) = match open_stores() {
    Ok(v) => v,
    Err(e) => return fail(&e),
  };
  let mut out = serde_json::Map::new();
  match kumbarium_store::stats(&state.library) {
    Ok(s) => {
      out.insert(
        "entries".into(),
        serde_json::json!({
          "live": s.live,
          "superseded": s.superseded,
          "retired": s.retired,
          "pending": s.pending,
          "rejected": s.rejected,
          "set_heads": s.set_heads,
          "set_parts": s.set_parts,
        }),
      );
    }
    Err(e) => return fail(&e.to_string()),
  }
  if p.docket_db.exists()
    && let Ok(conn) = kumbarium_docket::open(&p.docket_db)
    && let Ok((open, urgent, pending)) = kumbarium_docket::counts(&conn)
  {
    let overdue = kumbarium_docket::tasks_in(&conn, None, false)
      .map(|tasks| {
        let now = kumbarium_util::now_ms();
        tasks
          .iter()
          .filter(|t| {
            super::docket::days_to_goal(t.goal.as_deref(), now)
              .is_some_and(|d| d < 0)
          })
          .count()
      })
      .unwrap_or(0);
    out.insert(
      "docket".into(),
      serde_json::json!({
        "open": open,
        "urgent": urgent,
        "overdue": overdue,
        "pending": pending,
      }),
    );
  }
  if p.handoff_db.exists()
    && let Ok(conn) = kumbarium_handoff::open(&p.handoff_db)
    && let Ok(pending) = kumbarium_handoff::pending_handoffs(&conn)
  {
    out.insert("pending_briefings".into(), serde_json::json!(pending.len()));
  }
  if p.secrets_db.exists()
    && let Ok(conn) = kumbarium_secrets::open(&p.secrets_db)
    && let Ok((live, grants)) = kumbarium_secrets::counts(&conn)
  {
    let plaintext = matches!(
      kumbarium_secrets::sealing_mode(&conn),
      Ok(Some(kumbarium_secrets::Sealing::Plaintext))
    );
    let today = kumbarium_util::now_iso8601();
    let expired = kumbarium_secrets::list(&conn, None)
      .map(|rows| {
        rows
          .iter()
          .filter(|m| m.expires_at.as_deref().is_some_and(|d| &today[..10] > d))
          .count()
      })
      .unwrap_or(0);
    out.insert(
      "secrets".into(),
      serde_json::json!({
        "stocked": live,
        "expired": expired,
        "grants": grants,
        "plaintext": plaintext,
      }),
    );
  }
  if p.leases_db.exists()
    && let Ok(conn) = kumbarium_leases::open(&p.leases_db)
  {
    let now = kumbarium_util::now_ms();
    let ttl = state.cfg.leases_ttl_minutes;
    let active = kumbarium_leases::active_in(&conn, None, now, ttl)
      .map(|v| v.len())
      .unwrap_or(0);
    let stale = kumbarium_leases::stale_in(&conn, now, ttl)
      .map(|v| v.len())
      .unwrap_or(0);
    out.insert(
      "reading_room".into(),
      serde_json::json!({ "active": active, "stale": stale }),
    );
  }
  {
    let live = super::super::procs::live(&p.procs_dir);
    let old = live
      .iter()
      .filter(|r| r.version != env!("CARGO_PKG_VERSION"))
      .count();
    out.insert(
      "processes".into(),
      serde_json::json!({ "live": live.len(), "stale_binary": old }),
    );
  }
  match kumbarium_store::namespaces(&state.library) {
    Ok(rows) => {
      let mut list = Vec::new();
      for (path, description, created_at) in rows {
        let n: i64 = state
          .library
          .query_row(
            "SELECT count(*) FROM entries e
             JOIN namespaces ns ON ns.id = e.namespace_id
             WHERE ns.path = ?1 AND e.superseded_by IS NULL
               AND e.retired_at IS NULL AND e.status = 'live'",
            [&path],
            |row| row.get(0),
          )
          .unwrap_or(0);
        list.push(serde_json::json!({
          "path": path,
          "description": description,
          "created_at": created_at,
          "live_entries": n,
        }));
      }
      out.insert("namespaces".into(), serde_json::json!(list));
    }
    Err(e) => return fail(&e.to_string()),
  }
  match kumbarium_audit::summary(&state.audit) {
    Ok((count, latest)) => {
      out.insert(
        "audit".into(),
        serde_json::json!({ "events": count, "latest_at": latest }),
      );
    }
    Err(e) => return fail(&e.to_string()),
  }
  let mut backups = serde_json::Map::new();
  for name in ["memory", "audit", "docket", "handoff", "secrets", "leases"] {
    let dir = p.backups_dir.join(name);
    backups.insert(
      name.into(),
      match kumbarium_store::latest_backup_ms(&dir) {
        Some(ms) => {
          serde_json::json!(kumbarium_util::format_iso8601_ms(ms))
        }
        None => serde_json::Value::Null,
      },
    );
  }
  out.insert(
    "backups_latest_at".into(),
    serde_json::Value::Object(backups),
  );
  match serde_json::to_string_pretty(&serde_json::Value::Object(out)) {
    Ok(s) => {
      println!("{s}");
      ExitCode::SUCCESS
    }
    Err(e) => fail(&e.to_string()),
  }
}

pub(crate) fn config_cmd(init: bool) -> ExitCode {
  let p = match paths::resolve() {
    Ok(p) => p,
    Err(e) => return fail(&e.to_string()),
  };
  let sty = style::Style::detect();
  if init {
    if p.config_file.exists() {
      return fail(&format!(
        "config already exists at {}",
        p.config_file.display()
      ));
    }
    if let Some(dir) = p.config_file.parent()
      && let Err(e) = std::fs::create_dir_all(dir)
    {
      return fail(&format!("creating config dir: {e}"));
    }
    if let Err(e) = kumbarium_util::write_atomically(
      &p.config_file,
      config::TEMPLATE.as_bytes(),
    ) {
      return fail(&format!("writing config: {e}"));
    }
    println!("{}", shell_quote(&p.config_file.display().to_string()));
    return ExitCode::SUCCESS;
  }
  let (cfg, source) = match std::fs::read_to_string(&p.config_file) {
    Ok(text) => {
      let (cfg, warnings) = config::parse(&text);
      for w in warnings {
        eprintln!("kumbarium: {w}");
      }
      (cfg, "file")
    }
    Err(_) => (config::Config::default(), "defaults"),
  };
  println!(
    "{} {} ({source})",
    sty.dim("config:"),
    p.config_file.display()
  );
  let d = config::Config::default();
  let mark = |cur: i64, def: i64| -> String {
    if cur == def {
      String::new()
    } else {
      sty.yellow("  (custom)")
    }
  };
  let rows: [(&str, i64, i64); 11] = [
    (
      "backup.interval_hours",
      cfg.backup_interval_hours,
      d.backup_interval_hours,
    ),
    (
      "backup.library_recent",
      cfg.library_recent as i64,
      d.library_recent as i64,
    ),
    (
      "backup.library_dailies",
      cfg.library_dailies as i64,
      d.library_dailies as i64,
    ),
    (
      "backup.library_weeklies",
      cfg.library_weeklies as i64,
      d.library_weeklies as i64,
    ),
    (
      "backup.audit_recent",
      cfg.audit_recent as i64,
      d.audit_recent as i64,
    ),
    (
      "backup.audit_dailies",
      cfg.audit_dailies as i64,
      d.audit_dailies as i64,
    ),
    (
      "backup.audit_weeklies",
      cfg.audit_weeklies as i64,
      d.audit_weeklies as i64,
    ),
    (
      "write.split_target",
      cfg.split_target as i64,
      d.split_target as i64,
    ),
    (
      "history.collapse_max_changed_lines",
      cfg.collapse_max_changed_lines as i64,
      d.collapse_max_changed_lines as i64,
    ),
    (
      "recall.default_limit",
      cfg.recall_default_limit as i64,
      d.recall_default_limit as i64,
    ),
    (
      "janitor.dormant_days",
      cfg.janitor_dormant_days,
      d.janitor_dormant_days,
    ),
  ];
  for (key, cur, def) in rows {
    println!("{key:<36} {cur}{}", mark(cur, def));
  }
  if !cfg.aliases.is_empty() {
    println!("\n{}", sty.bold("aliases (personal vocabulary):"));
    for (name, expansion) in &cfg.aliases {
      println!("{name:<12} {}", sty.dim(expansion));
    }
  }
  ExitCode::SUCCESS
}

/// Build provenance, deterministic-from-source only (no build
/// timestamp: the binary stays reproducible from its commit).
/// Values are baked in by build.rs; a git-less build says
/// "unknown" rather than lying.
pub(crate) fn version_cmd() -> ExitCode {
  let sty = style::Style::detect();
  let sha = env!("KUMBARIUM_GIT_SHA");
  let short = sha.get(..12).unwrap_or(sha);
  let dirty = match env!("KUMBARIUM_GIT_DIRTY") {
    "true" => ", dirty",
    "false" => ", clean",
    _ => "",
  };
  println!(
    "kumbarium {} ({})",
    env!("CARGO_PKG_VERSION"),
    env!("KUMBARIUM_BUILD_PROFILE")
  );
  println!(
    "{} {short} ({}{dirty})",
    sty.dim("commit:    "),
    env!("KUMBARIUM_GIT_BRANCH")
  );
  // The release provenance: a bare tag means this IS that
  // release; a `-N-g<sha>` suffix means N commits past it (a
  // build between releases), and `-dirty` means uncommitted.
  let describe = env!("KUMBARIUM_GIT_DESCRIBE");
  // A bare tag matching this version IS the release; "unknown"
  // is a git-less build. Anything else is a build past the last
  // tag, and says so.
  let on_release = describe == concat!("v", env!("CARGO_PKG_VERSION"))
    || describe == "unknown";
  let provenance = if on_release {
    String::new()
  } else {
    format!(" ({})", sty.dim("ahead of the last release tag"))
  };
  println!("{} {describe}{provenance}", sty.dim("build:     "));
  println!("{} {}", sty.dim("target:    "), env!("KUMBARIUM_TARGET"));
  println!(
    "{} {}",
    sty.dim("repository:"),
    env!("CARGO_PKG_REPOSITORY")
  );
  ExitCode::SUCCESS
}
