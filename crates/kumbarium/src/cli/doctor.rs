//! `kum doctor` (D-048): the mechanic. The janitor judges
//! FACTS; the doctor judges the BUILDING (files, schemas,
//! invariants, integrity). Preview by default; `--apply`
//! performs only the preen-class repairs (debris, dead locks,
//! an unhashed ledger tail): derived state and debris, never
//! testimony. A hash mismatch or content divergence is
//! EVIDENCE and is reported, never "repaired".

use std::path::PathBuf;
use std::process::ExitCode;

use serde_json::json;

use super::super::{open_stores, paths, procs, style};
use super::term::*;

#[derive(PartialEq)]
enum Mark {
  Ok,
  Warn,
  Fail,
}

struct Finding {
  section: &'static str,
  mark: Mark,
  detail: String,
  /// A copy-pasteable remedy, or a preen note the apply pass
  /// acts on.
  remedy: Option<String>,
  /// True when `--apply` can fix this (preen-class).
  preenable: bool,
}

pub(crate) fn doctor_cmd(deep: bool, apply: bool, json: bool) -> ExitCode {
  let p = match paths::resolve() {
    Ok(p) => p,
    Err(e) => return fail(&e.to_string()),
  };
  let mut f: Vec<Finding> = Vec::new();

  // SECTION INTEGRITY: read-only quick_check (integrity_check
  // under --deep), no migration, no write.
  let sections: &[(&str, &PathBuf)] = &[
    ("memory", &p.memory_db),
    ("audit", &p.audit_db),
    ("docket", &p.docket_db),
    ("handoff", &p.handoff_db),
    ("secrets", &p.secrets_db),
    ("leases", &p.leases_db),
  ];
  for (name, path) in sections {
    if !path.exists() {
      continue;
    }
    match kumbarium_store::integrity(path, deep) {
      Ok(None) => f.push(ok(name, format!("{name}.db intact"))),
      Ok(Some(problems)) => f.push(Finding {
        section: name,
        mark: Mark::Fail,
        detail: format!(
          "{name}.db failed integrity: {}",
          problems.first().cloned().unwrap_or_default()
        ),
        remedy: Some(
          "restore the newest good snapshot (kum backup list; \
           kum help backup); do not edit in place"
            .into(),
        ),
        preenable: false,
      }),
      Err(e) => f.push(Finding {
        section: name,
        mark: Mark::Fail,
        detail: format!("{name}.db could not be checked: {e}"),
        remedy: None,
        preenable: false,
      }),
    }
  }

  // CHAIN HEALTH: an unhashed tail is preen-class (pure
  // recomputation); a mismatch is evidence and is reported.
  if p.audit_db.exists() {
    let (_, state) = match open_stores() {
      Ok(v) => v,
      Err(e) => return fail(&e),
    };
    match kumbarium_audit::verify_chain(&state.audit) {
      Ok(kumbarium_audit::ChainStatus::Intact { events, .. }) => {
        f.push(ok("audit", format!("hash chain intact ({events} events)")))
      }
      Ok(kumbarium_audit::ChainStatus::Broken { index, id, .. }) => {
        f.push(Finding {
          section: "audit",
          mark: Mark::Fail,
          detail: format!(
            "hash chain BROKEN at event {index} (id {id}); \
             every later event is untrustworthy"
          ),
          remedy: Some(
            "this is tamper evidence, not a repair target: \
             kum audit verify, then investigate; restore from \
             a trusted snapshot if the ledger was altered"
              .into(),
          ),
          preenable: false,
        })
      }
      Err(e) => f.push(Finding {
        section: "audit",
        mark: Mark::Fail,
        detail: format!("chain could not be verified: {e}"),
        remedy: None,
        preenable: false,
      }),
    }
    // An unhashed tail (old-binary writes) re-chains on open,
    // but say so and let --apply force it now.
    let unhashed: i64 = state
      .audit
      .query_row("SELECT count(*) FROM events WHERE hash IS NULL", [], |r| {
        r.get(0)
      })
      .unwrap_or(0);
    if unhashed > 0 {
      f.push(Finding {
        section: "audit",
        mark: Mark::Warn,
        detail: format!("{unhashed} event(s) not yet hashed"),
        remedy: Some("kum doctor --apply re-chains them".into()),
        preenable: true,
      });
    }

    // REFERENTIAL DRIFT: docket/handoff rows on namespaces the
    // registry no longer knows (report; a namespace vanished
    // from under filed work).
    drift_checks(&p, &state, &mut f);
  }

  // DEBRIS: interrupted-backup tmp files and dead-process
  // presence records (preen-class sweeps).
  let mut debris: Vec<PathBuf> = Vec::new();
  for (name, _) in sections {
    let dir = p.backups_dir.join(name);
    if let Ok(read) = std::fs::read_dir(&dir) {
      for e in read.flatten() {
        let path = e.path();
        if path
          .file_name()
          .and_then(|n| n.to_str())
          .is_some_and(|n| n.starts_with(".tmp-"))
        {
          debris.push(path);
        }
      }
    }
  }
  let stale_procs = procs::stale(&p.procs_dir);
  if debris.is_empty() && stale_procs.is_empty() {
    f.push(ok(
      "debris",
      "no interrupted-backup or stale records".into(),
    ));
  } else {
    if !debris.is_empty() {
      f.push(Finding {
        section: "debris",
        mark: Mark::Warn,
        detail: format!("{} interrupted-backup temp file(s)", debris.len()),
        remedy: Some("kum doctor --apply sweeps them".into()),
        preenable: true,
      });
    }
    if !stale_procs.is_empty() {
      f.push(Finding {
        section: "processes",
        mark: Mark::Warn,
        detail: format!(
          "{} stale presence record(s) from dead processes",
          stale_procs.len()
        ),
        remedy: Some("kum doctor --apply sweeps them".into()),
        preenable: true,
      });
    }
  }

  // KEYSTORE: a present-but-failing keystore is the downgrade
  // shape; a plaintext shelf is a stated choice, noted not
  // warned.
  if p.secrets_db.exists()
    && let Ok(conn) = kumbarium_secrets::open(&p.secrets_db)
  {
    match kumbarium_secrets::sealing_mode(&conn) {
      Ok(Some(kumbarium_secrets::Sealing::Keystore)) => {
        f.push(ok("secrets", "keystore-sealed".into()))
      }
      Ok(Some(kumbarium_secrets::Sealing::Plaintext)) => f.push(Finding {
        section: "secrets",
        mark: Mark::Ok,
        detail: "plaintext sealing (a stated choice)".into(),
        remedy: None,
        preenable: false,
      }),
      Ok(None) => {}
      Err(e) => f.push(Finding {
        section: "secrets",
        mark: Mark::Fail,
        detail: format!("keystore present but failing: {e}"),
        remedy: Some(
          "a suppressed keystore is the downgrade-attack \
           shape; do not proceed until it is restored"
            .into(),
        ),
        preenable: false,
      }),
    }
  }

  // BACKUPS (deep only, and only as coverage info): a section
  // with no snapshot at all.
  if deep {
    for (name, path) in sections {
      if !path.exists() {
        continue;
      }
      let dir = p.backups_dir.join(name);
      if kumbarium_store::latest_backup_ms(&dir).is_none() {
        f.push(Finding {
          section: name,
          mark: Mark::Warn,
          detail: format!("{name} has no snapshot yet"),
          remedy: Some("kum backup takes one now".into()),
          preenable: false,
        });
      }
    }
  }

  if json {
    return emit_json(&f);
  }
  render(&f, &p, apply, deep)
}

