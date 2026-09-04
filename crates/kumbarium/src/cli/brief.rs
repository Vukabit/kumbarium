//! The binder (D-041): `kum brief <scope>` renders the state of
//! a shelf as one page a person (or a freshly onboarded agent)
//! can ingest before touching anything: the shelf's charter,
//! the standing facts that earned their place, the briefing the
//! last session left, the matters that will not wait, and what
//! the restricted stacks hold (names, never values). Pure
//! read-side: every ingredient already lives on a shelf; the
//! binder is a rendering, not a record, so it is not witnessed
//! (browsing is not circulation; recall is).

use std::process::ExitCode;

use super::super::{open_stores, style};
use super::term::*;

/// Standing facts shown before the binder says "and more".
const FACT_CAP: usize = 10;
/// Open matters shown before the binder points at the docket.
const MATTER_CAP: usize = 8;

/// Rank the collection for the binder: confidence first (what
/// survived circulation), recency as the tiebreak. Deterministic
/// and cheap; D-026 is not violated because the binder is a
/// briefing surface, not retrieval (recall stays bm25-only).
fn rank_facts(entries: &mut [kumbarium_store::Entry]) {
  entries.sort_by(|a, b| {
    b.confidence
      .partial_cmp(&a.confidence)
      .unwrap_or(std::cmp::Ordering::Equal)
      .then_with(|| b.updated_at.cmp(&a.updated_at))
  });
}

/// Open matters rank by what will not wait: urgency first, then
/// the nearest goal, goalless matters last, oldest first inside
/// a band.
fn rank_matters(tasks: &mut [kumbarium_docket::Task]) {
  tasks.sort_by(|a, b| {
    let sev = |t: &kumbarium_docket::Task| match t.severity {
      kumbarium_docket::Severity::Urgent => 0,
      kumbarium_docket::Severity::High => 1,
      kumbarium_docket::Severity::Normal => 2,
      kumbarium_docket::Severity::Low => 3,
    };
    sev(a).cmp(&sev(b)).then_with(|| match (&a.goal, &b.goal) {
      (Some(x), Some(y)) => x.cmp(y),
      (Some(_), None) => std::cmp::Ordering::Less,
      (None, Some(_)) => std::cmp::Ordering::Greater,
      (None, None) => a.created_at.cmp(&b.created_at),
    })
  });
}

