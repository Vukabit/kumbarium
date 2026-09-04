//! The circulation desk and the janitor: the human-judgment
//! commands (inbox, review, approve, reject, confidence pass).

use std::process::ExitCode;

use super::super::{open_stores, style, tools};
use super::term::*;

/// The confidence pass (D-025): recompute every live entry from
/// the full ledger, preview the proposals, apply only on the
/// --apply sign-off. One batch janitor event witnesses the run.
pub(crate) fn janitor_cmd(apply: bool) -> ExitCode {
  let (p, mut state) = match open_stores() {
    Ok(v) => v,
    Err(e) => return fail(&e),
  };
  let sty = style::Style::detect();
  let events = match kumbarium_audit::events_asc(&state.audit) {
    Ok(v) => v,
    Err(e) => return fail(&e.to_string()),
  };
  let shelves = gather_shelves(&p, &mut state);
  let report = match kumbarium_janitor::pass(
    &state.library,
    &events,
    &shelves,
    state.cfg.janitor_dormant_days,
    kumbarium_util::now_ms(),
  ) {
    Ok(r) => r,
    Err(e) => return fail(&e.to_string()),
  };
  let quiet = report.proposals.is_empty()
    && report.dormant.is_empty()
    && report.pogo.is_empty()
    && report.creep.is_empty()
    && report.unwitnessed_grants.is_empty()
    && report.expired_stock.is_empty();
  if quiet {
    println!("janitor: no changes proposed; evidence unchanged");
    return ExitCode::SUCCESS;
  }
  // The tamper shape leads: everything else can wait a screen.
  if !report.unwitnessed_grants.is_empty() {
    println!(
      "{}",
      sty.red(
        "UNWITNESSED GRANTS: rows in the grants table with no \
         secret_grant event on the ledger. Something wrote \
         around the librarian; treat as tampering until \
         explained:"
      )
    );
    for g in &report.unwitnessed_grants {
      println!("  {}/{} -> {}", g.namespace, g.name, g.agent_id);
    }
    println!(
      "  (kum secret revoke removes them, witnessed; then \
       rotate the credentials involved)\n"
    );
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
  if !report.expired_stock.is_empty() {
    println!("\nexpired credentials still stocked; rotation owed:");
    for e in &report.expired_stock {
      println!(
        "  {}/{} {}",
        e.namespace,
        e.name,
        sty.red(&format!("expired {}", e.expires_at))
      );
    }
  }
  if !report.creep.is_empty() {
    println!("\ncreeping matters (the goal moved later twice or more):");
    for c in &report.creep {
      println!(
        "  {}  {:<20} {} slips, now {}",
        sty.id(&format!("{:<8}", kumbarium_docket::short_id(&c.id))),
        c.namespace,
        c.slips,
        c.goal.as_deref().unwrap_or("(no goal)")
      );
    }
  }
  if !report.pogo.is_empty() {
    println!(
      "\nserved-then-corrected (the library handed out a fact \
       that was superseded within 48h):"
    );
    for pg in &report.pogo {
      println!(
        "  {}  {:<20} corrected {}h after serving",
        sty.id(&format!("{:<8}", kumbarium_store::short_id(&pg.id))),
        pg.scope,
        pg.gap_hours
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
      "pogo": report.pogo.len(),
      "creep": report.creep.len(),
      "unwitnessed_grants": report.unwitnessed_grants.len(),
      "expired_stock": report.expired_stock.len(),
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

/// Extract the v2 shelf inputs (goal chains, grants, secret
/// expiry metadata; never values) so the janitor pass stays
/// pure computation. Missing shelves mean empty inputs.
fn gather_shelves(
  p: &super::super::paths::Paths,
  state: &mut tools::ServerState,
) -> kumbarium_janitor::Shelves {
  let mut shelves = kumbarium_janitor::Shelves::default();
  if p.docket_db.exists()
    && let Ok(conn) = state.docket()
    && let Ok(open) = kumbarium_docket::tasks_in(conn, None, false)
  {
    for t in open {
      let Ok(chain) = kumbarium_docket::history(conn, &t.id) else {
        continue;
      };
      shelves.goal_chains.push(kumbarium_janitor::GoalChain {
        id: t.id.clone(),
        namespace: t.namespace.clone(),
        goals: chain.iter().filter_map(|v| v.goal.clone()).collect(),
      });
    }
  }
  if p.secrets_db.exists()
    && let Ok(conn) = state.secrets()
  {
    if let Ok(grants) = kumbarium_secrets::grants(conn, None) {
      for g in grants {
        shelves.grants.push(kumbarium_janitor::GrantRow {
          namespace: g.namespace,
          name: g.name,
          agent_id: g.agent_id,
        });
      }
    }
    if let Ok(live) = kumbarium_secrets::list(conn, None) {
      for m in live {
        shelves.secrets.push(kumbarium_janitor::SecretStock {
          namespace: m.namespace,
          name: m.name,
          expires_at: m.expires_at,
        });
      }
    }
  }
  shelves
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
    Ok(conn) => kumbarium_docket::pending_tasks(conn).unwrap_or_default(),
    Err(_) => Vec::new(),
  };
  let pending_briefs = match state.handoff() {
    Ok(conn) => kumbarium_handoff::pending_handoffs(conn).unwrap_or_default(),
    Err(_) => Vec::new(),
  };
  if pending.is_empty() && pending_tasks.is_empty() && pending_briefs.is_empty()
  {
    println!("inbox empty: nothing awaiting approval");
    return ExitCode::SUCCESS;
  }
  const INBOX_COLS: &[Col] = &[
    Col {
      title: "id",
      width: 8,
    },
    Col {
      title: "submitted (local)",
      width: 19,
    },
    Col {
      title: "agent",
      width: 20,
    },
    Col {
      title: "namespace",
      width: 20,
    },
    Col {
      title: "content",
      width: 0,
    },
  ];
  println!("{}", sty.dim(&table_header(INBOX_COLS)));
  for e in pending {
    let first = e.content.lines().next().unwrap_or("");
    let excerpt: String = first.chars().take(40).collect();
    println!(
      "{} {} {} {} {}",
      sty.id(&cell(INBOX_COLS, 0, kumbarium_store::short_id(&e.id))),
      sty.dim(&cell(INBOX_COLS, 1, &local_display(&e.created_at))),
      cell(INBOX_COLS, 2, &e.agent_id),
      cell(INBOX_COLS, 3, &e.namespace),
      excerpt
    );
  }
  if !pending_tasks.is_empty() {
    println!(
      "\n{}",
      sty.dim(&format!(
        "pending tasks (the docket's queue):\n{}",
        table_header(&[
          Col {
            title: "id",
            width: 8
          },
          Col {
            title: "submitted (local)",
            width: 19
          },
          Col {
            title: "agent",
            width: 20
          },
          Col {
            title: "namespace",
            width: 20
          },
          Col {
            title: "matter",
            width: 0
          },
        ])
      ))
    );
    for t in &pending_tasks {
      let first = t.content.lines().next().unwrap_or("");
      let excerpt: String = first.chars().take(40).collect();
      println!(
        "{} {} {} {} [{}] {}",
        sty.id(&cell(INBOX_COLS, 0, kumbarium_docket::short_id(&t.id))),
        sty.dim(&cell(INBOX_COLS, 1, &local_display(&t.created_at))),
        cell(INBOX_COLS, 2, &t.agent_id),
        cell(INBOX_COLS, 3, &t.namespace),
        t.severity.as_str(),
        excerpt
      );
    }
  }
  if !pending_briefs.is_empty() {
    println!(
      "\n{}",
      sty.dim(&format!(
        "pending briefings (NEVER served until approved):\n{}",
        table_header(&[
          Col {
            title: "id",
            width: 8
          },
          Col {
            title: "submitted (local)",
            width: 19
          },
          Col {
            title: "agent",
            width: 20
          },
          Col {
            title: "namespace",
            width: 20
          },
          Col {
            title: "briefing",
            width: 0
          },
        ])
      ))
    );
    for h in &pending_briefs {
      let first = h.content.lines().next().unwrap_or("");
      let excerpt: String = first.chars().take(40).collect();
      println!(
        "{} {} {} {} {}",
        sty.id(&cell(INBOX_COLS, 0, kumbarium_handoff::short_id(&h.id))),
        sty.dim(&cell(INBOX_COLS, 1, &local_display(&h.created_at))),
        cell(INBOX_COLS, 2, &h.agent_id),
        cell(INBOX_COLS, 3, &h.namespace),
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
  let (_, mut state) = match open_stores() {
    Ok(v) => v,
    Err(e) => return fail(&e),
  };
  let sty = style::Style::detect();
  let full = match kumbarium_store::resolve_id(&state.library, id) {
    Ok(f) => f,
    Err(kumbarium_store::StoreError::EntryNotFound(_)) => {
      // Ids are building-wide: review the docket's queue too.
      return review_task(&mut state, id);
    }
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
    Err(kumbarium_docket::DocketError::TaskNotFound(_)) => {
      // Third shelf in the desk's chain (D-034).
      return super::handoff::judge_handoff(state, id, approving, reason);
    }
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

/// Review a pending docket task: the judged view, provenance
/// and severity prominent (an untrusted writer filing urgent is
/// itself a signal).
fn review_task(state: &mut tools::ServerState, id: &str) -> ExitCode {
  let sty = style::Style::detect();
  let conn = match state.docket() {
    Ok(c) => c,
    Err(e) => return fail(&e),
  };
  let full = match kumbarium_docket::resolve_id(conn, id) {
    Ok(f) => f,
    Err(kumbarium_docket::DocketError::TaskNotFound(_)) => {
      return review_handoff(state, id);
    }
    Err(e) => return fail(&e.to_string()),
  };
  let t = match kumbarium_docket::get(conn, &full) {
    Ok(t) => t,
    Err(e) => return fail(&e.to_string()),
  };
  if t.status != kumbarium_docket::Status::Pending {
    return fail(&format!(
      "task {} is {}, not pending; the desk judges only pending \
       matters",
      kumbarium_docket::short_id(&full),
      t.status.as_str()
    ));
  }
  println!("{}", sty.bold("pending task (the docket)"));
  println!(
    "id:         {} (short: {})",
    t.id,
    kumbarium_docket::short_id(&t.id)
  );
  println!("namespace:  {}", t.namespace);
  println!(
    "severity:   {} {}",
    t.severity.as_str(),
    sty.dim("(the filer's claim, not yours yet)")
  );
  println!("goal:       {}", t.goal.as_deref().unwrap_or("none"));
  println!(
    "submitted:  {} by {}",
    local_display(&t.created_at),
    t.agent_id
  );
  if !t.source.is_empty() {
    println!("source:     {}", t.source);
  }
  println!("\n{}", t.content);
  println!(
    "\n{}",
    sty.dim(
      "a task poisons what an agent DOES; weigh the provenance \
       before an urgent stranger jumps the queue"
    )
  );
  println!(
    "\njudge with: kum approve {} or kum reject {} [reason]",
    kumbarium_docket::short_id(&t.id),
    kumbarium_docket::short_id(&t.id)
  );
  ExitCode::SUCCESS
}

/// Review a pending briefing: the sharpest injection surface,
/// so provenance leads (D-036).
fn review_handoff(state: &mut tools::ServerState, id: &str) -> ExitCode {
  let sty = style::Style::detect();
  let conn = match state.handoff() {
    Ok(c) => c,
    Err(e) => return fail(&e),
  };
  let full = match kumbarium_handoff::resolve_id(conn, id) {
    Ok(f) => f,
    Err(kumbarium_handoff::HandoffError::HandoffNotFound(_)) => {
      return fail(&format!("no entry, task, or handoff with id {id:?}"));
    }
    Err(e) => return fail(&e.to_string()),
  };
  let h = match kumbarium_handoff::get(conn, &full) {
    Ok(h) => h,
    Err(e) => return fail(&e.to_string()),
  };
  if h.status != kumbarium_handoff::Status::Pending {
    return fail(&format!(
      "briefing {} is {}, not pending",
      kumbarium_handoff::short_id(&full),
      h.status.as_str()
    ));
  }
  println!("{}", sty.bold("pending briefing (the handoff shelf)"));
  println!(
    "submitted:  {} by {}",
    local_display(&h.created_at),
    h.agent_id
  );
  println!("namespace:  {}", h.namespace);
  println!(
    "id:         {} (short: {})",
    h.id,
    kumbarium_handoff::short_id(&h.id)
  );
  println!("\n{}", h.content);
  println!(
    "\n{}",
    sty.dim(
      "a briefing poisons a session's OPENING FRAME at maximum \
       trust; approving makes this THE standing note the next \
       session receives automatically"
    )
  );
  println!(
    "\njudge with: kum approve {} or kum reject {} [reason]",
    kumbarium_handoff::short_id(&h.id),
    kumbarium_handoff::short_id(&h.id)
  );
  ExitCode::SUCCESS
}