fn ok(section: &'static str, detail: String) -> Finding {
  Finding {
    section,
    mark: Mark::Ok,
    detail,
    remedy: None,
    preenable: false,
  }
}

/// Namespaces filed-against that the registry no longer knows.
fn drift_checks(
  p: &paths::Paths,
  state: &super::super::tools::ServerState,
  f: &mut Vec<Finding>,
) {
  let known: std::collections::HashSet<String> =
    match kumbarium_store::namespaces(&state.library) {
      Ok(rows) => rows.into_iter().map(|(path, _, _)| path).collect(),
      Err(_) => return,
    };
  let mut orphaned = 0usize;
  if p.docket_db.exists()
    && let Ok(conn) = kumbarium_docket::open(&p.docket_db)
    && let Ok(tasks) = kumbarium_docket::tasks_in(&conn, None, true)
  {
    orphaned += tasks
      .iter()
      .filter(|t| !known.contains(&t.namespace))
      .count();
  }
  if p.handoff_db.exists()
    && let Ok(conn) = kumbarium_handoff::open(&p.handoff_db)
    && let Ok(all) = kumbarium_handoff::standings(&conn)
  {
    orphaned += all.iter().filter(|h| !known.contains(&h.namespace)).count();
  }
  if orphaned > 0 {
    f.push(Finding {
      section: "referential",
      mark: Mark::Warn,
      detail: format!(
        "{orphaned} matter(s)/briefing(s) on namespaces the \
         registry no longer knows"
      ),
      remedy: Some(
        "re-register the namespace (kum namespace add) or move \
         the rows (kum move); the doctor never deletes data"
          .into(),
      ),
      preenable: false,
    });
  } else {
    f.push(ok(
      "referential",
      "every filed row is on a live shelf".into(),
    ));
  }
}

