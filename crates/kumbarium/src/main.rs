//! Kumbarium: the librarian process. `serve` speaks MCP over
//! stdio (D-014); the rest is the human-facing CLI.

mod import;
mod paths;
mod rpc;
mod tools;

use std::process::ExitCode;

const VERSION: &str = env!("CARGO_PKG_VERSION");

fn main() -> ExitCode {
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
      for (path, description, created_at) in rows {
        let day = created_at.get(..10).unwrap_or(&created_at);
        println!("{path}  [{day}]  {description}");
      }
      ExitCode::SUCCESS
    }
    Err(e) => fail(&e.to_string()),
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
  kumbarium backup                    snapshot both dbs now
  kumbarium paths                     where persisted data lives
  kumbarium version                   print the version";
