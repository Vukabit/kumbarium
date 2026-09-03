//! Kumbarium: the librarian process. `serve` speaks MCP over
//! stdio (D-014); the rest is the human-facing CLI.

mod config;
mod diff;
mod help;
mod import;
mod markdown;
mod paths;
mod rpc;
mod style;
mod tools;

use std::process::ExitCode;

const VERSION: &str = env!("CARGO_PKG_VERSION");

// Unused only when this file is included by the `kum` alias
// binary, which supplies its own main.
#[allow(dead_code)]
fn main() -> ExitCode {
  run()
}

pub fn run() -> ExitCode {
  // Die quietly when stdout's reader hangs up (`kum list |
  // head`): restore the default SIGPIPE disposition Rust
  // overrides, instead of panicking mid-println. Never affects
  // `serve`: MCP clients read until we exit anyway.
  #[cfg(unix)]
  unsafe {
    libc::signal(libc::SIGPIPE, libc::SIG_DFL);
  }
  let args: Vec<String> = std::env::args().skip(1).collect();
  let argv: Vec<&str> = args.iter().map(String::as_str).collect();
  match argv.as_slice() {
    ["version"] => {
      println!("kumbarium {VERSION}");
      ExitCode::SUCCESS
    }
    ["paths"] => match paths::resolve() {
      Ok(p) => {
        println!("{p}");
        ExitCode::SUCCESS
      }
      Err(e) => fail(&e.to_string()),
    },
    ["serve"] => serve(),
    ["namespace", "add", path, rest @ ..] => {
      namespace_add(path, &rest.join(" "))
    }
    ["namespace", "list"] => namespace_list(),
    ["import", "claude", rest @ ..] => import_claude(rest),
    ["backup"] => backup_now(),
    ["list"] => list_entries(None, false),
    ["list", "--all"] => list_entries(None, true),
    ["list", ns] => list_entries(Some(ns), false),
    ["list", ns, "--all"] => list_entries(Some(ns), true),
    ["show", id] => show_entry(id, false),
    ["show", id, "--full"] => show_entry(id, true),
    ["history", id, rest @ ..] => {
      let with_diff = rest.contains(&"--diff");
      let all = rest.contains(&"--all");
      history_cmd(id, with_diff, all)
    }
    ["revert", id] => revert_cmd(id, false),
    ["revert", id, "--apply"] => revert_cmd(id, true),
    ["confirm", id] => confirm_cmd(id),
    ["janitor"] => janitor_cmd(false),
    ["janitor", "--apply"] => janitor_cmd(true),
    ["retire", id] => retire_cmd(id, true),
    ["unretire", id] => retire_cmd(id, false),
    ["status"] => status_cmd(),
    ["config"] => config_cmd(false),
    ["config", "--init"] => config_cmd(true),
    ["grep", pattern] => grep_cmd(pattern, None, false),
    ["grep", pattern, "--all"] => grep_cmd(pattern, None, true),
    ["grep", pattern, ns] => grep_cmd(pattern, Some(ns), false),
    ["grep", pattern, ns, "--all"] => grep_cmd(pattern, Some(ns), true),
    ["move", id, ns] => move_cmd(id, ns),
    ["audit", "tail", rest @ ..] => {
      let mut n = 20usize;
      let mut scope = None;
      let mut it = rest.iter();
      while let Some(arg) = it.next() {
        match *arg {
          "--scope" => match it.next() {
            Some(sc) => scope = Some(*sc),
            None => return fail("--scope needs a namespace"),
          },
          num => match num.parse() {
            Ok(v) => n = v,
            Err(_) => {
              return fail("audit tail takes a number");
            }
          },
        }
      }
      audit_tail(n, scope)
    }
    ["instructions"] => {
      let sty = style::Style::detect();
      if let Some(md) = help::page("instructions") {
        println!("{}", markdown::render(md, &sty));
      }
      ExitCode::SUCCESS
    }
    ["instructions", "--snippet"] => {
      // Raw and unrendered: this output is file content.
      print!("{}", help::SNIPPET);
      ExitCode::SUCCESS
    }
    ["help"] | ["--help"] | ["-h"] => {
      println!("{USAGE}\n\ntopics: kumbarium help <topic>");
      println!("  {}", help::TOPICS);
      ExitCode::SUCCESS
    }
    ["help", topic] => match help::page(topic) {
      Some(md) => {
        let sty = style::Style::detect();
        println!("{}", markdown::render(md, &sty));
        ExitCode::SUCCESS
      }
      None => fail(&format!(
        "no help topic {topic:?}; topics: {}",
        help::TOPICS
      )),
    },
    ["audit", "export", rest @ ..] => {
      let to_stdout = rest.contains(&"--stdout");
      let raw = rest.contains(&"--raw");
      audit_export(to_stdout, raw)
    }
    [] => {
      println!("{USAGE}");
      ExitCode::SUCCESS
    }
    other => {
      eprintln!("kumbarium: unknown command {other:?}");
      eprintln!("{USAGE}");
      ExitCode::FAILURE
    }
  }
}

/// Open both databases at their platform paths, creating the
/// data directory on first run.
fn open_stores() -> Result<(paths::Paths, tools::ServerState), String> {
  let p = paths::resolve().map_err(|e| e.to_string())?;
  let data_dir = p.library_db.parent().ok_or("library path has no parent")?;
  std::fs::create_dir_all(data_dir)
    .map_err(|e| format!("creating data dir: {e}"))?;
  let library = kumbarium_store::open(&p.library_db)
    .map_err(|e| format!("opening library: {e}"))?;
  let audit = kumbarium_audit::open(&p.audit_db)
    .map_err(|e| format!("opening audit log: {e}"))?;
  // Config: missing file = defaults; malformed lines warn on
  // stderr and keep defaults (an agent's server still starts).
  let cfg = match std::fs::read_to_string(&p.config_file) {
    Ok(text) => {
      let (cfg, warnings) = config::parse(&text);
      for w in warnings {
        eprintln!("kumbarium: {w}");
      }
      cfg
    }
    Err(_) => config::Config::default(),
  };
  let state = tools::ServerState {
    library,
    audit,
    agent_id: "unknown-agent".into(),
    cfg,
  };
  Ok((p, state))
}

