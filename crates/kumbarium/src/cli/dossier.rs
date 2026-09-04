//! The dossier (D-042): `kum dossier <agent>` renders one
//! agent's whole witnessed story as a deterministic postmortem:
//! what it was served, what it wrote and how those writes
//! fared, what the desk judged, what credentials it touched or
//! was refused, and the chronological record itself, with the
//! hash chain verified up front so the page states its own
//! trustworthiness. The binder's sibling on the other axis: the
//! binder reads a SCOPE, the dossier reads an AGENT. Pure
//! rendering, nothing written, not witnessed (browsing is not
//! circulation).

use std::collections::BTreeSet;
use std::process::ExitCode;

use super::super::{open_stores, style};
use super::term::*;

/// Everything the ledger says about one agent in one window,
/// tallied in a single pass.
#[derive(Default)]
struct Tally {
  events: usize,
  recalls: usize,
  scopes: BTreeSet<String>,
  served_ids: BTreeSet<String>,
  briefings_served: usize,
  matters_served: usize,
  remembers: usize,
  supersedes: usize,
  tasks_filed: usize,
  briefings_left: usize,
  approved: usize,
  rejected: usize,
  secret_reads: Vec<String>,
  secret_refused: Vec<String>,
  secret_execs: usize,
  secret_copies: usize,
}

fn within(at: &str, since: Option<&str>, until: Option<&str>) -> bool {
  let day = at.get(..10).unwrap_or(at);
  since.is_none_or(|s| day >= s) && until.is_none_or(|u| day <= u)
}

/// A calendar day, the docket-goal grammar.
fn valid_date(date: &str) -> Result<(), String> {
  let ok = date.len() == 10
    && kumbarium_util::parse_iso8601_ms(&format!("{date}T00:00:00.000Z"))
      .is_some();
  match ok {
    true => Ok(()),
    false => Err(format!("invalid date {date:?}; use YYYY-MM-DD")),
  }
}

