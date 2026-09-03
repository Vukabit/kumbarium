//! Kumbarium: the librarian process. `serve` speaks MCP over
//! stdio (D-014); the rest is the human-facing CLI.

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
    ["audit", "tail"] => audit_tail(20),
    ["audit", "tail", n] => match n.parse() {
      Ok(n) => audit_tail(n),
      Err(_) => fail("audit tail takes a number"),
    },
    ["audit", "export"] => audit_export(),
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
      println!("{}", sty.dim("namespace  [created]  description"));
      for (path, description, created_at) in rows {
        let day = created_at.get(..10).unwrap_or(&created_at);
        println!("{}  [{day}]  {description}", sty.bold(&path));
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
  for e in &entries {
    let day = e.created_at.get(..10).unwrap_or(&e.created_at);
    let dead = if e.superseded_by.is_some() {
      sty.red(" [superseded]")
    } else {
      String::new()
    };
    let preview: String = e
      .content
      .lines()
      .next()
      .unwrap_or("")
      .chars()
      .take(56)
      .collect();
    println!(
      "{}  {day}  {} {:<20} {preview}{dead}",
      sty.id(kumbarium_store::short_id(&e.id)),
      sty.kind(&format!("{:<13}", e.kind.as_str())),
      e.namespace
    );
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
  println!("created:    {}", e.created_at);
  println!("updated:    {}", e.updated_at);
  if let Some(at) = &e.last_accessed_at {
    println!("accessed:   {at}");
  }
  if let Some(at) = &e.last_confirmed_at {
    println!("confirmed:  {at}");
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
          "at                        kind      agent              \
           scope  detail"
        )
      );
      for e in events {
        let scope = if e.scope.is_empty() {
          String::new()
        } else {
          format!(" {}", e.scope)
        };
        println!(
          "{}  {} {:<20}{scope}  {}",
          sty.dim(&e.at),
          sty.event(&format!("{:<9}", e.kind)),
          e.agent_id,
          e.detail
        );
      }
      ExitCode::SUCCESS
    }
    Err(e) => fail(&e.to_string()),
  }
}

fn audit_export() -> ExitCode {
  let (p, state) = match open_stores() {
    Ok(v) => v,
    Err(e) => return fail(&e),
  };
  let events = match kumbarium_audit::events_asc(&state.audit) {
    Ok(events) => events,
    Err(e) => return fail(&e.to_string()),
  };
  let minutes = kumbarium_audit::render_minutes(&events);
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
      println!("{}", target.display());
      ExitCode::SUCCESS
    }
    Err(e) => fail(&format!("writing minutes: {e}")),
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
  kumbarium audit tail [n]            recent audit events
  kumbarium audit export              minutes markdown to exports/
  kumbarium backup                    snapshot both dbs now
  kumbarium paths                     where persisted data lives
  kumbarium version                   print the version";
