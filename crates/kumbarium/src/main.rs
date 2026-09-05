//! Kumbarium: the librarian process. `serve` speaks MCP over
//! stdio (D-014); the rest is the human-facing CLI.

mod bundle;
mod cli;
mod config;
mod diff;
mod help;
mod import;
mod keystore;
mod markdown;
mod paths;
mod procs;
mod rpc;
mod style;
mod tools;

use std::process::ExitCode;

use cli::admin::*;
use cli::brief::*;
use cli::desk::*;
use cli::dock::*;
use cli::docket::*;
use cli::dossier::*;
use cli::entries::*;
use cli::handoff::*;
use cli::lease::*;
use cli::secret::*;
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
  let args = expand_alias(args);
  let argv: Vec<&str> = args.iter().map(String::as_str).collect();
  // GNU flag forms first (muscle memory from every peer tool):
  // --version anywhere-first, and --help/-h ANYWHERE in the
  // argv routes to the manual instead of running the command
  // (running on --help is how a write verb mutates state on a
  // help request).
  if matches!(argv.first(), Some(&"--version") | Some(&"-V")) {
    return version_cmd();
  }
  if argv.len() > 1
    && argv.first() != Some(&"help")
    && argv.iter().any(|a| *a == "--help" || *a == "-h")
  {
    let topic = argv[0];
    let sty = style::Style::detect();
    return match help::page(topic) {
      Some(md) => {
        println!("{}", markdown::render(md, &sty));
        ExitCode::SUCCESS
      }
      None => match usage_of(topic) {
        Some(usage) => {
          println!("{}", paint_cli_page(usage, &sty));
          ExitCode::SUCCESS
        }
        None => fail(&format!("no command {topic:?}; the map: kumbarium help")),
      },
    };
  }
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
    ["serve", "reload"] => cli::process::serve_reload_cmd(None),
    ["serve", "reload", pid] => cli::process::serve_reload_cmd(Some(pid)),
    ["namespace", "add", path, rest @ ..] => {
      namespace_add(path, &rest.join(" "))
    }
    ["namespace", "describe", path, rest @ ..] => {
      namespace_describe(path, &rest.join(" "))
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
    ["backup", "list"] => backup_list(),
    ["backup"] => backup_now(),
    ["doctor", rest @ ..] => {
      let mut deep = false;
      let mut apply = false;
      let mut json = false;
      let mut bad = None;
      for a in rest {
        match *a {
          "--deep" => deep = true,
          "--apply" => apply = true,
          "--json" => json = true,
          other => bad = Some(other.to_string()),
        }
      }
      match bad {
        Some(flag) => fail(&format!("unknown flag {flag:?}")),
        None => cli::doctor::doctor_cmd(deep, apply, json),
      }
    }
    ["list", rest @ ..] => match browse_args(rest) {
      Ok((ns, all, json)) => list_entries(ns, all, json),
      Err(e) => fail(&e),
    },
    ["show", id] if !id.starts_with('-') => show_entry(id, false),
    ["show", id, "--full"] => show_entry(id, true),
    ["history", id, rest @ ..] => {
      let with_diff = rest.contains(&"--diff");
      let all = rest.contains(&"--all");
      history_cmd(id, with_diff, all)
    }
    ["revert", id] => revert_cmd(id, false),
    ["revert", id, "--apply"] => revert_cmd(id, true),
    ["confirm", id] => confirm_cmd(id),
    ["link", from, rel, to] => link_cmd(from, rel, to),
    ["task", "done", id, note @ ..] => {
      task_judge_cmd(id, true, &note.join(" "))
    }
    ["task", "drop", id, note @ ..] => {
      task_judge_cmd(id, false, &note.join(" "))
    }
    ["task", "grade", id, rest @ ..] => task_grade_cmd(id, rest),
    ["task", "reword", id, rest @ ..] => task_reword_cmd(id, rest),
    ["task", "history", id] => task_history_cmd(id),
    ["task", ns, rest @ ..] => task_file_cmd(ns, rest),
    // Bare singular browses like the plural (the audit's
    // coherence rule: bare nouns show data, never usage
    // walls); the verb map lives in kum help docket.
    ["task"] => tasks_cmd(&[]),
    ["import"] => {
      let sty = style::Style::detect();
      println!("{}", paint_cli_page(IMPORT_USAGE, &sty));
      ExitCode::SUCCESS
    }
    ["tasks", rest @ ..] => tasks_cmd(rest),
    ["handoff", "drop", ns] => handoff_drop_cmd(ns),
    ["handoff", ns, rest @ ..] => handoff_cmd(ns, rest),
    ["handoff"] | ["handoffs"] => handoffs_cmd(),
    ["brief", ns] if !ns.starts_with('-') => brief_cmd(ns),
    ["dossier", agent, rest @ ..] => dossier_cmd(agent, rest),
    ["dossier"] => fail("dossier needs an agent: kumbarium dossier <agent>"),
    ["agents", rest @ ..] => match browse_args(rest) {
      Ok((Some(_), _, _)) => {
        fail("the roster is building-wide; usage: kumbarium agents [--all]")
      }
      Ok((None, all, json)) => agents_cmd(all, json),
      Err(e) => fail(&e),
    },
    // Reserved for the agent-lifecycle family: no identity may
    // bear these words, so the dossier route refuses them
    // instead of rendering an empty story.
    ["agent", verb, ..] if tools::reserved_agent_word(verb) => fail(&format!(
      "kum agent {verb} is reserved for the agent lifecycle \
         (not built yet); the roster: kum agents, the deep \
         story: kum dossier <agent>"
    )),
    ["agent", name] => dossier_cmd(name, &[]),
    ["agent", name, rest @ ..] => dossier_cmd(name, rest),
    ["agent"] => {
      fail("the roster: kum agents | kum agent <name> (the dossier)")
    }
    ["brief"] => fail("brief needs a scope: kumbarium brief <ns>"),
    ["leases", rest @ ..] => match browse_args(rest) {
      Ok((_, true, _)) => fail("leases takes no --all"),
      Ok((ns, _, json)) => leases_cmd(ns, json),
      Err(e) => fail(&e),
    },
    ["lease", "break", id] => lease_break_cmd(id),
    ["lease", ..] => {
      fail("the reading room: kum leases [ns] | kum lease break <id>")
    }
    ["secret", rest @ ..] => secret_cmd(rest),
    ["secrets", rest @ ..] => match browse_args(rest) {
      Ok((_, true, _)) => fail("secrets takes no --all"),
      Ok((ns, _, json)) => secrets_cmd(ns, json),
      Err(e) => fail(&e),
    },
    ["roadmap"] => roadmap_cmd(None),
    ["roadmap", ns] if !ns.starts_with('-') => roadmap_cmd(Some(ns)),
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
    ["forget", id] => forget_cmd(id, false),
    ["forget", id, "--yes"] => forget_cmd(id, true),
    ["retire", id] => retire_cmd(id, true),
    ["unretire", id] => retire_cmd(id, false),
    ["status"] => status_cmd(),
    ["status", "--json"] => status_json(),
    ["processes", rest @ ..] => match browse_args(rest) {
      Ok((Some(_), _, _)) | Ok((_, true, _)) => {
        fail("usage: kumbarium processes [--json]")
      }
      Ok((None, false, json)) => cli::process::processes_cmd(json),
      Err(e) => fail(&e),
    },
    ["update"] => cli::update::update_cmd(false, false),
    ["update", "--check"] => cli::update::update_cmd(true, false),
    ["update", "--yes"] => cli::update::update_cmd(false, true),
    ["completions", shell] => cli::completions::completions_cmd(shell, false),
    ["completions", shell, "--install"]
    | ["completions", "--install", shell] => {
      cli::completions::completions_cmd(shell, true)
    }
    ["completions"] => {
      fail("completions needs a shell: kumbarium completions bash|zsh|fish")
    }
    ["config"] => config_cmd(false),
    ["config", "--init"] => config_cmd(true),
    ["config", "--open"] => config_open(),
    ["grep", pattern] => grep_cmd(pattern, None, false),
    ["grep", pattern, "--all"] => grep_cmd(pattern, None, true),
    ["grep", pattern, ns] if !ns.starts_with('-') => {
      grep_cmd(pattern, Some(ns), false)
    }
    ["grep", pattern, ns, "--all"] if !ns.starts_with('-') => {
      grep_cmd(pattern, Some(ns), true)
    }
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
      println!(
        "  {}",
        sty.dim("the whole manual, in order: kumbarium help --all")
      );
      ExitCode::SUCCESS
    }
    ["help", "--all"] => {
      let sty = style::Style::detect();
      println!("{}", sty.bold("# The Kumbarium manual"));
      println!(
        "{}",
        sty.dim(
          "the building, in reading order; one topic per \
           section (kum help <topic> opens any of them alone)"
        )
      );
      for topic in help::MANUAL_ORDER {
        if let Some(md) = help::page(topic) {
          println!("\n{}", markdown::render(md, &sty));
        }
      }
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
      let word = other.first().copied().unwrap_or("");
      // A REAL command with the wrong shape gets its usage
      // line, never a false "no such command" (the audit's
      // worst finding: kum show -> 'no command "show"').
      if let Some(usage) = usage_of(word) {
        eprintln!("kumbarium: usage: {usage}");
        eprintln!("details: kumbarium help {}", help_topic(word));
        return ExitCode::FAILURE;
      }
      // An MCP-only verb gets pointed at the agent door.
      if [
        "remember",
        "recall",
        "get",
        "task_list",
        "supersede",
        "lease_take",
        "lease_release",
        "secret_read",
        "task_file",
        "task_update",
        "handoff_write",
      ]
      .contains(&word)
      {
        eprintln!(
          "kumbarium: {word:?} is an agent tool (MCP), not a \
           CLI command; agents call it via kumbarium serve"
        );
        eprintln!("wiring an agent up: kumbarium instructions");
        return ExitCode::FAILURE;
      }
      // One line and a pointer, never the whole wall: a typo
      // deserves a hint, not a punishment.
      eprintln!("kumbarium: no command or alias {word:?}");
      if let Some(near) = nearest_command(word) {
        eprintln!("did you mean {near:?}?");
      }
      eprintln!("the map: kumbarium help");
      ExitCode::FAILURE
    }
  }
}