fn render(
  f: &[Finding],
  p: &paths::Paths,
  apply: bool,
  deep: bool,
) -> ExitCode {
  let sty = style::Style::detect();
  let fails = f.iter().filter(|x| x.mark == Mark::Fail).count();
  let warns = f.iter().filter(|x| x.mark == Mark::Warn).count();
  for x in f {
    if x.mark == Mark::Ok {
      // Passing checks stay to one dim line each; the healthy
      // run reads at a glance (the anti-brew-noise rule).
      println!(
        "{} {} {}",
        sty.green("ok  "),
        sty.dim(x.section),
        sty.dim(&x.detail)
      );
      continue;
    }
    let (glyph, painted) = match x.mark {
      Mark::Warn => ("warn", sty.yellow(&x.detail)),
      _ => ("fail", sty.red(&x.detail)),
    };
    println!("{} {} {painted}", sty.bold(glyph), sty.bold(x.section));
    if let Some(r) = &x.remedy {
      println!("     {}", sty.dim(&format!("fix: {r}")));
    }
  }

  if apply {
    return apply_repairs(f, p);
  }

  let tier = if deep { "deep" } else { "quick" };
  if fails == 0 && warns == 0 {
    println!(
      "\n{} ({tier} tier; deep tier adds index cross-checks \
       and backup reads)",
      sty.green("healthy")
    );
    ExitCode::SUCCESS
  } else {
    let preenable = f.iter().filter(|x| x.preenable).count();
    println!(
      "\n{}",
      sty.bold(&format!("{fails} failing, {warns} warning(s)"))
    );
    if preenable > 0 {
      println!(
        "{}",
        sty.dim(&format!("{preenable} are repairable: kum doctor --apply"))
      );
    }
    ExitCode::FAILURE
  }
}