/// Run backups for both databases if due (or forced). Guarded
/// by the maintenance lock (D-015): if another process holds
/// it, skip quietly; its holder is doing this work.
fn maintenance(
  p: &paths::Paths,
  state: &tools::ServerState,
  force: bool,
) -> Result<Vec<String>, String> {
  let mut report = Vec::new();
  let resource = p.lock_file.with_extension("");
  let lock = kumbarium_util::ProcessLock::try_acquire(&resource)
    .map_err(|e| format!("maintenance lock: {e}"))?;
  if lock.is_none() {
    report.push("maintenance lock held elsewhere; skipping backups".into());
    return Ok(report);
  }
  let cfg = state.cfg;
  let interval_ms = cfg.backup_interval_hours * 3_600_000;
  let jobs = [
    (
      "library",
      &state.library,
      kumbarium_store::Retention {
        recent: cfg.library_recent,
        dailies: cfg.library_dailies,
        weeklies: cfg.library_weeklies,
      },
    ),
    (
      "audit",
      &state.audit,
      kumbarium_store::Retention {
        recent: cfg.audit_recent,
        dailies: cfg.audit_dailies,
        weeklies: cfg.audit_weeklies,
      },
    ),
  ];
  for (name, conn, retention) in jobs {
    let dir = p.backups_dir.join(name);
    let due = force
      || match kumbarium_store::latest_backup_ms(&dir) {
        Some(last) => kumbarium_util::now_ms() - last >= interval_ms,
        None => true,
      };
    if !due {
      report.push(format!("{name}: backup not due"));
      continue;
    }
    let snap = kumbarium_store::backup(conn, &dir)
      .map_err(|e| format!("{name} backup: {e}"))?;
    let removed = kumbarium_store::prune(&dir, retention)
      .map_err(|e| format!("{name} prune: {e}"))?;
    report.push(format!(
      "{name}: snapshot {} ({} pruned)",
      snap.file_name().unwrap_or_default().to_string_lossy(),
      removed.len()
    ));
  }
  Ok(report)
}

fn backup_now() -> ExitCode {
  let (p, state) = match open_stores() {
    Ok(v) => v,
    Err(e) => return fail(&e),
  };
  match maintenance(&p, &state, true) {
    Ok(report) => {
      for line in report {
        println!("{line}");
      }
      ExitCode::SUCCESS
    }
    Err(e) => fail(&e),
  }
}

fn serve() -> ExitCode {
  let (p, mut state) = match open_stores() {
    Ok(v) => v,
    Err(e) => return fail(&e),
  };
  // On-launch backup check (12h-or-elapsed policy); failures
  // are reported but never block serving.
  match maintenance(&p, &state, false) {
    Ok(report) => {
      for line in report {
        eprintln!("kumbarium: {line}");
      }
    }
    Err(e) => eprintln!("kumbarium: {e}"),
  }
  // stdout is protocol-only; say where we are on stderr.
  eprintln!(
    "kumbarium {VERSION} serving MCP on stdio \
     (library: {})",
    p.library_db.display()
  );
  let stdin = std::io::stdin();
  let mut stdout = std::io::stdout();
  match rpc::serve(stdin.lock(), &mut stdout, &mut state) {
    Ok(()) => ExitCode::SUCCESS,
    Err(e) => fail(&format!("transport error: {e}")),
  }
}