/// The shared argument shape of the browse commands: one
/// optional namespace positional plus `--all` / `--json` in
/// any order; anything else is refused loudly (a swallowed
/// flag was the audit's worst class of lie).
fn browse_args<'a>(
  rest: &[&'a str],
) -> Result<(Option<&'a str>, bool, bool), String> {
  let mut ns = None;
  let mut all = false;
  let mut json = false;
  for a in rest {
    match *a {
      "--all" => all = true,
      "--json" => json = true,
      w if !w.starts_with('-') && ns.is_none() => ns = Some(w),
      w if !w.starts_with('-') => {
        return Err(format!("unexpected argument {w:?}"));
      }
      other => return Err(format!("unknown flag {other:?}")),
    }
  }
  Ok((ns, all, json))
}

/// One usage line per command word, for the wrong-shape error
/// path and flag-form help. The map page (USAGE) stays the
/// human-ordered source; this is the machine-keyed index.
fn usage_of(word: &str) -> Option<&'static str> {
  Some(match word {
    "list" => "kumbarium list [ns] [--all]",
    "show" => "kumbarium show <id> [--full]",
    "grep" => "kumbarium grep <pattern> [ns] [--all]",
    "history" => "kumbarium history <id> [--diff] [--all]",
    "confirm" => "kumbarium confirm <id>",
    "link" => "kumbarium link <from-id> <rel> <to-id>",
    "move" => "kumbarium move <id> <namespace>",
    "forget" => "kumbarium forget <id> [--yes]",
    "retire" => "kumbarium retire <id>",
    "unretire" => "kumbarium unretire <id>",
    "revert" => "kumbarium revert <id> [--apply]",
    "janitor" => "kumbarium janitor [--apply]",
    "task" => {
      "kumbarium task <ns> <matter...> | task \
       done|drop|grade|reword|history <id>"
    }
    "tasks" => "kumbarium tasks [ns] [--all] [--severity S]",
    "roadmap" => "kumbarium roadmap [ns]",
    "brief" => "kumbarium brief <ns>",
    "agents" => "kumbarium agents [--all]",
    "agent" => "kumbarium agent <name> (the dossier)",
    "dossier" => {
      "kumbarium dossier <agent> [--since D] [--until D] [--session F]"
    }
    "leases" => "kumbarium leases [ns]",
    "lease" => "kumbarium lease break <id>",
    "handoff" => "kumbarium handoff <ns> [<note...>] | handoff drop <ns>",
    "handoffs" => "kumbarium handoffs",
    "inbox" => "kumbarium inbox",
    "review" => "kumbarium review <id>",
    "approve" => "kumbarium approve <id>",
    "reject" => "kumbarium reject <id> [reason]",
    "secret" => "kumbarium secret <verb> ... (kum help secrets for the verbs)",
    "secrets" => "kumbarium secrets [ns]",
    "audit" => "kumbarium audit [tail [n] [--scope ns] | verify]",
    "export" => "kumbarium export [minutes|bundle <ns>]",
    "import" => "kumbarium import [bundle <FILE>|claude]",
    "namespace" => {
      "kumbarium namespace [add <path> [desc]|describe <path> <desc>|list]"
    }
    "status" => "kumbarium status",
    "processes" => "kumbarium processes [--json]",
    "backup" => "kumbarium backup [list]",
    "doctor" => "kumbarium doctor [--deep] [--apply] [--json]",
    "config" => "kumbarium config [--init|--open]",
    "paths" => "kumbarium paths",
    "serve" => "kumbarium serve [reload [pid]]",
    "update" => "kumbarium update [--check|--yes]",
    "completions" => "kumbarium completions bash|zsh|fish [--install]",
    "instructions" => "kumbarium instructions [--snippet]",
    "version" => "kumbarium version",
    "help" => "kumbarium help [topic|--all]",
    _ => return None,
  })
}

