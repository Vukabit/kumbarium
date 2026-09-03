//! Kumbarium: the librarian process. `serve` speaks MCP over
//! stdio (D-014); the rest is the human-facing CLI.

mod bundle;
mod cli;
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

use cli::admin::*;
use cli::desk::*;
use cli::dock::*;
use cli::docket::*;
use cli::entries::*;
use cli::term::*;
use cli::usage::*;

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
    ["version"] => version_cmd(),
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
    ["namespace", "list"] | ["namespace"] => namespace_list(),
    ["import", "claude", rest @ ..] => import_claude(rest),
    ["export"] => {
      let sty = style::Style::detect();
      println!("{}", paint_cli_page(EXPORTS, &sty));
      ExitCode::SUCCESS
    }
    ["export", "minutes", rest @ ..] => export_minutes_cmd(rest),
    ["export", "bundle", scope, rest @ ..] => export_bundle_cmd(scope, rest),
    ["import", "bundle", file] => import_bundle_cmd(file, false),
    ["import", "bundle", file, "--pending"] => import_bundle_cmd(file, true),
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
    ["task", "done", id, note @ ..] => {
      task_judge_cmd(id, true, &note.join(" "))
    }
    ["task", "drop", id, note @ ..] => {
      task_judge_cmd(id, false, &note.join(" "))
    }
    ["task", "grade", id, rest @ ..] => task_grade_cmd(id, rest),
    ["task", "history", id] => task_history_cmd(id),
    ["task", ns, rest @ ..] => task_file_cmd(ns, rest),
    ["task"] => {
      let sty = style::Style::detect();
      println!("{}", paint_cli_page(DOCKET_USAGE, &sty));
      ExitCode::SUCCESS
    }
    ["import"] => {
      let sty = style::Style::detect();
      println!("{}", paint_cli_page(IMPORT_USAGE, &sty));
      ExitCode::SUCCESS
    }
    ["tasks", rest @ ..] => tasks_cmd(rest),
    ["roadmap"] => roadmap_cmd(None),
    ["roadmap", ns] => roadmap_cmd(Some(ns)),
    ["janitor"] => janitor_cmd(false),
    ["janitor", "--apply"] => janitor_cmd(true),
    ["inbox"] => inbox_cmd(),
    ["review", id] => review_cmd(id),
    ["approve", id] => judge_cmd(id, true, None),
    ["reject", id, reason @ ..] => {
      let reason = reason.join(" ");
      let reason = (!reason.is_empty()).then_some(reason);
      judge_cmd(id, false, reason)
    }
    ["retire", id] => retire_cmd(id, true),
    ["unretire", id] => retire_cmd(id, false),
    ["status"] => status_cmd(),
    ["config"] => config_cmd(false),
    ["config", "--init"] => config_cmd(true),
    ["config", "--open"] => config_open(),
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
      let sty = style::Style::detect();
      println!("{}", paint_cli_page(USAGE, &sty));
      println!("\n{}", sty.bold("topics: kumbarium help <topic>"));
      println!("  {}", sty.dim(help::TOPICS));
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
    ["audit", "verify"] => audit_verify(),
    ["audit"] => audit_tail(20, None),
    [] => {
      let sty = style::Style::detect();
      println!("{}", paint_cli_page(USAGE, &sty));
      ExitCode::SUCCESS
    }
    other => {
      // One line and a pointer, never the whole wall: a typo
      // deserves a hint, not a punishment.
      let word = other.first().copied().unwrap_or("");
      eprintln!("kumbarium: unknown command {word:?}");
      eprintln!("the map: kumbarium help");
      ExitCode::FAILURE
    }
  }
}

/// The pair every command opens first.
pub(crate) type Stores = (paths::Paths, tools::ServerState);

/// Open both databases at their platform paths, creating the
/// data directory on first run.
pub(crate) fn open_stores() -> Result<Stores, String> {
  let p = paths::resolve().map_err(|e| e.to_string())?;
  let library_dir = p.memory_db.parent().ok_or("memory path has no parent")?;
  std::fs::create_dir_all(library_dir)
    .map_err(|e| format!("creating library dir: {e}"))?;
  relocate_legacy(&p)?;
  let library = kumbarium_store::open(&p.memory_db)
    .map_err(|e| format!("opening memory shelf: {e}"))?;
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
    docket: None,
    docket_path: p.docket_db.clone(),
  };
  Ok((p, state))
}

/// One-time relocation for pre-D-033 layouts: library.db moves
/// to library/memory.db (WAL sidecars must be gone: rename only
/// when no -wal file exists, i.e. no live connection checkpoint
/// pending; a fresh open checkpoints and removes them, so the
/// move happens on the first open AFTER the last old-binary
/// process exits). The backups shelf renames with it.
fn relocate_legacy(p: &paths::Paths) -> Result<(), String> {
  let data_dir = p
    .memory_db
    .parent()
    .and_then(|d| d.parent())
    .ok_or("memory path has no data root")?;
  let old_db = data_dir.join("library.db");
  if old_db.exists() && !p.memory_db.exists() {
    let wal = data_dir.join("library.db-wal");
    if wal.exists() {
      return Err(
        "library.db has a live WAL sidecar; close other \
         kumbarium processes and retry"
          .into(),
      );
    }
    std::fs::rename(&old_db, &p.memory_db)
      .map_err(|e| format!("moving library.db to the shelf: {e}"))?;
  }
  let old_backups = p.backups_dir.join("library");
  let new_backups = p.backups_dir.join("memory");
  if old_backups.exists() && !new_backups.exists() {
    std::fs::rename(&old_backups, &new_backups)
      .map_err(|e| format!("moving backups shelf: {e}"))?;
  }
  Ok(())
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
  let cfg = &state.cfg;
  let interval_ms = cfg.backup_interval_hours * 3_600_000;
  let jobs = [
    (
      "memory",
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
    p.memory_db.display()
  );
  let stdin = std::io::stdin();
  let mut stdout = std::io::stdout();
  match rpc::serve(stdin.lock(), &mut stdout, &mut state) {
    Ok(()) => ExitCode::SUCCESS,
    Err(e) => fail(&format!("transport error: {e}")),
  }
}