pub(crate) fn dossier_cmd(agent: &str, rest: &[&str]) -> ExitCode {
  let mut since: Option<String> = None;
  let mut until: Option<String> = None;
  let mut session: Option<String> = None;
  let mut it = rest.iter();
  while let Some(flag) = it.next() {
    if *flag == "--session" {
      match it.next() {
        Some(frag) => session = Some((*frag).to_string()),
        None => return fail("--session needs an id fragment"),
      }
      continue;
    }
    let slot = match *flag {
      "--since" => &mut since,
      "--until" => &mut until,
      other => return fail(&format!("unknown flag {other:?}")),
    };
    match it.next() {
      Some(date) => {
        if let Err(e) = valid_date(date) {
          return fail(&e);
        }
        *slot = Some((*date).to_string());
      }
      None => return fail(&format!("{flag} needs YYYY-MM-DD")),
    }
  }
  let (_, state) = match open_stores() {
    Ok(v) => v,
    Err(e) => return fail(&e),
  };
  let sty = style::Style::detect();

  // The chain check leads: a dossier that cannot vouch for its
  // own source says so before saying anything else.
  let verified = match kumbarium_audit::verify_chain(&state.audit) {
    Ok(kumbarium_audit::ChainStatus::Intact { events, head }) => {
      let head = head.unwrap_or_default();
      format!(
        "ledger verified: chain intact ({events} events, head {})",
        head.get(..12).unwrap_or(&head)
      )
    }
    Ok(kumbarium_audit::ChainStatus::Broken { index, .. }) => format!(
      "ledger COMPROMISED: chain breaks at event {index}; \
       everything below is untrustworthy from there on"
    ),
    Err(e) => return fail(&e.to_string()),
  };

  let events = match kumbarium_audit::events_asc(&state.audit) {
    Ok(v) => v,
    Err(e) => return fail(&e.to_string()),
  };
  let mut t = Tally::default();
  let mut record: Vec<&kumbarium_audit::StoredEvent> = Vec::new();
  let mut sessions: BTreeSet<String> = BTreeSet::new();
  for ev in &events {
    if !within(&ev.at, since.as_deref(), until.as_deref()) {
      continue;
    }
    if let Some(frag) = &session
      && !ev.session_id.contains(frag.as_str())
    {
      continue;
    }
    if ev.agent_id == agent && !ev.session_id.is_empty() {
      sessions.insert(ev.session_id.clone());
    }
    let detail: serde_json::Value =
      serde_json::from_str(&ev.detail).unwrap_or_default();
    // Desk judgments name the agent as SUBMITTER on someone
    // else's event; everything else is the agent's own.
    if ev.kind == "approve" || ev.kind == "reject" {
      let submitter = detail.get("submitter").and_then(|s| s.as_str());
      if submitter == Some(agent) {
        match ev.kind.as_str() {
          "approve" => t.approved += 1,
          _ => t.rejected += 1,
        }
        record.push(ev);
        continue;
      }
    }
    if ev.agent_id != agent {
      continue;
    }
    t.events += 1;
    record.push(ev);
    if !ev.scope.is_empty() {
      t.scopes.insert(ev.scope.clone());
    }
    match ev.kind.as_str() {
      "recall" => {
        t.recalls += 1;
        if let Some(ids) = detail.get("returned").and_then(|r| r.as_array()) {
          for id in ids.iter().filter_map(|x| x.as_str()) {
            t.served_ids.insert(id.to_string());
          }
        }
        if detail
          .get("handoff_served")
          .and_then(|x| x.as_bool())
          .unwrap_or(false)
        {
          t.briefings_served += 1;
        }
        t.matters_served += detail
          .get("matters_served")
          .and_then(|x| x.as_u64())
          .unwrap_or(0) as usize;
      }
      "remember" => t.remembers += 1,
      "supersede" => t.supersedes += 1,
      "task_file" => t.tasks_filed += 1,
      "handoff_write" => t.briefings_left += 1,
      "secret_read" => {
        let name = detail
          .get("name")
          .and_then(|x| x.as_str())
          .unwrap_or("?")
          .to_string();
        let granted = detail
          .get("granted")
          .and_then(|x| x.as_bool())
          .unwrap_or(false);
        if granted {
          t.secret_reads.push(name);
        } else {
          t.secret_refused.push(name);
        }
      }
      "secret_exec" => t.secret_execs += 1,
      "secret_copy" => t.secret_copies += 1,
      _ => {}
    }
  }
  if record.is_empty() {
    println!("no witnessed events for {agent:?} in that window");
    return ExitCode::SUCCESS;
  }

  // The estate: this agent's writes as they stand TODAY (state
  // outlives the window on purpose: a write from last month
  // superseded yesterday is exactly what a postmortem wants).
  let all_entries =
    match kumbarium_store::entries_in(&state.library, None, true) {
      Ok(v) => v,
      Err(e) => return fail(&e.to_string()),
    };
  let mut live = 0usize;
  let mut superseded_by_self = 0usize;
  let mut superseded_by_others = 0usize;
  let mut pending = 0usize;
  let mut rejected_writes = 0usize;
  for e in all_entries.iter().filter(|e| e.agent_id == agent) {
    match e.status {
      kumbarium_store::Status::Pending => pending += 1,
      kumbarium_store::Status::Rejected => rejected_writes += 1,
      kumbarium_store::Status::Live => match &e.superseded_by {
        None => live += 1,
        Some(next) => {
          let successor_agent = kumbarium_store::get(&state.library, next)
            .map(|s| s.agent_id)
            .unwrap_or_default();
          if successor_agent == agent {
            superseded_by_self += 1;
          } else {
            superseded_by_others += 1;
          }
        }
      },
    }
  }

  let mut window = match (&since, &until) {
    (None, None) => "all time".to_string(),
    (Some(s), None) => format!("since {s}"),
    (None, Some(u)) => format!("through {u}"),
    (Some(s), Some(u)) => format!("{s} through {u}"),
  };
  if let Some(frag) = &session {
    window.push_str(&format!(", session ~{frag}"));
  }
  println!("{}", sty.bold(&format!("the dossier: {agent}")));
  println!("{}", sty.dim(&format!("window: {window}")));
  println!("{}", sty.dim(&verified));

  if !sessions.is_empty() {
    let shorts: Vec<&str> = sessions
      .iter()
      .map(|s| s.get(s.len().saturating_sub(8)..).unwrap_or(s))
      .collect();
    println!(
      "{}",
      sty.dim(&format!(
        "{} minted session(s): {} (narrow with --session <frag>)",
        sessions.len(),
        shorts.join(", ")
      ))
    );
  }
  println!("\n{}", sty.bold("what it was served"));
  println!(
    "  {} across {} ({} distinct entries)",
    count(t.recalls, "recall"),
    count(t.scopes.len(), "scope"),
    t.served_ids.len()
  );
  println!(
    "  briefings served: {}; matters served: {}",
    t.briefings_served, t.matters_served
  );

  println!("\n{}", sty.bold("what it wrote, and how it fared"));
  println!(
    "  {} witnessed in window; the estate as it stands: {live} \
     live, {pending} pending, {rejected_writes} rejected",
    count(t.remembers, "memory write"),
  );
  println!(
    "  revised by itself: {superseded_by_self}; corrected by \
     OTHERS: {superseded_by_others} (the survival fact)"
  );
  println!(
    "  supersedes it performed: {}; tasks filed: {}; briefings \
     left: {}",
    t.supersedes, t.tasks_filed, t.briefings_left
  );
  if t.approved + t.rejected > 0 {
    println!(
      "  the desk's judgment of its submissions: {} approved, \
       {} rejected",
      t.approved, t.rejected
    );
  }

  if !t.secret_reads.is_empty()
    || !t.secret_refused.is_empty()
    || t.secret_execs + t.secret_copies > 0
  {
    println!("\n{}", sty.bold("the restricted stacks"));
    if !t.secret_reads.is_empty() {
      println!(
        "  reads granted: {} ({})",
        t.secret_reads.len(),
        t.secret_reads.join(", ")
      );
    }
    if !t.secret_refused.is_empty() {
      println!(
        "  {} {} ({})",
        sty.red("REFUSED:"),
        t.secret_refused.len(),
        t.secret_refused.join(", ")
      );
    }
    if t.secret_execs + t.secret_copies > 0 {
      println!(
        "  redacted execs: {}; concealed copies: {}",
        t.secret_execs, t.secret_copies
      );
    }
  }

  const COLS: &[Col] = &[
    Col {
      title: "at (local)",
      width: 19,
    },
    Col {
      title: "kind",
      width: 15,
    },
    Col {
      title: "scope",
      width: 20,
    },
    Col {
      title: "detail",
      width: 0,
    },
  ];
  println!("\n{}", sty.bold("the record, oldest first"));
  println!("{}", sty.dim(&table_header(COLS)));
  for ev in &record {
    let detail = kumbarium_audit::describe_event(&ev.kind, &ev.detail);
    let lines = hang(body_col(COLS), &detail);
    println!(
      "{} {} {} {}",
      sty.dim(&cell(COLS, 0, &local_display(&ev.at))),
      sty.event(&cell(COLS, 1, &ev.kind)),
      cell(COLS, 2, &ev.scope),
      lines[0]
    );
    for line in &lines[1..] {
      println!("{line}");
    }
  }
  ExitCode::SUCCESS
}

fn count(n: usize, noun: &str) -> String {
  match n {
    1 => format!("1 {noun}"),
    _ => format!("{n} {noun}s"),
  }
}