/// The help topic a command word reads under (page() aliases
/// cover most; the rest map here).
fn help_topic(word: &str) -> &'static str {
  match word {
    "task" | "tasks" | "roadmap" => "docket",
    "lease" => "leases",
    "secret" => "secrets",
    "agent" => "agents",
    "review" | "approve" | "reject" | "inbox" => "approvals",
    "forget" | "unretire" => "retire",
    "confirm" => "list",
    "doctor" => "doctor",
    "link" => "show",
    w if help::page(w).is_some() => {
      // page() accepts it directly; leak the word as 'static
      // is impossible, so return a known alias instead.
      match w {
        "list" => "list",
        "show" => "show",
        "grep" => "grep",
        "history" => "history",
        "move" => "move",
        "retire" => "retire",
        "revert" => "revert",
        "janitor" => "janitor",
        "brief" => "brief",
        "agents" => "agents",
        "dossier" => "dossier",
        "leases" => "leases",
        "handoff" | "handoffs" => "handoff",
        "secrets" => "secrets",
        "audit" => "audit",
        "export" => "export",
        "import" => "import",
        "namespace" => "namespace",
        "status" => "status",
        "backup" => "backup",
        "serve" => "serve",
        "instructions" => "instructions",
        _ => "instructions",
      }
    }
    _ => "instructions",
  }
}

