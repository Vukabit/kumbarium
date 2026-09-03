//! The circulation desk and the janitor: the human-judgment
//! commands (inbox, review, approve, reject, confidence pass).

use std::process::ExitCode;

use super::super::{open_stores, style, tools};
use super::term::*;

/// The confidence pass (D-025): recompute every live entry from
/// the full ledger, preview the proposals, apply only on the
/// --apply sign-off. One batch janitor event witnesses the run.
pub(crate) fn janitor_cmd(apply: bool) -> ExitCode {
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

/// The circulation desk's queue: pending entries, oldest first.
pub(crate) fn inbox_cmd() -> ExitCode {
  let (_, mut state) = match open_stores() {
    Ok(v) => v,
    Err(e) => return fail(&e),
  };
  let sty = style::Style::detect();
  let pending = match kumbarium_store::pending_in(&state.library) {
    Ok(v) => v,
    Err(e) => return fail(&e.to_string()),
  };
  let pending_tasks = match state.docket() {
    Ok(conn) => {
      kumbarium_docket::pending_tasks(conn).unwrap_or_default()
    }
    Err(_) => Vec::new(),
  };
  if pending.is_empty() && pending_tasks.is_empty() {
    println!("inbox empty: nothing awaiting approval");
    return ExitCode::SUCCESS;
  }
  println!(
    "{}",
    sty.dim(
      "id        submitted (local)    agent                \
namespace            content"
    )
  );
  for e in pending {
    let first = e.content.lines().next().unwrap_or("");
    let excerpt: String = first.chars().take(40).collect();
    println!(
      "{}  {}  {:<20} {:<20} {}",
      sty.id(&format!("{:<8}", kumbarium_store::short_id(&e.id))),
      sty.dim(&local_display(&e.created_at)),
      e.agent_id,
      e.namespace,
      excerpt
    );
  }
  if !pending_tasks.is_empty() {
    println!(
      "\n{}",
      sty.dim(
        "pending tasks (the docket's queue):\nid        \
submitted (local)    agent                namespace            \
matter"
      )
    );
    for t in &pending_tasks {
      let first = t.content.lines().next().unwrap_or("");
      let excerpt: String = first.chars().take(40).collect();
      println!(
        "{}  {}  {:<20} {:<20} [{}] {}",
        sty.id(&format!(
          "{:<8}",
          kumbarium_docket::short_id(&t.id)
        )),
        sty.dim(&local_display(&t.created_at)),
        t.agent_id,
        t.namespace,
        t.severity.as_str(),
        excerpt
      );
    }
  }
  println!(
    "\nreview with: kum review <id>; then kum approve <id> or \
     kum reject <id> [reason]"
  );
  ExitCode::SUCCESS
}

/// The full view a judgment deserves: content, provenance, and
/// the collision surface (live near-matches already shelved in
/// the target scope). Never the writer's self-description.
pub(crate) fn review_cmd(id: &str) -> ExitCode {
  let (_, state) = match open_stores() {
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
  if e.status != kumbarium_store::Status::Pending {
    return fail(&format!(
      "{} is {}, not pending; the desk judges only pending \
       entries",
      kumbarium_store::short_id(&full),
      e.status.as_str()
    ));
  }
  println!("{}", sty.bold("pending entry"));
  println!(
    "id:         {} (short: {})",
    e.id,
    kumbarium_store::short_id(&e.id)
  );
  println!("namespace:  {}", e.namespace);
  println!("kind:       {}", e.kind.as_str());
  println!(
    "submitted:  {} by {}",
    local_display(&e.created_at),
    e.agent_id
  );
  if !e.source.is_empty() {
    println!("source:     {}", e.source);
  }
  if !e.tags.is_empty() {
    println!("tags:       {}", e.tags.join(", "));
  }
  println!("\n{}", e.content);
  // Collision surface: what the target shelf already holds that
  // this candidate may duplicate or contradict.
  let chain = match kumbarium_librarian::namespace_chain(&e.namespace) {
    Ok(c) => c,
    Err(err) => return fail(&format!("namespace chain: {err}")),
  };
  match kumbarium_store::recall(&state.library, &e.content, &chain, 5) {
    Ok(hits) => {
      let hits: Vec<_> =
        hits.into_iter().filter(|h| h.entry.id != e.id).collect();
      if hits.is_empty() {
        println!(
          "\n{}",
          sty.dim("collision surface: no live near-matches in scope")
        );
      } else {
        println!(
          "\n{}",
          sty.bold("collision surface (live near-matches in scope):")
        );
        for h in hits {
          let first = h.entry.content.lines().next().unwrap_or("");
          let excerpt: String = first.chars().take(56).collect();
          println!(
            "  {}  {:<20} {}",
            sty.id(&format!("{:<8}", kumbarium_store::short_id(&h.entry.id))),
            h.entry.namespace,
            excerpt
          );
        }
      }
    }
    Err(err) => return fail(&err.to_string()),
  }
  println!(
    "\njudge with: kum approve {} or kum reject {} [reason]",
    kumbarium_store::short_id(&e.id),
    kumbarium_store::short_id(&e.id)
  );
  ExitCode::SUCCESS
}

/// Approve or reject one pending entry (human-only; witnessed).
pub(crate) fn judge_cmd(
  id: &str,
  approving: bool,
  reason: Option<String>,
) -> ExitCode {
  let (_, mut state) = match open_stores() {
    Ok(v) => v,
    Err(e) => return fail(&e),
  };
  let sty = style::Style::detect();
  let full = match kumbarium_store::resolve_id(&state.library, id) {
    Ok(f) => f,
    Err(kumbarium_store::StoreError::EntryNotFound(_)) => {
      // Not on the memory shelf: the docket's queue shares the
      // desk (D-032).
      return judge_task(&mut state, id, approving, reason);
    }
    Err(e) => return fail(&e.to_string()),
  };
  let e = match kumbarium_store::get(&state.library, &full) {
    Ok(e) => e,
    Err(err) => return fail(&err.to_string()),
  };
  let result = if approving {
    kumbarium_store::approve(&state.library, &full)
  } else {
    kumbarium_store::reject(&state.library, &full)
  };
  if let Err(err) = result {
    return fail(&err.to_string());
  }
  let mut detail = serde_json::json!({
    "id": full,
    "submitter": e.agent_id,
  });
  if let Some(reason) = &reason {
    detail["reason"] = serde_json::json!(reason);
  }
  let event = kumbarium_audit::Event {
    agent_id: "kumbarium-cli".into(),
    kind: if approving {
      kumbarium_audit::EventKind::Approve
    } else {
      kumbarium_audit::EventKind::Reject
    },
    scope: e.namespace.clone(),
    detail,
  };
  if let Err(err) = kumbarium_audit::append(&state.audit, &event) {
    return fail(&format!("judged, but audit append failed: {err}"));
  }
  if approving {
    println!(
      "approved {}: now live in {}",
      sty.id(kumbarium_store::short_id(&full)),
      e.namespace
    );
  } else {
    println!(
      "rejected {} (kept for the record; forget removes wrong \
       or sensitive content)",
      sty.id(kumbarium_store::short_id(&full))
    );
  }
  ExitCode::SUCCESS
}

/// Judge a pending docket task with the desk's verbs.
fn judge_task(
  state: &mut tools::ServerState,
  id: &str,
  approving: bool,
  reason: Option<String>,
) -> ExitCode {
  let sty = style::Style::detect();
  let conn = match state.docket() {
    Ok(c) => c,
    Err(e) => return fail(&e),
  };
  let full = match kumbarium_docket::resolve_id(conn, id) {
    Ok(f) => f,
    Err(e) => return fail(&e.to_string()),
  };
  let task = match kumbarium_docket::get(conn, &full) {
    Ok(t) => t,
    Err(e) => return fail(&e.to_string()),
  };
  let result = if approving {
    kumbarium_docket::approve(conn, &full)
  } else {
    kumbarium_docket::reject(conn, &full)
  };
  if let Err(e) = result {
    return fail(&e.to_string());
  }
  let mut detail = serde_json::json!({
    "id": full,
    "submitter": task.agent_id,
  });
  if let Some(reason) = &reason {
    detail["reason"] = serde_json::json!(reason);
  }
  let event = kumbarium_audit::Event {
    agent_id: "kumbarium-cli".into(),
    kind: if approving {
      kumbarium_audit::EventKind::Approve
    } else {
      kumbarium_audit::EventKind::Reject
    },
    scope: task.namespace.clone(),
    detail,
  };
  if let Err(e) = kumbarium_audit::append(&state.audit, &event) {
    return fail(&format!("judged, but audit append failed: {e}"));
  }
  if approving {
    println!(
      "approved task {}: now on the docket in {}",
      sty.id(kumbarium_docket::short_id(&full)),
      task.namespace
    );
  } else {
    println!(
      "rejected task {} (kept for the record)",
      sty.id(kumbarium_docket::short_id(&full))
    );
  }
  ExitCode::SUCCESS
}
