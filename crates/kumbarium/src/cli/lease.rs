//! The reading room's human surfaces (D-043): the register
//! (`kum leases [ns]`) and the override (`kum lease break
//! <id>`, witnessed with the holder named). Agents take and
//! release through their own tools; the human mostly reads the
//! room and, rarely, clears a stuck card.

use std::process::ExitCode;

use super::super::{open_stores, style};
use super::term::*;

/// `kum leases [ns]`: the register. Active cards, then any
/// stale ones (expired-but-unreleased: the crashed-agent
/// shape), so the room's true state is one glance.
pub(crate) fn leases_cmd(ns: Option<&str>, json: bool) -> ExitCode {
  let (p, mut state) = match open_stores() {
    Ok(v) => v,
    Err(e) => return fail(&e),
  };
  let sty = style::Style::detect();
  if !p.leases_db.exists() {
    if json {
      return print_json(&serde_json::json!({
        "active": [], "stale": []
      }));
    }
    println!("the reading room is empty (no leases ever taken)");
    return ExitCode::SUCCESS;
  }
  let ns = match ns {
    Some(raw) => {
      let n = kumbarium_librarian::normalize_namespace(raw);
      if let Err(e) = kumbarium_librarian::validate_namespace(&n) {
        return fail(&format!("invalid namespace: {e}"));
      }
      Some(n)
    }
    None => None,
  };
  let now = kumbarium_util::now_ms();
  let ttl = state.cfg.leases_ttl_minutes;
  let conn = match state.leases() {
    Ok(c) => c,
    Err(e) => return fail(&e),
  };
  let active = match kumbarium_leases::active_in(conn, ns.as_deref(), now, ttl)
  {
    Ok(v) => v,
    Err(e) => return fail(&e.to_string()),
  };
  let stale = match kumbarium_leases::stale_in(conn, now, ttl) {
    Ok(v) => v
      .into_iter()
      .filter(|l| ns.as_deref().is_none_or(|n| l.namespace == n))
      .collect::<Vec<_>>(),
    Err(e) => return fail(&e.to_string()),
  };
  if json {
    let row = |l: &kumbarium_leases::Lease| {
      serde_json::json!({
        "id": l.id,
        "namespace": l.namespace,
        "resource": l.resource,
        "agent_id": l.agent_id,
        "session_id": l.session_id,
        "note": l.note,
        "taken_at": l.taken_at,
        "renewed_at": l.renewed_at,
      })
    };
    return print_json(&serde_json::json!({
      "ttl_minutes": ttl,
      "active": active.iter().map(row).collect::<Vec<_>>(),
      "stale": stale.iter().map(row).collect::<Vec<_>>(),
    }));
  }
  println!(
    "{} {}",
    sty.bold("the reading room"),
    sty.dim(&format!("(leases lapse after {ttl} idle minutes)"))
  );
  const COLS: &[Col] = &[
    Col {
      title: "id",
      width: 8,
    },
    Col {
      title: "holder",
      width: 20,
    },
    Col {
      title: "namespace",
      width: 20,
    },
    Col {
      title: "since (local)",
      width: 19,
    },
    Col {
      title: "resource",
      width: 0,
    },
  ];
  if active.is_empty() {
    println!("no active leases; the room is open");
  } else {
    println!("{}", sty.dim(&table_header(COLS)));
    for l in &active {
      let note = l
        .note
        .as_deref()
        .map(|n| format!("; {n}"))
        .unwrap_or_default();
      // The freshness the TTL keys on is RENEWAL, not taking:
      // a card renewed a minute ago is fresh however old its
      // since column reads (docker-ps discipline).
      let active_ago = kumbarium_util::parse_iso8601_ms(&l.renewed_at)
        .map(|ms| {
          let mins = ((now - ms) / 60_000).max(0);
          match mins {
            0 => "active <1m ago".to_string(),
            m if m < 60 => format!("active {m}m ago"),
            m => format!("active {}h{}m ago", m / 60, m % 60),
          }
        })
        .unwrap_or_default();
      let lines = hang(
        body_col(COLS),
        &format!("{}  ({active_ago}{})", l.resource, note),
      );
      let holder = format!(
        "{} ({})",
        l.agent_id,
        kumbarium_leases::short_id(&l.session_id)
      );
      println!(
        "{} {} {} {} {}",
        sty.id(&cell(COLS, 0, kumbarium_leases::short_id(&l.id))),
        cell(COLS, 1, &holder),
        cell(COLS, 2, &l.namespace),
        sty.dim(&cell(COLS, 3, &local_display(&l.taken_at))),
        lines[0]
      );
      for line in &lines[1..] {
        println!("{line}");
      }
    }
  }
  if !stale.is_empty() {
    println!(
      "\n{}",
      sty.yellow(
        "stale (expired, never released; the crashed-agent \
         shape). kum lease break <id> clears one:"
      )
    );
    for l in &stale {
      let holder = format!(
        "{} ({})",
        l.agent_id,
        kumbarium_leases::short_id(&l.session_id)
      );
      println!(
        "  {} {} {}/{} (last active {})",
        sty.id(&cell(COLS, 0, kumbarium_leases::short_id(&l.id))),
        cell(COLS, 1, &holder),
        l.namespace,
        l.resource,
        local_display(&l.renewed_at)
      );
    }
  }
  ExitCode::SUCCESS
}

/// `kum lease break <id>`: the human clears a stuck card,
/// whoever holds it, witnessed with the holder named.
pub(crate) fn lease_break_cmd(id: &str) -> ExitCode {
  let (_, mut state) = match open_stores() {
    Ok(v) => v,
    Err(e) => return fail(&e),
  };
  let sty = style::Style::detect();
  let now = kumbarium_util::now_ms();
  let broken = {
    let conn = match state.leases() {
      Ok(c) => c,
      Err(e) => return fail(&e),
    };
    match kumbarium_leases::break_lease(conn, id, now) {
      Ok(l) => l,
      Err(e) => return fail(&e.to_string()),
    }
  };
  let event = kumbarium_audit::Event {
    agent_id: "kumbarium-cli".into(),
    session_id: state.session_id.clone(),
    kind: kumbarium_audit::EventKind::LeaseBreak,
    scope: broken.namespace.clone(),
    detail: serde_json::json!({
      "id": broken.id,
      "resource": broken.resource,
      "holder": broken.agent_id,
    }),
  };
  if let Err(e) = kumbarium_audit::append(&state.audit, &event) {
    return fail(&format!("broken, but audit append failed: {e}"));
  }
  println!(
    "broke {}'s lease on {}/{} ({})",
    broken.agent_id,
    broken.namespace,
    broken.resource,
    sty.id(kumbarium_leases::short_id(&broken.id))
  );
  ExitCode::SUCCESS
}