/// Levenshtein-lite did-you-mean: the known command with edit
/// distance <= 2, if exactly one qualifies well. The word list
/// is the completions module's (one source, no drift).
fn nearest_command(word: &str) -> Option<&'static str> {
  const WORDS: &[&str] = cli::completions::COMMAND_WORDS;
  fn dist(a: &str, b: &str) -> usize {
    let a: Vec<char> = a.chars().collect();
    let b: Vec<char> = b.chars().collect();
    let mut prev: Vec<usize> = (0..=b.len()).collect();
    for (i, ca) in a.iter().enumerate() {
      let mut cur = vec![i + 1];
      for (j, cb) in b.iter().enumerate() {
        let cost = if ca == cb { 0 } else { 1 };
        cur.push((prev[j + 1] + 1).min(cur[j] + 1).min(prev[j] + cost));
      }
      prev = cur;
    }
    prev[b.len()]
  }
  let mut best: Option<(&'static str, usize)> = None;
  for w in WORDS {
    let d = dist(word, w);
    if d <= 2 {
      best = match best {
        Some((_, bd)) if d >= bd => best,
        _ => Some((w, d)),
      };
    }
  }
  best.map(|(w, _)| w)
}

/// One alias expansion (D-035): when the first word is a config
/// alias, its value splices in as kumbarium ARGUMENTS and the
/// rest follow. Internal-only by construction (never shell);
/// builtins cannot be shadowed (the parser refuses those names),
/// and expansion happens exactly once, so chains cannot loop.
/// Config problems stay silent here; open_stores voices them.
fn expand_alias(args: Vec<String>) -> Vec<String> {
  let Some(first) = args.first() else {
    return args;
  };
  let Ok(p) = paths::resolve() else {
    return args;
  };
  let Ok(text) = std::fs::read_to_string(&p.config_file) else {
    return args;
  };
  let (cfg, _) = config::parse(&text);
  let Some((_, expansion)) = cfg.aliases.iter().find(|(name, _)| name == first)
  else {
    return args;
  };
  let mut out: Vec<String> =
    expansion.split_whitespace().map(str::to_string).collect();
  out.extend(args.into_iter().skip(1));
  out
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
    session_id: kumbarium_util::generate_id(),
    cfg,
    docket: None,
    docket_path: p.docket_db.clone(),
    handoff: None,
    handoff_path: p.handoff_db.clone(),
    served_handoffs: std::collections::HashSet::new(),
    secrets: None,
    secrets_path: p.secrets_db.clone(),
    leases: None,
    leases_path: p.leases_db.clone(),
    presence: None,
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
  // Every shelf backs up (D-033). The docket joins the rotation
  // once its file exists; it shares the library retention tier
  // (task data is primary data).
  let docket_conn = match p.docket_db.exists() {
    true => Some(
      kumbarium_docket::open(&p.docket_db)
        .map_err(|e| format!("opening docket shelf: {e}"))?,
    ),
    false => None,
  };
  let mut jobs = vec![
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
  if let Some(conn) = &docket_conn {
    jobs.push((
      "docket",
      conn,
      kumbarium_store::Retention {
        recent: cfg.library_recent,
        dailies: cfg.library_dailies,
        weeklies: cfg.library_weeklies,
      },
    ));
  }
  let handoff_conn = match p.handoff_db.exists() {
    true => Some(
      kumbarium_handoff::open(&p.handoff_db)
        .map_err(|e| format!("opening handoff shelf: {e}"))?,
    ),
    false => None,
  };
  if let Some(conn) = &handoff_conn {
    jobs.push((
      "handoff",
      conn,
      kumbarium_store::Retention {
        recent: cfg.library_recent,
        dailies: cfg.library_dailies,
        weeklies: cfg.library_weeklies,
      },
    ));
  }
  // The restricted stacks back up as ciphertext; the master
  // key is in the keystore, never in any snapshot (D-038).
  let secrets_conn = match p.secrets_db.exists() {
    true => Some(
      kumbarium_secrets::open(&p.secrets_db)
        .map_err(|e| format!("opening secrets shelf: {e}"))?,
    ),
    false => None,
  };
  if let Some(conn) = &secrets_conn {
    jobs.push((
      "secrets",
      conn,
      kumbarium_store::Retention {
        recent: cfg.library_recent,
        dailies: cfg.library_dailies,
        weeklies: cfg.library_weeklies,
      },
    ));
  }
  let leases_conn = match p.leases_db.exists() {
    true => Some(
      kumbarium_leases::open(&p.leases_db)
        .map_err(|e| format!("opening leases shelf: {e}"))?,
    ),
    false => None,
  };
  if let Some(conn) = &leases_conn {
    jobs.push((
      "leases",
      conn,
      kumbarium_store::Retention {
        recent: cfg.library_recent,
        dailies: cfg.library_dailies,
        weeklies: cfg.library_weeklies,
      },
    ));
  }
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

/// `kum backup list`: every section's snapshots, newest first.
/// Restore stays a documented hand move (kum help backup), so
/// the listing names the exact files that move.
fn backup_list() -> ExitCode {
  let p = match paths::resolve() {
    Ok(p) => p,
    Err(e) => return fail(&e.to_string()),
  };
  let sty = style::Style::detect();
  let mut any = false;
  for name in ["memory", "audit", "docket", "handoff", "secrets", "leases"] {
    let dir = p.backups_dir.join(name);
    let snaps = match kumbarium_store::snapshots(&dir) {
      Ok(s) => s,
      Err(e) => return fail(&format!("{name}: {e}")),
    };
    if snaps.is_empty() {
      continue;
    }
    any = true;
    println!("{}", sty.bold(&format!("{name} ({})", snaps.len())));
    for (_, path) in &snaps {
      let size_kb = std::fs::metadata(path).map(|m| m.len() / 1024);
      let file = path.file_name().unwrap_or_default().to_string_lossy();
      match size_kb {
        Ok(kb) => println!("  {file}  {kb} KB"),
        Err(_) => println!("  {file}"),
      }
    }
  }
  if !any {
    println!(
      "no snapshots yet; kum backup takes one now (serve \
       startup takes them on schedule)"
    );
  } else {
    println!(
      "{}",
      sty.dim(&format!(
        "restoring is a hand move by design: see kum help \
         backup (files live under {})",
        p.backups_dir.display()
      ))
    );
  }
  ExitCode::SUCCESS
}

/// Force a snapshot of every section, reporting nothing (the
/// doctor's pre-repair safety net; the loud version is
/// backup_now).
pub(crate) fn backup_now_quiet(
  p: &paths::Paths,
  state: &tools::ServerState,
) -> Result<(), String> {
  maintenance(p, state, true).map(|_| ())
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
  // Reload carryover (D-048): a re-exec'd serve consumes its
  // predecessor's state file exactly once (the session id,
  // claimed agent, served-handoffs set so the opening frame
  // does not replay, and any half-read input bytes).
  let mut residue: Vec<u8> = Vec::new();
  let mut reloaded = false;
  if let Some(sp) = std::env::var_os("KUMBARIUM_RELOAD_STATE") {
    unsafe { std::env::remove_var("KUMBARIUM_RELOAD_STATE") };
    let sp = std::path::PathBuf::from(sp);
    if let Ok(text) = std::fs::read_to_string(&sp) {
      let _ = std::fs::remove_file(&sp);
      if let Ok(v) = serde_json::from_str::<serde_json::Value>(&text) {
        if let Some(a) = v.get("agent_id").and_then(|x| x.as_str()) {
          state.agent_id = a.into();
        }
        if let Some(s) = v.get("session_id").and_then(|x| x.as_str()) {
          state.session_id = s.into();
        }
        if let Some(arr) = v.get("served_handoffs").and_then(|x| x.as_array()) {
          state.served_handoffs = arr
            .iter()
            .filter_map(|x| x.as_str())
            .map(str::to_string)
            .collect();
        }
        if let Some(arr) = v.get("residue").and_then(|x| x.as_array()) {
          residue = arr
            .iter()
            .filter_map(|x| x.as_u64())
            .map(|b| b as u8)
            .collect();
        }
        reloaded = true;
      }
    }
  }
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
  // Presence (D-048): the record that says this process is
  // here. Best-effort; awareness never blocks serving.
  state.presence = procs::Presence::register(
    &p.procs_dir,
    &procs::PresenceInfo {
      pid: std::process::id(),
      version: VERSION.into(),
      agent: state.agent_id.clone(),
      session: state.session_id.clone(),
      client: procs::parent_client_name(),
      since: kumbarium_util::now_iso8601(),
    },
  );
  // stdout is protocol-only; say where we are on stderr.
  eprintln!(
    "kumbarium {VERSION} serving MCP on stdio \
     (library: {})",
    p.memory_db.display()
  );
  let mut stdout = std::io::stdout();
  if reloaded {
    eprintln!(
      "kumbarium: reborn after reload; session {} continues",
      &state.session_id[state.session_id.len().saturating_sub(8)..]
    );
    // The client refetches the tool list on this notification,
    // so a new binary's tools appear mid-conversation.
    use std::io::Write;
    let note = serde_json::json!({
      "jsonrpc": "2.0",
      "method": "notifications/tools/list_changed",
    });
    let _ = writeln!(stdout, "{note}");
    let _ = stdout.flush();
  }
  #[cfg(unix)]
  {
    let idle = state.cfg.serve_idle_ping_minutes;
    match rpc::hot::serve(&mut state, residue, &p.procs_dir, idle) {
      Ok(()) => ExitCode::SUCCESS,
      Err(e) => fail(&format!("transport error: {e}")),
    }
  }
  #[cfg(not(unix))]
  {
    let _ = residue;
    let stdin = std::io::stdin();
    match rpc::serve(stdin.lock(), &mut stdout, &mut state) {
      Ok(()) => ExitCode::SUCCESS,
      Err(e) => fail(&format!("transport error: {e}")),
    }
  }
}