fn apply_repairs(f: &[Finding], p: &paths::Paths) -> ExitCode {
  let sty = style::Style::detect();
  let preenable: Vec<&Finding> = f.iter().filter(|x| x.preenable).collect();
  if preenable.is_empty() {
    println!(
      "\n{}",
      sty.dim("nothing to repair (no preen-class findings)")
    );
    // A --apply run with nothing preenable but live failures
    // still exits nonzero so a gate sees the ill health.
    return if f.iter().any(|x| x.mark == Mark::Fail) {
      ExitCode::FAILURE
    } else {
      ExitCode::SUCCESS
    };
  }
  // File surgery requires an empty registry: a live process's
  // WAL/records must not be swept from under it.
  let live = procs::live(&p.procs_dir);
  if !live.is_empty() {
    println!(
      "\n{}",
      sty.yellow(&format!(
        "{} live process(es) present; deferring repairs \
         (close them and rerun):",
        live.len()
      ))
    );
    for r in &live {
      println!("  pid {} ({}, {})", r.pid, r.agent, r.client);
    }
    return ExitCode::FAILURE;
  }

  let (_, state) = match open_stores() {
    Ok(v) => v,
    Err(e) => return fail(&e),
  };
  // Snapshot before touching anything: reversibility is
  // state-level, not an undo flag.
  match super::super::backup_now_quiet(p, &state) {
    Ok(()) => println!("\n{}", sty.dim("snapshotted before repair")),
    Err(e) => return fail(&format!("pre-repair snapshot failed: {e}")),
  }

  let mut repaired = 0usize;
  // Sweep tmp debris.
  for (name, _) in [
    ("memory", ()),
    ("audit", ()),
    ("docket", ()),
    ("handoff", ()),
    ("secrets", ()),
    ("leases", ()),
  ] {
    let dir = p.backups_dir.join(name);
    if let Ok(read) = std::fs::read_dir(&dir) {
      for e in read.flatten() {
        let path = e.path();
        if path
          .file_name()
          .and_then(|n| n.to_str())
          .is_some_and(|n| n.starts_with(".tmp-"))
          && std::fs::remove_file(&path).is_ok()
        {
          repaired += 1;
        }
      }
    }
  }
  // Sweep stale presence records.
  for path in procs::stale(&p.procs_dir) {
    if std::fs::remove_file(&path).is_ok() {
      repaired += 1;
      let mut lock = path.into_os_string();
      lock.push(".lock");
      let _ = std::fs::remove_file(PathBuf::from(lock));
    }
  }
  // Re-chain an unhashed tail.
  if state
    .audit
    .query_row("SELECT count(*) FROM events WHERE hash IS NULL", [], |r| {
      r.get::<_, i64>(0)
    })
    .unwrap_or(0)
    > 0
  {
    match kumbarium_audit::backfill_chain(&state.audit) {
      Ok(()) => repaired += 1,
      Err(e) => eprintln!("kumbarium: re-chaining failed: {e}"),
    }
  }

  // Witness one doctor event with the manifest.
  let event = kumbarium_audit::Event {
    agent_id: "kumbarium-cli".into(),
    session_id: state.session_id.clone(),
    kind: kumbarium_audit::EventKind::Doctor,
    scope: String::new(),
    detail: json!({ "repaired": repaired }),
  };
  if let Err(e) = kumbarium_audit::append(&state.audit, &event) {
    return fail(&format!("repaired, but audit append failed: {e}"));
  }
  println!("{}", sty.green(&format!("repaired {repaired} finding(s)")));
  if f.iter().any(|x| x.mark == Mark::Fail) {
    println!(
      "{}",
      sty.dim("failing checks remain (report-only; see fixes above)")
    );
    return ExitCode::FAILURE;
  }
  ExitCode::SUCCESS
}

fn emit_json(f: &[Finding]) -> ExitCode {
  let rows: Vec<serde_json::Value> = f
    .iter()
    .map(|x| {
      json!({
        "section": x.section,
        "status": match x.mark {
          Mark::Ok => "ok",
          Mark::Warn => "warn",
          Mark::Fail => "fail",
        },
        "detail": x.detail,
        "fix": x.remedy,
        "repairable": x.preenable,
      })
    })
    .collect();
  let code = if f
    .iter()
    .any(|x| x.mark == Mark::Fail || x.mark == Mark::Warn)
  {
    ExitCode::FAILURE
  } else {
    ExitCode::SUCCESS
  };
  print_json(&json!(rows));
  code
}
