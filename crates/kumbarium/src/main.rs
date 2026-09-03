//! Kumbarium: the librarian process. `serve` speaks MCP over
//! stdio (D-014); the rest is the human-facing CLI.

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
    ["history", id] => history_cmd(id, false),
    ["history", id, "--diff"] => history_cmd(id, true),
    ["revert", id] => revert_cmd(id, false),
    ["revert", id, "--apply"] => revert_cmd(id, true),
    ["retire", id] => retire_cmd(id, true),
    ["unretire", id] => retire_cmd(id, false),
    ["audit", "tail"] => audit_tail(20),
    ["audit", "tail", n] => match n.parse() {
      Ok(n) => audit_tail(n),
      Err(_) => fail("audit tail takes a number"),
    },
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
    ["audit", "export"] => audit_export(false),
    ["audit", "export", "--stdout"] => audit_export(true),
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
  let state = tools::ServerState {
    library,
    audit,
    agent_id: "unknown-agent".into(),
  };
  Ok((p, state))
}

/// Backup policy (mechanics live in kumbarium-store). Audit
/// keeps a shallower tier: higher volume, lower stakes.
const BACKUP_INTERVAL_MS: i64 = 12 * 3_600_000;
const LIBRARY_RETENTION: kumbarium_store::Retention =
  kumbarium_store::Retention {
    recent: 2,
    dailies: 7,
    weeklies: 4,
  };
const AUDIT_RETENTION: kumbarium_store::Retention =
  kumbarium_store::Retention {
    recent: 2,
    dailies: 3,
    weeklies: 0,
  };

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
  let jobs = [
    ("library", &state.library, LIBRARY_RETENTION),
    ("audit", &state.audit, AUDIT_RETENTION),
  ];
  for (name, conn, retention) in jobs {
    let dir = p.backups_dir.join(name);
    let due = force
      || match kumbarium_store::latest_backup_ms(&dir) {
        Some(last) => kumbarium_util::now_ms() - last >= BACKUP_INTERVAL_MS,
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
  println!("confidence: {:.2}", e.confidence);
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

fn audit_tail(n: usize) -> ExitCode {
  let (_, state) = match open_stores() {
    Ok(v) => v,
    Err(e) => return fail(&e),
  };
  match kumbarium_audit::tail(&state.audit, n) {
    Ok(events) => {
      let sty = style::Style::detect();
      println!(
        "{}",
        sty.dim(
          "at (local)           kind      agent                \
scope                detail"
        )
      );
      for e in events {
        println!(
          "{}  {} {:<20} {:<20} {}",
          sty.dim(&local_display(&e.at)),
          sty.event(&format!("{:<9}", e.kind)),
          e.agent_id,
          e.scope,
          kumbarium_audit::describe_event(&e.kind, &e.detail)
        );
      }
      ExitCode::SUCCESS
    }
    Err(e) => fail(&e.to_string()),
  }
}

fn audit_export(to_stdout: bool) -> ExitCode {
  let (p, state) = match open_stores() {
    Ok(v) => v,
    Err(e) => return fail(&e),
  };
  let events = match kumbarium_audit::events_asc(&state.audit) {
    Ok(events) => events,
    Err(e) => return fail(&e.to_string()),
  };
  let minutes = kumbarium_audit::render_minutes(&events);
  if to_stdout {
    print!("{minutes}");
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

fn history_cmd(id: &str, with_diff: bool) -> ExitCode {
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
  println!(
    "{}",
    sty.dim(
      "version    id        created     agent                 \
       bytes"
    )
  );
  for (i, e) in versions.iter().enumerate().rev() {
    let live = if i + 1 == n { " (live)" } else { "" };
    let ver = format!("v{}{live}", i + 1);
    let local = local_display(&e.created_at);
    let day = local.get(..10).unwrap_or(&local);
    println!(
      "{ver:<11}{}  {day}  {:<20}  {}",
      sty.id(kumbarium_store::short_id(&e.id)),
      e.agent_id,
      e.content.len()
    );
  }
  if with_diff {
    for pair in versions.windows(2) {
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
  let ids = match tools::store_split(&mut state, &new, Some(&head.id)) {
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
  kumbarium retire <id>               hide from suggestions
  kumbarium unretire <id>             restore to suggestions
  kumbarium revert <id> [--apply]     restore an old version
                                      (preview only until the
                                      --apply sign-off; CLI
                                      only, agents cannot)
  kumbarium audit tail [n]            recent audit events
  kumbarium audit export [--stdout]   minutes markdown to
                                      exports/ (or streamed)
  kumbarium backup                    snapshot both dbs now
  kumbarium paths                     where persisted data lives
  kumbarium version                   print the version
  kumbarium help [topic]              manual pages with grammar
                                      and examples";