fn namespace_add(path: &str, description: &str) -> ExitCode {
  if let Err(e) = kumbarium_librarian::validate_namespace(path) {
    return fail(&format!("invalid namespace {path:?}: {e}"));
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

fn namespace_list() -> ExitCode {
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

fn list_entries(namespace: Option<&str>, all: bool) -> ExitCode {
  let (_, state) = match open_stores() {
    Ok(v) => v,
    Err(e) => return fail(&e),
  };
  let entries =
    match kumbarium_store::entries_in(&state.library, namespace, all) {
      Ok(entries) => entries,
      Err(e) => return fail(&e.to_string()),
    };
  if entries.is_empty() {
    println!("no entries");
    return ExitCode::SUCCESS;
  }
  let sty = style::Style::detect();
  println!(
    "{}",
    sty.dim(
      "id        created     kind          namespace            \
       content"
    )
  );
  // Set-aware order: entries arrive newest-first; the first time
  // a continues-set appears, all its visible members are emitted
  // together in chain order, so a group sits at its newest
  // member's position and parts always read 1..n.
  let first_line = |content: &str| -> String {
    content
      .lines()
      .next()
      .unwrap_or("")
      .chars()
      .take(48)
      .collect()
  };
  let by_id: std::collections::HashMap<&str, &kumbarium_store::Entry> =
    entries.iter().map(|e| (e.id.as_str(), e)).collect();
  let mut emitted: std::collections::HashSet<&str> =
    std::collections::HashSet::new();
  for e in &entries {
    if emitted.contains(e.id.as_str()) {
      continue;
    }
    let chain = kumbarium_store::continues_chain(&state.library, &e.id)
      .map(|(chain, _)| chain)
      .unwrap_or_else(|_| vec![e.id.clone()]);
    let n = chain.len();
    let set_title = if n > 1 {
      kumbarium_store::get(&state.library, &chain[0])
        .map(|head| first_line(&head.content))
        .ok()
    } else {
      None
    };
    for (i, id) in chain.iter().enumerate() {
      let Some(m) = by_id.get(id.as_str()) else {
        continue;
      };
      emitted.insert(m.id.as_str());
      let local = local_display(&m.created_at);
      let day = local.get(..10).unwrap_or(&local);
      let dead = if m.superseded_by.is_some() {
        sty.red(" [superseded]")
      } else if m.retired_at.is_some() {
        sty.yellow(" [retired]")
      } else {
        String::new()
      };
      let part = if n > 1 {
        sty.dim(&format!(" ({}/{n})", i + 1))
      } else {
        String::new()
      };
      let title = set_title.clone().unwrap_or_else(|| first_line(&m.content));
      println!(
        "{}  {day}  {} {:<20} {title}{part}{dead}",
        sty.id(kumbarium_store::short_id(&m.id)),
        sty.kind(&format!("{:<13}", m.kind.as_str())),
        m.namespace
      );
    }
  }
  println!(
    "{}",
    sty.dim(&format!(
      "({} entries; ids are short forms, any unique fragment \
       works)",
      entries.len()
    ))
  );
  ExitCode::SUCCESS
}

fn show_entry(id: &str, full: bool) -> ExitCode {
  let (_, state) = match open_stores() {
    Ok(v) => v,
    Err(e) => return fail(&e),
  };
  let full_id = match kumbarium_store::resolve_id(&state.library, id) {
    Ok(full_id) => full_id,
    Err(err) => return fail(&err.to_string()),
  };
  let e = match kumbarium_store::get(&state.library, &full_id) {
    Ok(e) => e,
    Err(err) => return fail(&err.to_string()),
  };
  let sty = style::Style::detect();
  println!(
    "id:         {} (short: {})",
    e.id,
    sty.id(kumbarium_store::short_id(&e.id))
  );
  println!("namespace:  {}", e.namespace);
  println!("kind:       {}", sty.kind(e.kind.as_str()));
  println!("agent:      {}", e.agent_id);
  if !e.source.is_empty() {
    println!("source:     {}", e.source);
  }
  match &e.confidence_basis {
    Some(basis) => {
      println!("confidence: {:.2} ({basis})", e.confidence)
    }
    None => println!("confidence: {:.2}", e.confidence),
  }
  println!("created:    {}", local_display(&e.created_at));
  println!("updated:    {}", local_display(&e.updated_at));
  if let Some(at) = &e.last_accessed_at {
    println!("accessed:   {}", local_display(at));
  }
  if let Some(at) = &e.last_confirmed_at {
    println!("confirmed:  {}", local_display(at));
  }
  if let Some(at) = &e.retired_at {
    println!("retired:    {}", sty.yellow(&local_display(at)));
  }
  if let Some(note) = &e.note {
    println!("note:       {}", sty.dim(note));
  }
  if !e.tags.is_empty() {
    println!("tags:       {}", e.tags.join(", "));
  }
  if let Some(new) = &e.superseded_by {
    println!("superseded by: {}", sty.yellow(new));
  }
  match kumbarium_store::predecessor_of(&state.library, &full_id) {
    Ok(Some(old)) => println!("supersedes: {old}"),
    Ok(None) => {}
    Err(err) => return fail(&err.to_string()),
  }
  match kumbarium_store::links_of(&state.library, &full_id) {
    Ok(links) => {
      for l in links {
        if l.from_id == e.id {
          println!("link:       {} -> {}", l.rel.as_str(), sty.id(&l.to_id));
        } else {
          println!("link:       {} <- {}", l.rel.as_str(), sty.id(&l.from_id));
        }
      }
    }
    Err(err) => return fail(&err.to_string()),
  }
  let (chain, branched) =
    match kumbarium_store::continues_chain(&state.library, &full_id) {
      Ok(v) => v,
      Err(err) => return fail(&err.to_string()),
    };
  let n = chain.len();
  if n > 1 && !full {
    let pos = chain.iter().position(|c| *c == full_id).unwrap_or(0) + 1;
    let note = if branched { "; chain BRANCHES" } else { "" };
    println!(
      "set:        {}",
      sty.yellow(&format!("part {pos} of {n}{note} (--full stitches)"))
    );
  }
  if full && n > 1 {
    if branched {
      println!(
        "{}",
        sty.yellow("warning: continues chain branches; showing mint order")
      );
    }
    for (i, part_id) in chain.iter().enumerate() {
      let part = match kumbarium_store::get(&state.library, part_id) {
        Ok(part) => part,
        Err(err) => return fail(&err.to_string()),
      };
      println!(
        "\n{}\n{}",
        sty.bold(&format!(
          "-- part {}/{n}  {} --",
          i + 1,
          kumbarium_store::short_id(part_id)
        )),
        markdown::render(&part.content, &sty)
      );
    }
  } else {
    println!("\n{}", markdown::render(&e.content, &sty));
  }
  ExitCode::SUCCESS
}

fn audit_tail(n: usize, scope: Option<&str>) -> ExitCode {
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
          "at (local)           kind      agent                \
scope                detail"
        )
      );
      // Columns before detail: 19+2 + 9+1 + 20+1 + 20+1 = 73.
      const DETAIL_COL: usize = 73;
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
          sty.event(&format!("{:<9}", e.kind)),
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

fn audit_export(to_stdout: bool, raw: bool) -> ExitCode {
  let (p, state) = match open_stores() {
    Ok(v) => v,
    Err(e) => return fail(&e),
  };
  let events = match kumbarium_audit::events_asc(&state.audit) {
    Ok(events) => events,
    Err(e) => return fail(&e.to_string()),
  };
  // --raw keeps the STORED form: UTC, machine-comparable
  // across exporting machines. Default is local time.
  let utc_display = |at: &str| -> String {
    let day = at.get(..10).unwrap_or(at);
    let time = at.get(11..19).unwrap_or("");
    format!("{day} {time}")
  };
  let minutes = if raw {
    kumbarium_audit::render_minutes(
      &events,
      &utc_display,
      "All times UTC (as stored).",
    )
  } else {
    kumbarium_audit::render_minutes(
      &events,
      &local_display,
      "Times are local to the exporting machine.",
    )
  };
  if to_stdout {
    // On a TTY, hanging-wrap table rows at the detail column
    // (8+2 + 9+1 + 20+1 + 20+1 = 62) so overflow stays
    // readable; piped/redirected output is byte-identical to
    // the file artifact.
    const EXPORT_DETAIL_COL: usize = 62;
    let width = term_width().filter(|w| *w > EXPORT_DETAIL_COL + 16);
    let Some(width) = width else {
      print!("{minutes}");
      return ExitCode::SUCCESS;
    };
    let mut in_fence = false;
    for line in minutes.lines() {
      if line.starts_with("```") {
        in_fence = !in_fence;
        println!("{line}");
        continue;
      }
      let wrappable = in_fence
        && line.len() > width
        && line.is_char_boundary(EXPORT_DETAIL_COL);
      if !wrappable {
        println!("{line}");
        continue;
      }
      let (prefix, rest) = line.split_at(EXPORT_DETAIL_COL);
      let chunks = wrap_words(rest, width - EXPORT_DETAIL_COL);
      println!(
        "{prefix}{}",
        chunks.first().map(String::as_str).unwrap_or("")
      );
      for chunk in chunks.iter().skip(1) {
        println!("{:EXPORT_DETAIL_COL$}{chunk}", "");
      }
    }
    return ExitCode::SUCCESS;
  }
  if let Err(e) = std::fs::create_dir_all(&p.exports_dir) {
    return fail(&format!("creating exports dir: {e}"));
  }
  let stamp = kumbarium_util::now_iso8601()
    .get(..19)
    .unwrap_or_default()
    .replace(':', "-");
  let target = p.exports_dir.join(format!("minutes-{stamp}Z.md"));
  match kumbarium_util::write_atomically(&target, minutes.as_bytes()) {
    Ok(()) => {
      println!("{}", shell_quote(&target.display().to_string()));
      ExitCode::SUCCESS
    }
    Err(e) => fail(&format!("writing minutes: {e}")),
  }
}

/// Quote a path for copy-paste when a HUMAN is reading (the
/// macOS data dir contains a space) but print it bare into a
/// pipe or command substitution, where literal quotes would
/// corrupt the path. Same tty rule the color system uses.
fn shell_quote(path: &str) -> String {
  use std::io::IsTerminal;
  let plain = path
    .bytes()
    .all(|b| b.is_ascii_alphanumeric() || b"/._-+:@%".contains(&b));
  if plain || !std::io::stdout().is_terminal() {
    path.to_string()
  } else {
    format!("'{}'", path.replace('\'', "'\\''"))
  }
}

/// Retire (or restore) an entry: human-only lifecycle verb,
/// immediate because fully reversible; audited either way.
fn retire_cmd(id: &str, retiring: bool) -> ExitCode {
  let (_, state) = match open_stores() {
    Ok(v) => v,
    Err(e) => return fail(&e),
  };
  let sty = style::Style::detect();
  let full = match kumbarium_store::resolve_id(&state.library, id) {
    Ok(f) => f,
    Err(e) => return fail(&e.to_string()),
  };
  let result = if retiring {
    kumbarium_store::retire(&state.library, &full)
  } else {
    kumbarium_store::unretire(&state.library, &full)
  };
  if let Err(e) = result {
    return fail(&e.to_string());
  }
  let entry = match kumbarium_store::get(&state.library, &full) {
    Ok(e) => e,
    Err(e) => return fail(&e.to_string()),
  };
  let kind = if retiring {
    kumbarium_audit::EventKind::Retire
  } else {
    kumbarium_audit::EventKind::Unretire
  };
  let event = kumbarium_audit::Event {
    agent_id: "kumbarium-cli".into(),
    kind,
    scope: entry.namespace.clone(),
    detail: serde_json::json!({ "id": full }),
  };
  if let Err(e) = kumbarium_audit::append(&state.audit, &event) {
    return fail(&format!("done, but audit append failed: {e}"));
  }
  let short = kumbarium_store::short_id(&full);
  if retiring {
    println!(
      "retired {} (kept in history; `kum unretire {short}` \
       restores)",
      sty.id(short)
    );
  } else {
    println!("restored {} to suggestions", sty.id(short));
  }
  ExitCode::SUCCESS
}

fn history_cmd(id: &str, with_diff: bool, all: bool) -> ExitCode {
  let (_, state) = match open_stores() {
    Ok(v) => v,
    Err(e) => return fail(&e),
  };
  let sty = style::Style::detect();
  let versions = match resolve_history(&state, id) {
    Ok(v) => v,
    Err(e) => return fail(&e),
  };
  let n = versions.len();
  // Collapse-eligible: noted AND measurably small vs its
  // predecessor. The diff decides; the note only informs.
  let changed: Vec<usize> = versions
    .iter()
    .enumerate()
    .map(|(i, e)| {
      if i == 0 {
        usize::MAX
      } else {
        diff::lines(&versions[i - 1].content, &e.content)
          .iter()
          .filter(|(c, _)| *c != ' ')
          .count()
      }
    })
    .collect();
  let collapsed = |i: usize| -> bool {
    // The live head never collapses: the current truth is
    // always shown in full.
    !all
      && i + 1 != n
      && versions[i].note.is_some()
      // The note informs, the diff decides (config:
      // history.collapse_max_changed_lines).
      && changed[i] <= state.cfg.collapse_max_changed_lines
  };
  println!(
    "{}",
    sty.dim(
      "version    id        created     agent                 \
       bytes"
    )
  );
  let mut hidden = 0usize;
  for (i, e) in versions.iter().enumerate().rev() {
    if collapsed(i) {
      hidden += 1;
      println!(
        "{}",
        sty.dim(&format!(
          "v{:<2}        {}  {:?} ({} lines changed)",
          i + 1,
          kumbarium_store::short_id(&e.id),
          e.note.as_deref().unwrap_or(""),
          changed[i]
        ))
      );
      continue;
    }
    let live = if i + 1 == n { " (live)" } else { "" };
    let ver = format!("v{}{live}", i + 1);
    let local = local_display(&e.created_at);
    let day = local.get(..10).unwrap_or(&local);
    let note = match &e.note {
      Some(note) => sty.dim(&format!("  {note:?}")),
      None => String::new(),
    };
    println!(
      "{ver:<11}{}  {day}  {:<20}  {}{note}",
      sty.id(kumbarium_store::short_id(&e.id)),
      e.agent_id,
      e.content.len()
    );
  }
  if hidden > 0 {
    println!(
      "{}",
      sty.dim(&format!(
        "({hidden} noted small version(s) collapsed; --all \
         expands)"
      ))
    );
  }
  if with_diff {
    for (pair_i, pair) in versions.windows(2).enumerate() {
      if collapsed(pair_i + 1) {
        continue;
      }
      println!(
        "\n{}",
        sty.bold(&format!(
          "-- v{} -> v{} --",
          version_of(&versions, &pair[0].id),
          version_of(&versions, &pair[1].id)
        ))
      );
      print_diff(&pair[0].content, &pair[1].content, &sty);
    }
  }
  ExitCode::SUCCESS
}

fn revert_cmd(id: &str, apply: bool) -> ExitCode {
  let (_, mut state) = match open_stores() {
    Ok(v) => v,
    Err(e) => return fail(&e),
  };
  let sty = style::Style::detect();
  let versions = match resolve_history(&state, id) {
    Ok(v) => v,
    Err(e) => return fail(&e),
  };
  let target_full = match kumbarium_store::resolve_id(&state.library, id) {
    Ok(f) => f,
    Err(e) => return fail(&e.to_string()),
  };
  let head = versions.last().expect("history never empty").clone();
  let Some(target) = versions.iter().find(|e| e.id == target_full).cloned()
  else {
    return fail("target version not found in history");
  };
  if target.id == head.id {
    return fail(&format!(
      "{} is already the live version; pick an ancestor \
       (see: kumbarium history {})",
      kumbarium_store::short_id(&target.id),
      kumbarium_store::short_id(&target.id)
    ));
  }
  println!(
    "revert plan: supersede live {} with the content of {} \
     (v{} of {})",
    sty.id(kumbarium_store::short_id(&head.id)),
    sty.id(kumbarium_store::short_id(&target.id)),
    version_of(&versions, &target.id),
    versions.len()
  );
  print_diff(&head.content, &target.content, &sty);
  if !apply {
    println!(
      "\n{}",
      sty.yellow(
        "preview only: nothing written; re-run with --apply \
         to sign off"
      )
    );
    return ExitCode::SUCCESS;
  }
  let new = kumbarium_store::NewEntry {
    namespace: target.namespace.clone(),
    kind: target.kind,
    content: target.content.clone(),
    agent_id: "kumbarium-cli".into(),
    source: target.source.clone(),
    tags: target.tags.clone(),
  };
  let revert_note =
    format!("revert to {}", kumbarium_store::short_id(&target.id));
  let ids = match tools::store_split(
    &mut state,
    &new,
    Some(&head.id),
    Some(&revert_note),
  ) {
    Ok(ids) => ids,
    Err(e) => return fail(&e),
  };
  let event = kumbarium_audit::Event {
    agent_id: "kumbarium-cli".into(),
    kind: kumbarium_audit::EventKind::Supersede,
    scope: target.namespace.clone(),
    detail: serde_json::json!({
      "old_id": head.id,
      "new_id": ids[0],
      "revert_to": target.id,
      "parts": ids.len(),
      "note": revert_note,
    }),
  };
  if let Err(e) = kumbarium_audit::append(&state.audit, &event) {
    return fail(&format!("reverted, but audit append failed: {e}"));
  }
  println!(
    "\nreverted: {} superseded by {} ({} part(s))",
    sty.id(kumbarium_store::short_id(&head.id)),
    sty.id(kumbarium_store::short_id(&ids[0])),
    ids.len()
  );
  ExitCode::SUCCESS
}

/// Full entries for a fact's version chain, oldest first.
fn resolve_history(
  state: &tools::ServerState,
  id: &str,
) -> Result<Vec<kumbarium_store::Entry>, String> {
  let full = kumbarium_store::resolve_id(&state.library, id)
    .map_err(|e| e.to_string())?;
  let ids = kumbarium_store::version_history(&state.library, &full)
    .map_err(|e| e.to_string())?;
  ids
    .iter()
    .map(|v| kumbarium_store::get(&state.library, v).map_err(|e| e.to_string()))
    .collect()
}

fn version_of(versions: &[kumbarium_store::Entry], id: &str) -> usize {
  versions.iter().position(|e| e.id == id).unwrap_or(0) + 1
}

fn print_diff(old: &str, new: &str, sty: &style::Style) {
  for (mark, line) in diff::lines(old, new) {
    match mark {
      '-' => println!("{}", sty.red(&format!("- {line}"))),
      '+' => println!("{}", sty.green(&format!("+ {line}"))),
      _ => println!("  {}", sty.dim(&line)),
    }
  }
}

fn import_claude(rest: &[&str]) -> ExitCode {
  let mut opts = import::Options {
    dirs: Vec::new(),
    apply: false,
    map: Vec::new(),
  };
  let mut it = rest.iter();
  while let Some(arg) = it.next() {
    match *arg {
      "--apply" => opts.apply = true,
      "--dir" => match it.next() {
        Some(d) => opts.dirs.push(d.into()),
        None => return fail("--dir needs a path"),
      },
      "--map" => match it.next().and_then(|m| m.split_once('=')) {
        Some((name, ns)) => {
          opts.map.push((name.to_string(), ns.to_string()));
        }
        None => return fail("--map needs name=namespace"),
      },
      other => {
        return fail(&format!("unknown import flag {other:?}"));
      }
    }
  }
  if opts.dirs.is_empty() {
    opts.dirs = import::default_dirs();
  }
  if opts.dirs.is_empty() {
    return fail("no Claude memory dirs found; pass --dir <path>");
  }
  let (_, mut state) = match open_stores() {
    Ok(v) => v,
    Err(e) => return fail(&e),
  };
  match import::run(&mut state, &opts) {
    Ok(report) => {
      for line in report {
        println!("{line}");
      }
      ExitCode::SUCCESS
    }
    Err(e) => fail(&e),
  }
}

/// Render a stored UTC timestamp in the machine's local time
/// for interactive display. Storage stays strict UTC (D-005);
/// this is presentation only. Non-unix or unparseable input
/// passes through unchanged.
fn local_display(iso_utc: &str) -> String {
  #[cfg(unix)]
  {
    let Some(ms) = kumbarium_util::parse_iso8601_ms(iso_utc) else {
      return iso_utc.to_string();
    };
    let secs = ms.div_euclid(1000) as libc::time_t;
    let mut tm: libc::tm = unsafe { std::mem::zeroed() };
    let ok = unsafe { !libc::localtime_r(&secs, &mut tm).is_null() };
    if !ok {
      return iso_utc.to_string();
    }
    format!(
      "{:04}-{:02}-{:02} {:02}:{:02}:{:02}",
      tm.tm_year + 1900,
      tm.tm_mon + 1,
      tm.tm_mday,
      tm.tm_hour,
      tm.tm_min,
      tm.tm_sec
    )
  }
  #[cfg(not(unix))]
  {
    iso_utc.to_string()
  }
}

/// The terminal's column count, when stdout is a terminal.
fn term_width() -> Option<usize> {
  #[cfg(unix)]
  {
    use std::io::IsTerminal;
    if !std::io::stdout().is_terminal() {
      return None;
    }
    let mut ws: libc::winsize = unsafe { std::mem::zeroed() };
    let ok =
      unsafe { libc::ioctl(libc::STDOUT_FILENO, libc::TIOCGWINSZ, &mut ws) }
        == 0;
    (ok && ws.ws_col > 0).then_some(ws.ws_col as usize)
  }
  #[cfg(not(unix))]
  {
    None
  }
}

/// Word-wrap `text` to `width` columns; a word longer than the
/// width is hard-cut rather than overflowing.
fn wrap_words(text: &str, width: usize) -> Vec<String> {
  let mut lines = Vec::new();
  let mut current = String::new();
  for word in text.split_whitespace() {
    let mut word = word;
    while word.len() > width {
      if !current.is_empty() {
        lines.push(std::mem::take(&mut current));
      }
      let (head, rest) = word.split_at(width);
      lines.push(head.to_string());
      word = rest;
    }
    let sep = if current.is_empty() { 0 } else { 1 };
    if !current.is_empty() && current.len() + sep + word.len() > width {
      lines.push(std::mem::take(&mut current));
    }
    if !current.is_empty() {
      current.push(' ');
    }
    current.push_str(word);
  }
  if !current.is_empty() {
    lines.push(current);
  }
  if lines.is_empty() {
    lines.push(String::new());
  }
  lines
}

fn status_cmd() -> ExitCode {
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
  println!(
    "  sets:      {} ({} parts)",
    stats.set_heads, stats.set_parts
  );
  match kumbarium_store::namespaces(&state.library) {
    Ok(rows) => {
      for (path, _, _) in rows {
        let n: i64 = state
          .library
          .query_row(
            "SELECT count(*) FROM entries e
             JOIN namespaces ns ON ns.id = e.namespace_id
             WHERE ns.path = ?1 AND e.superseded_by IS NULL
               AND e.retired_at IS NULL",
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
  for (name, dir) in [
    ("library", p.backups_dir.join("library")),
    ("audit", p.backups_dir.join("audit")),
  ] {
    let line = match kumbarium_store::latest_backup_ms(&dir) {
      Some(ms) => {
        let age_h = (kumbarium_util::now_ms() - ms).max(0) / 3_600_000;
        format!("last backup {age_h}h ago")
      }
      None => "no backups yet".into(),
    };
    println!("  {name:<10} {line}");
  }
  for (name, path) in [("library.db", &p.library_db), ("audit.db", &p.audit_db)]
  {
    if let Ok(meta) = std::fs::metadata(path) {
      println!("  {name:<10} {} KB", meta.len() / 1024);
    }
  }
  ExitCode::SUCCESS
}

/// rg-flavored literal search: smart-case, exhaustive (--all
/// includes superseded/retired), grouped headings on a tty and
/// `id:line:text` when piped. Deliberately NOT recall: recall
/// ranks live memories for agents; grep finds every occurrence
/// for forensics.
fn grep_cmd(pattern: &str, namespace: Option<&str>, all: bool) -> ExitCode {
  let (_, state) = match open_stores() {
    Ok(v) => v,
    Err(e) => return fail(&e),
  };
  let sty = style::Style::detect();
  let entries =
    match kumbarium_store::entries_in(&state.library, namespace, all) {
      Ok(entries) => entries,
      Err(e) => return fail(&e.to_string()),
    };
  // Smart-case, rg-style: all-lowercase pattern matches
  // case-insensitively; any uppercase makes it exact.
  let sensitive = pattern.chars().any(|c| c.is_uppercase());
  let needle = if sensitive {
    pattern.to_string()
  } else {
    pattern.to_lowercase()
  };
  let mut hits = 0usize;
  for e in &entries {
    let mut first = true;
    for (lineno, line) in e.content.lines().enumerate() {
      let hay = if sensitive {
        line.to_string()
      } else {
        line.to_lowercase()
      };
      if !hay.contains(&needle) {
        continue;
      }
      hits += 1;
      if sty.on {
        if first {
          first = false;
          let mark = if e.superseded_by.is_some() {
            " [superseded]"
          } else if e.retired_at.is_some() {
            " [retired]"
          } else {
            ""
          };
          println!(
            "{}  {}{}",
            sty.id(kumbarium_store::short_id(&e.id)),
            e.namespace,
            sty.yellow(mark)
          );
        }
        println!(
          "{}: {}",
          sty.dim(&format!("{:>4}", lineno + 1)),
          highlight(line, &needle, sensitive, &sty)
        );
      } else {
        println!("{}:{}:{line}", kumbarium_store::short_id(&e.id), lineno + 1);
      }
    }
    if !first && sty.on {
      println!();
    }
  }
  if hits == 0 {
    eprintln!("no matches");
    return ExitCode::FAILURE;
  }
  ExitCode::SUCCESS
}

/// Paint every occurrence of the needle in a line, rg-style.
fn highlight(
  line: &str,
  needle: &str,
  sensitive: bool,
  sty: &style::Style,
) -> String {
  let hay = if sensitive {
    line.to_string()
  } else {
    line.to_lowercase()
  };
  let mut out = String::new();
  let mut pos = 0;
  while let Some(found) = hay[pos..].find(needle) {
    let start = pos + found;
    let end = start + needle.len();
    if !line.is_char_boundary(start) || !line.is_char_boundary(end) {
      break;
    }
    out.push_str(&line[pos..start]);
    out.push_str(&sty.bold(&sty.red(&line[start..end])));
    pos = end;
  }
  out.push_str(&line[pos..]);
  out
}

/// Move a memory to another namespace: a supersession into the
/// target with an auto-note, so history records the move rather
/// than anything mutating in place.
fn move_cmd(id: &str, namespace: &str) -> ExitCode {
  if let Err(e) = kumbarium_librarian::validate_namespace(namespace) {
    return fail(&format!("invalid namespace: {e}"));
  }
  let (_, mut state) = match open_stores() {
    Ok(v) => v,
    Err(e) => return fail(&e),
  };
  let sty = style::Style::detect();
  let full = match kumbarium_store::resolve_id(&state.library, id) {
    Ok(f) => f,
    Err(e) => return fail(&e.to_string()),
  };
  let e = match kumbarium_store::get(&state.library, &full) {
    Ok(e) => e,
    Err(err) => return fail(&err.to_string()),
  };
  if e.namespace == namespace {
    return fail("entry is already in that namespace");
  }
  let note = format!("moved from {}", e.namespace);
  let new = kumbarium_store::NewEntry {
    namespace: namespace.to_string(),
    kind: e.kind,
    content: e.content.clone(),
    agent_id: "kumbarium-cli".into(),
    source: e.source.clone(),
    tags: e.tags.clone(),
  };
  let ids = match tools::store_split(&mut state, &new, Some(&full), Some(&note))
  {
    Ok(ids) => ids,
    Err(err) => return fail(&err),
  };
  let event = kumbarium_audit::Event {
    agent_id: "kumbarium-cli".into(),
    kind: kumbarium_audit::EventKind::Supersede,
    scope: namespace.to_string(),
    detail: serde_json::json!({
      "old_id": full,
      "new_id": ids[0],
      "note": note,
    }),
  };
  if let Err(err) = kumbarium_audit::append(&state.audit, &event) {
    return fail(&format!("moved, but audit append failed: {err}"));
  }
  println!(
    "moved {} -> {} as {}",
    sty.id(kumbarium_store::short_id(&full)),
    namespace,
    sty.id(kumbarium_store::short_id(&ids[0]))
  );
  ExitCode::SUCCESS
}

fn config_cmd(init: bool) -> ExitCode {
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
  ExitCode::SUCCESS
}

/// Record confirmation evidence from the CLI (same semantics
/// as the MCP tool: stamps last_confirmed_at, never touches the
/// confidence number; the janitor judges that later).
fn confirm_cmd(id: &str) -> ExitCode {
  let (_, state) = match open_stores() {
    Ok(v) => v,
    Err(e) => return fail(&e),
  };
  let sty = style::Style::detect();
  let full = match kumbarium_store::resolve_id(&state.library, id) {
    Ok(f) => f,
    Err(e) => return fail(&e.to_string()),
  };
  if let Err(e) = kumbarium_store::confirm(&state.library, &full) {
    return fail(&e.to_string());
  }
  let scope = kumbarium_store::get(&state.library, &full)
    .map(|e| e.namespace)
    .unwrap_or_default();
  let event = kumbarium_audit::Event {
    agent_id: "kumbarium-cli".into(),
    kind: kumbarium_audit::EventKind::Confirm,
    scope,
    detail: serde_json::json!({ "id": full }),
  };
  if let Err(e) = kumbarium_audit::append(&state.audit, &event) {
    return fail(&format!("confirmed, but audit append failed: {e}"));
  }
  println!("confirmed {}", sty.id(kumbarium_store::short_id(&full)));
  ExitCode::SUCCESS
}

/// The confidence pass (D-025): recompute every live entry from
/// the full ledger, preview the proposals, apply only on the
/// --apply sign-off. One batch janitor event witnesses the run.
fn janitor_cmd(apply: bool) -> ExitCode {
  let (_, state) = match open_stores() {
    Ok(v) => v,
    Err(e) => return fail(&e),
  };
  let sty = style::Style::detect();
  let events = match kumbarium_audit::events_asc(&state.audit) {
    Ok(v) => v,
    Err(e) => return fail(&e.to_string()),
  };
  let report = match kumbarium_janitor::pass(
    &state.library,
    &events,
    state.cfg.janitor_dormant_days,
    kumbarium_util::now_ms(),
  ) {
    Ok(r) => r,
    Err(e) => return fail(&e.to_string()),
  };
  if report.proposals.is_empty() && report.dormant.is_empty() {
    println!("janitor: no changes proposed; evidence unchanged");
    return ExitCode::SUCCESS;
  }
  if !report.proposals.is_empty() {
    println!(
      "{}",
      sty.dim("id        namespace            old      new   basis")
    );
    for p in &report.proposals {
      println!(
        "{}  {:<20} {:.2} -> {:.2}  {}",
        sty.id(&format!("{:<8}", kumbarium_store::short_id(&p.id))),
        p.namespace,
        p.old,
        p.new,
        p.basis
      );
    }
  }
  if !report.dormant.is_empty() {
    println!(
      "\ndormant ({}+ days old, never recalled); retire is \
       your call:",
      state.cfg.janitor_dormant_days
    );
    for d in &report.dormant {
      println!(
        "  {}  {:<20} {} days old",
        sty.id(&format!("{:<8}", kumbarium_store::short_id(&d.id))),
        d.namespace,
        d.age_days
      );
    }
  }
  if !apply {
    println!(
      "\n{}",
      sty.yellow(
        "preview only: nothing written; re-run with --apply \
         to sign off"
      )
    );
    return ExitCode::SUCCESS;
  }
  for p in &report.proposals {
    if let Err(e) =
      kumbarium_store::set_confidence(&state.library, &p.id, p.new, &p.basis)
    {
      return fail(&format!("applying {}: {e}", p.id));
    }
  }
  let applied: Vec<serde_json::Value> = report
    .proposals
    .iter()
    .map(|p| serde_json::json!({ "id": p.id, "from": p.old, "to": p.new }))
    .collect();
  let event = kumbarium_audit::Event {
    agent_id: "kumbarium-cli".into(),
    kind: kumbarium_audit::EventKind::Janitor,
    scope: String::new(),
    detail: serde_json::json!({
      "changed": report.proposals.len(),
      "dormant": report.dormant.len(),
      "dormant_days": state.cfg.janitor_dormant_days,
      "applied": applied,
    }),
  };
  if let Err(e) = kumbarium_audit::append(&state.audit, &event) {
    return fail(&format!("applied, but audit append failed: {e}"));
  }
  println!(
    "\napplied {} confidence change(s); {} dormant flagged",
    report.proposals.len(),
    report.dormant.len()
  );
  ExitCode::SUCCESS
}

fn fail(message: &str) -> ExitCode {
  eprintln!("kumbarium: {message}");
  ExitCode::FAILURE
}

const USAGE: &str = "\
kumbarium: the place of remembering

Usage:
  kumbarium serve                     speak MCP over stdio
  kumbarium namespace add <path> [d]  register a namespace
  kumbarium namespace list            list namespaces
  kumbarium import claude [--apply]   import Claude Code
      [--dir <path>]... [--map name=namespace]...  memories
  kumbarium list [ns] [--all]         browse entries
  kumbarium show <id> [--full]        one entry (--full stitches
                                      a split set in order)
  kumbarium history <id> [--diff]     a fact's version chain
                     [--all]          (--all expands collapsed
                                      noted-small versions)
  kumbarium confirm <id>              record a fact proved true
  kumbarium janitor [--apply]         confidence pass over the
                                      ledger (preview until the
                                      --apply sign-off; CLI
                                      only, agents cannot)
  kumbarium retire <id>               hide from suggestions
  kumbarium unretire <id>             restore to suggestions
  kumbarium revert <id> [--apply]     restore an old version
                                      (preview only until the
                                      --apply sign-off; CLI
                                      only, agents cannot)
  kumbarium status                    library health at a glance
  kumbarium grep <pat> [ns] [--all]   literal search, rg-style
  kumbarium move <id> <namespace>     relocate (as supersession)
  kumbarium audit tail [n]            recent audit events
             [--scope <ns>]           (optionally one scope)
  kumbarium audit export [--stdout]   minutes markdown to
                         [--raw]      exports/ or streamed
                                      (--raw keeps stored UTC)
  kumbarium backup                    snapshot both dbs now
  kumbarium config [--init]           effective tunables
                                      (--init writes template)
  kumbarium paths                     where persisted data lives
  kumbarium version                   print the version
  kumbarium help [topic]              manual pages with grammar
                                      and examples
  kumbarium instructions [--snippet]  agent setup: MCP
                                      registration + root-file
                                      instruction block";
