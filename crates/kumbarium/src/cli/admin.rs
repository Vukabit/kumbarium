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

pub(crate) fn audit_tail(n: usize, scope: Option<&str>) -> ExitCode {
  let (_, state) = match open_stores() {
    Ok(v) => v,
    Err(e) => return fail(&e),
  };
  match kumbarium_audit::tail(&state.audit, n, scope) {
    Ok(events) => {
      let sty = style::Style::detect();
      println!(
        "{}",
        sty.dim(
          "at (local)           kind        agent                \
scope                detail"
        )
      );
      // Columns before detail: 19+2 + 9+1 + 20+1 + 20+1 = 73.
      const DETAIL_COL: usize = 75;
      let wrap_width = term_width()
        .filter(|w| *w > DETAIL_COL + 16)
        .map(|w| w - DETAIL_COL);
      for e in events {
        let detail = kumbarium_audit::describe_event(&e.kind, &e.detail);
        let chunks = match wrap_width {
          Some(width) => wrap_words(&detail, width),
          None => vec![detail.clone()],
        };
        println!(
          "{}  {} {:<20} {:<20} {}",
          sty.dim(&local_display(&e.at)),
          sty.event(&format!("{:<11}", e.kind)),
          e.agent_id,
          e.scope,
          chunks.first().map(String::as_str).unwrap_or("")
        );
        for chunk in chunks.iter().skip(1) {
          println!("{:DETAIL_COL$}{chunk}", "");
        }
      }
      ExitCode::SUCCESS
    }
    Err(e) => fail(&e.to_string()),
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
    let mut line = format!("  docket:    {open} open");
    if urgent > 0 {
      line.push_str(&format!(" ({urgent} urgent)"));
    }
    if pending > 0 {
      line.push_str(&format!(", {pending} pending"));
    }
    println!("{line}");
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
    ("audit.db", &p.audit_db),
  ] {
    if let Ok(meta) = std::fs::metadata(path) {
      println!("  {name:<10} {} KB", meta.len() / 1024);
    }
  }
  ExitCode::SUCCESS
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
  println!("{} {}", sty.dim("target:    "), env!("KUMBARIUM_TARGET"));
  println!(
    "{} {}",
    sty.dim("repository:"),
    env!("CARGO_PKG_REPOSITORY")
  );
  ExitCode::SUCCESS
}
