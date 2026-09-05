//! `kum processes` (D-048): the occupancy listing. The third
//! axis beside the roster (identities across history) and the
//! reading room (work claims): live incarnations, right now.
//! Every row shown is liveness-verified by its record's OS
//! lock; the listing never renders a ghost.

use std::process::ExitCode;

use super::super::{open_stores, procs, style};
use super::term::*;

pub(crate) fn processes_cmd(json: bool) -> ExitCode {
  let (p, state) = match open_stores() {
    Ok(v) => v,
    Err(e) => return fail(&e),
  };
  let rows = procs::live(&p.procs_dir);
  let activity_at = |session: &str| -> Option<String> {
    kumbarium_audit::session_last_at(&state.audit, session)
      .ok()
      .flatten()
  };
  if json {
    let me = env!("CARGO_PKG_VERSION");
    let out: Vec<serde_json::Value> = rows
      .iter()
      .map(|r| {
        serde_json::json!({
          "pid": r.pid,
          "version": r.version,
          "agent": r.agent,
          "session_id": r.session,
          "client": r.client,
          "since": r.since,
          "last_activity_at": activity_at(&r.session),
          "stale_binary": r.version != me,
        })
      })
      .collect();
    return print_json(&serde_json::json!(out));
  }
  let sty = style::Style::detect();
  if rows.is_empty() {
    println!(
      "no live kumbarium processes on this library home \
       (clients spawn kum serve; kum instructions wires one up)"
    );
    return ExitCode::SUCCESS;
  }
  const COLS: &[Col] = &[
    Col {
      title: "pid",
      width: 6,
    },
    Col {
      title: "version",
      width: 8,
    },
    Col {
      title: "agent",
      width: 20,
    },
    Col {
      title: "session",
      width: 8,
    },
    Col {
      title: "client",
      width: 14,
    },
    Col {
      title: "since (local)",
      width: 19,
    },
    Col {
      title: "activity",
      width: 0,
    },
  ];
  println!("{}", sty.dim(&table_header(COLS)));
  let me = env!("CARGO_PKG_VERSION");
  let now = kumbarium_util::now_ms();
  for r in &rows {
    let activity = activity_at(&r.session)
      .and_then(|at| kumbarium_util::parse_iso8601_ms(&at))
      .map(|ms| ago(now - ms))
      .unwrap_or_else(|| "nothing witnessed yet".into());
    let session_short = r.session.get(r.session.len().saturating_sub(8)..);
    println!(
      "{} {} {} {} {} {} {}",
      cell(COLS, 0, &r.pid.to_string()),
      cell(COLS, 1, &r.version),
      cell(COLS, 2, &r.agent),
      sty.id(&cell(COLS, 3, session_short.unwrap_or(&r.session))),
      cell(COLS, 4, &r.client),
      sty.dim(&cell(COLS, 5, &local_display(&r.since))),
      sty.dim(&activity)
    );
    if r.version != me {
      // The load-bearing paint: after an update, this is the
      // question the listing exists to answer.
      println!(
        "       {}",
        sty.yellow(&format!(
          "STALE BINARY (this CLI is {me}); kum serve reload {}",
          r.pid
        ))
      );
    }
  }
  let debris = procs::stale(&p.procs_dir).len();
  if debris > 0 {
    println!(
      "{}",
      sty.dim(&format!(
        "({debris} stale record(s) from dead processes; \
         kum doctor sweeps them)"
      ))
    );
  }
  ExitCode::SUCCESS
}

/// `kum serve reload [pid]`: signal live serve processes to
/// re-exec the (possibly just-updated) binary in place. Fds
/// survive exec, so clients keep their pipes and never notice;
/// the session id carries over (D-048).
pub(crate) fn serve_reload_cmd(pid: Option<&str>) -> ExitCode {
  #[cfg(not(unix))]
  {
    let _ = pid;
    fail(
      "hot reload is unavailable on this platform (no exec); \
       restart the client session instead",
    )
  }
  #[cfg(unix)]
  {
    let p = match super::super::paths::resolve() {
      Ok(p) => p,
      Err(e) => return fail(&e.to_string()),
    };
    let wanted: Option<u32> = match pid {
      Some(raw) => match raw.parse() {
        Ok(n) => Some(n),
        Err(_) => return fail(&format!("{raw:?} is not a pid")),
      },
      None => None,
    };
    let rows: Vec<_> = procs::live(&p.procs_dir)
      .into_iter()
      .filter(|r| wanted.is_none_or(|w| r.pid == w))
      .collect();
    if rows.is_empty() {
      return fail(match wanted {
        Some(_) => "no live serve process with that pid (kum processes)",
        None => "no live serve processes to reload (kum processes)",
      });
    }
    let mut failures = 0;
    for r in &rows {
      let ok = unsafe { libc::kill(r.pid as libc::pid_t, libc::SIGUSR1) } == 0;
      if ok {
        println!(
          "signalled pid {} ({} on {}); it reloads after any \
           request in flight",
          r.pid, r.agent, r.version
        );
      } else {
        failures += 1;
        eprintln!("kumbarium: signalling pid {} failed", r.pid);
      }
    }
    if failures > 0 {
      ExitCode::FAILURE
    } else {
      ExitCode::SUCCESS
    }
  }
}

/// `active 2m ago` grammar, shared with the reading room's
/// activity column by convention.
fn ago(delta_ms: i64) -> String {
  let mins = (delta_ms / 60_000).max(0);
  match mins {
    0 => "active <1m ago".to_string(),
    m if m < 60 => format!("active {m}m ago"),
    m => format!("active {}h{}m ago", m / 60, m % 60),
  }
}