pub(crate) fn brief_cmd(ns: &str) -> ExitCode {
  let ns = kumbarium_librarian::normalize_namespace(ns);
  if let Err(e) = kumbarium_librarian::validate_namespace(&ns) {
    return fail(&format!("invalid namespace: {e}"));
  }
  let chain = match kumbarium_librarian::namespace_chain(&ns) {
    Ok(c) => c,
    Err(e) => return fail(&e.to_string()),
  };
  let (p, mut state) = match open_stores() {
    Ok(v) => v,
    Err(e) => return fail(&e),
  };
  let sty = style::Style::detect();
  let registered = match kumbarium_store::namespaces(&state.library) {
    Ok(rows) => rows,
    Err(e) => return fail(&e.to_string()),
  };
  let description = registered
    .iter()
    .find(|(path, _, _)| *path == ns)
    .map(|(_, d, _)| d.clone());
  let Some(description) = description else {
    return fail(&format!(
      "namespace {ns:?} is not registered; kumbarium namespace \
       add {ns}"
    ));
  };

  // The charter line.
  println!("{}", sty.bold(&format!("the binder: {ns}")));
  if !description.is_empty() {
    println!("{}", sty.dim(&description));
  }
  println!(
    "{}",
    sty.dim(&format!("scope chain: {}", chain.join(" -> ")))
  );

  // Standing facts, chain-wide, survivors first.
  let mut facts: Vec<kumbarium_store::Entry> = Vec::new();
  for scope in &chain {
    match kumbarium_store::entries_in(&state.library, Some(scope), false) {
      Ok(mut rows) => facts.append(&mut rows),
      Err(e) => return fail(&e.to_string()),
    }
  }
  let total = facts.len();
  rank_facts(&mut facts);
  println!("\n{}", sty.bold("standing facts (survivors first)"));
  if facts.is_empty() {
    println!("  the shelf is empty; first session writes the charter");
  }
  // One line per fact: the binder is a table of contents, not
  // the book (kum show reads any of them in full), so overflow
  // truncates rather than wrapping.
  const FACT_COLS: &[Col] = &[
    Col {
      title: "",
      width: 1,
    },
    Col {
      title: "id",
      width: 8,
    },
    Col {
      title: "conf",
      width: 4,
    },
    Col {
      title: "kind",
      width: 13,
    },
    Col {
      title: "namespace",
      width: 20,
    },
    Col {
      title: "fact",
      width: 0,
    },
  ];
  let fact_width = term_width()
    .filter(|w| *w > body_col(FACT_COLS) + 16)
    .map(|w| w - body_col(FACT_COLS));
  for e in facts.iter().take(FACT_CAP) {
    let first = e.content.lines().next().unwrap_or("");
    let first = match fact_width {
      Some(w) if first.chars().count() > w => {
        let cut: String = first.chars().take(w.saturating_sub(3)).collect();
        format!("{cut}...")
      }
      _ => first.to_string(),
    };
    println!(
      "  {} {:.2} {} {} {first}",
      sty.id(&cell(FACT_COLS, 1, kumbarium_store::short_id(&e.id))),
      e.confidence,
      sty.kind(&cell(FACT_COLS, 3, e.kind.as_str())),
      sty.dim(&cell(FACT_COLS, 4, &e.namespace)),
    );
  }
  if total > FACT_CAP {
    println!(
      "  {}",
      sty.dim(&format!("and {} more (kum list {ns})", total - FACT_CAP))
    );
  }

  // The standing briefing: what the last session left behind.
  println!("\n{}", sty.bold("the standing briefing"));
  match state.handoff() {
    Ok(conn) => match kumbarium_handoff::standing(conn, &ns) {
      Ok(Some(h)) => {
        println!(
          "  {}",
          sty.dim(&format!(
            "left by {} at {}:",
            h.agent_id,
            local_display(&h.created_at)
          ))
        );
        for line in indent_block(2, &h.content) {
          println!("{line}");
        }
      }
      Ok(None) => println!("  none standing for this shelf"),
      Err(e) => return fail(&e.to_string()),
    },
    Err(e) => return fail(&e),
  }

  // Matters that will not wait.
  let mut matters: Vec<kumbarium_docket::Task> = Vec::new();
  if p.docket_db.exists() {
    match state.docket() {
      Ok(conn) => match kumbarium_docket::tasks_in(conn, Some(&chain), false) {
        Ok(rows) => matters = rows,
        Err(e) => return fail(&e.to_string()),
      },
      Err(e) => return fail(&e),
    }
  }
  let open = matters.len();
  rank_matters(&mut matters);
  println!("\n{}", sty.bold("open matters"));
  if matters.is_empty() {
    println!("  the docket is clear in this scope");
  }
  const MATTER_COLS: &[Col] = &[
    Col {
      title: "",
      width: 1,
    },
    Col {
      title: "id",
      width: 8,
    },
    Col {
      title: "sev",
      width: 6,
    },
    Col {
      title: "matter",
      width: 0,
    },
  ];
  for t in matters.iter().take(MATTER_CAP) {
    let goal = t
      .goal
      .as_deref()
      .map(|g| format!(" (goal {g})"))
      .unwrap_or_default();
    let first = t.content.lines().next().unwrap_or("");
    let lines = hang(body_col(MATTER_COLS), &format!("{first}{goal}"));
    println!(
      "  {} {} {}",
      sty.id(&cell(MATTER_COLS, 1, kumbarium_docket::short_id(&t.id))),
      cell(MATTER_COLS, 2, t.severity.as_str()),
      lines[0],
    );
    for line in &lines[1..] {
      println!("{line}");
    }
  }
  if open > MATTER_CAP {
    println!(
      "  {}",
      sty.dim(&format!("and {} more (kum tasks {ns})", open - MATTER_CAP))
    );
  }

  // The reading room: who is at work in this chain, now.
  let ttl = state.cfg.leases_ttl_minutes;
  if p.leases_db.exists()
    && let Ok(conn) = state.leases()
  {
    let now = kumbarium_util::now_ms();
    let mut cards = Vec::new();
    for scope in &chain {
      if let Ok(mut v) =
        kumbarium_leases::active_in(conn, Some(scope), now, ttl)
      {
        cards.append(&mut v);
      }
    }
    if !cards.is_empty() {
      println!("\n{}", sty.bold("the reading room (agents at work)"));
      for l in &cards {
        println!(
          "  {} (session {}) holds {}/{} (since {})",
          l.agent_id,
          kumbarium_leases::short_id(&l.session_id),
          l.namespace,
          l.resource,
          local_display(&l.taken_at)
        );
      }
    }
  }

  // The restricted stacks: names only, structurally.
  if p.secrets_db.exists()
    && let Ok(conn) = state.secrets()
  {
    let mut held: Vec<String> = Vec::new();
    for scope in &chain {
      if let Ok(rows) = kumbarium_secrets::list(conn, Some(scope)) {
        for m in rows {
          let expiry = m
            .expires_at
            .map(|d| format!(" (expires {d})"))
            .unwrap_or_default();
          held.push(format!("{}/{}{expiry}", m.namespace, m.name));
        }
      }
    }
    if !held.is_empty() {
      println!("\n{}", sty.bold("the restricted stacks hold"));
      for line in held {
        println!("  {line}");
      }
      println!(
        "  {}",
        sty.dim("values never print here; kum secret read is witnessed")
      );
    }
  }
  println!(
    "\n{}",
    sty.dim(
      "the binder is a rendering, not a record: kum recall \
       serves agents, this page serves you"
    )
  );
  ExitCode::SUCCESS
}

#[cfg(test)]
mod tests {
  use super::*;

  fn entry(conf: f64, updated: &str) -> kumbarium_store::Entry {
    kumbarium_store::Entry {
      id: kumbarium_util::generate_id(),
      namespace: "global".into(),
      kind: kumbarium_store::Kind::Decision,
      content: "x".into(),
      agent_id: "t".into(),
      source: String::new(),
      confidence: conf,
      confidence_basis: None,
      superseded_by: None,
      created_at: updated.into(),
      updated_at: updated.into(),
      last_accessed_at: None,
      last_confirmed_at: None,
      retired_at: None,
      note: None,
      status: kumbarium_store::Status::Live,
      tags: vec![],
    }
  }

  #[test]
  fn facts_rank_by_survival_then_recency() {
    let mut v = vec![
      entry(0.50, "2026-09-03T00:00:00.000Z"),
      entry(0.65, "2026-09-01T00:00:00.000Z"),
      entry(0.50, "2026-09-02T00:00:00.000Z"),
    ];
    rank_facts(&mut v);
    assert_eq!(v[0].confidence, 0.65);
    assert_eq!(v[1].updated_at, "2026-09-03T00:00:00.000Z");
  }
}
