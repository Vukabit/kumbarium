//! The handoff commands (D-036): leave a briefing, read the
//! standing one, survey every shelf's. The chain view rides
//! `kum history` via fall-through, where it reads as the
//! scope's session diary.

use std::process::ExitCode;

use super::super::{open_stores, style, tools};
use super::term::*;

/// `kum handoff <ns> <note...>` writes; `kum handoff <ns>`
/// reads the standing briefing.
pub(crate) fn handoff_cmd(ns: &str, rest: &[&str]) -> ExitCode {
  let ns = kumbarium_librarian::normalize_namespace(ns);
  if let Err(e) = kumbarium_librarian::validate_namespace(&ns) {
    return fail(&format!("invalid namespace: {e}"));
  }
  let (_, mut state) = match open_stores() {
    Ok(v) => v,
    Err(e) => return fail(&e),
  };
  let sty = style::Style::detect();
  if rest.is_empty() {
    // Read the standing briefing.
    let conn = match state.handoff() {
      Ok(c) => c,
      Err(e) => return fail(&e),
    };
    return match kumbarium_handoff::standing(conn, &ns) {
      Ok(Some(h)) => {
        println!(
          "{} {}",
          sty.bold(&format!("standing briefing for {ns}")),
          sty.dim(&format!(
            "(left by {} at {}, id {})",
            h.agent_id,
            local_display(&h.created_at),
            kumbarium_handoff::short_id(&h.id)
          ))
        );
        println!("\n{}", h.content);
        ExitCode::SUCCESS
      }
      Ok(None) => {
        println!("no standing briefing for {ns}");
        ExitCode::SUCCESS
      }
      Err(e) => fail(&e.to_string()),
    };
  }
  match kumbarium_store::namespace_id(&state.library, &ns) {
    Ok(Some(_)) => {}
    Ok(None) => {
      return fail(&format!(
        "namespace {ns:?} is not registered; kumbarium namespace \
         add {ns}"
      ));
    }
    Err(e) => return fail(&e.to_string()),
  }
  let content = rest.join(" ");
  let conn = match state.handoff() {
    Ok(c) => c,
    Err(e) => return fail(&e),
  };
  let h = match kumbarium_handoff::write_handoff(
    conn,
    &ns,
    &content,
    "kumbarium-cli",
    "",
    kumbarium_handoff::Status::Live,
  ) {
    Ok(h) => h,
    Err(e) => return fail(&e.to_string()),
  };
  let event = kumbarium_audit::Event {
    agent_id: "kumbarium-cli".into(),
    session_id: state.session_id.clone(),
    kind: kumbarium_audit::EventKind::HandoffWrite,
    scope: ns.clone(),
    detail: serde_json::json!({ "id": h.id }),
  };
  if let Err(e) = kumbarium_audit::append(&state.audit, &event) {
    return fail(&format!("left, but audit append failed: {e}"));
  }
  println!(
    "briefing left for {ns} ({}); the next session's first \
     recall receives it",
    sty.id(kumbarium_handoff::short_id(&h.id))
  );
  ExitCode::SUCCESS
}

/// `kum handoffs`: every shelf's standing briefing, first line
/// each.
pub(crate) fn handoffs_cmd() -> ExitCode {
  let (_, mut state) = match open_stores() {
    Ok(v) => v,
    Err(e) => return fail(&e),
  };
  let sty = style::Style::detect();
  let conn = match state.handoff() {
    Ok(c) => c,
    Err(e) => return fail(&e),
  };
  let all = match kumbarium_handoff::standings(conn) {
    Ok(v) => v,
    Err(e) => return fail(&e.to_string()),
  };
  if all.is_empty() {
    println!("no standing briefings on any shelf");
    return ExitCode::SUCCESS;
  }
  const COLS: &[Col] = &[
    Col {
      title: "id",
      width: 8,
    },
    Col {
      title: "left (local)",
      width: 19,
    },
    Col {
      title: "by",
      width: 20,
    },
    Col {
      title: "namespace",
      width: 20,
    },
    Col {
      title: "briefing",
      width: 0,
    },
  ];
  println!("{}", sty.dim(&table_header(COLS)));
  for h in &all {
    let first = h.content.lines().next().unwrap_or("");
    let lines = hang(body_col(COLS), first);
    println!(
      "{} {} {} {} {}",
      sty.id(&cell(COLS, 0, kumbarium_handoff::short_id(&h.id))),
      sty.dim(&cell(COLS, 1, &local_display(&h.created_at))),
      cell(COLS, 2, &h.agent_id),
      cell(COLS, 3, &h.namespace),
      lines[0]
    );
    for line in &lines[1..] {
      println!("{line}");
    }
  }
  ExitCode::SUCCESS
}

/// Render one briefing in full (`kum show` fall-through).
pub(crate) fn show_handoff(
  state: &mut tools::ServerState,
  id: &str,
) -> Result<ExitCode, String> {
  let conn = state.handoff()?;
  let full = match kumbarium_handoff::resolve_id(conn, id) {
    Ok(f) => f,
    Err(kumbarium_handoff::HandoffError::HandoffNotFound(_)) => {
      // Fourth shelf in the chain: the restricted stacks.
      return super::secret::show_secret(state, id);
    }
    Err(e) => return Err(e.to_string()),
  };
  let h = kumbarium_handoff::get(conn, &full).map_err(|e| e.to_string())?;
  let sty = style::Style::detect();
  println!("{}", sty.bold("briefing (the handoff shelf)"));
  println!(
    "id:         {} (short: {})",
    h.id,
    kumbarium_handoff::short_id(&h.id)
  );
  println!("namespace:  {}", h.namespace);
  if h.status != kumbarium_handoff::Status::Live {
    println!("status:     {}", sty.yellow(h.status.as_str()));
  }
  println!(
    "left:       {} by {}",
    local_display(&h.created_at),
    h.agent_id
  );
  if let Some(next) = &h.superseded_by {
    println!(
      "superseded: by {} (kum history {} reads the diary)",
      kumbarium_handoff::short_id(next),
      kumbarium_handoff::short_id(&h.id)
    );
  }
  println!("\n{}", h.content);
  Ok(ExitCode::SUCCESS)
}

/// The chain as a diary (`kum history` fall-through).
pub(crate) fn handoff_history_cmd(
  state: &mut tools::ServerState,
  id: &str,
) -> ExitCode {
  let sty = style::Style::detect();
  let conn = match state.handoff() {
    Ok(c) => c,
    Err(e) => return fail(&e),
  };
  let full = match kumbarium_handoff::resolve_id(conn, id) {
    Ok(f) => f,
    Err(e) => return fail(&e.to_string()),
  };
  let chain = match kumbarium_handoff::history(conn, &full) {
    Ok(c) => c,
    Err(e) => return fail(&e.to_string()),
  };
  println!(
    "{}",
    sty.bold(&format!(
      "session diary for {} ({} briefings)",
      chain.first().map(|h| h.namespace.as_str()).unwrap_or("?"),
      chain.len()
    ))
  );
  for (i, h) in chain.iter().enumerate() {
    let head =
      h.superseded_by.is_none() && h.status == kumbarium_handoff::Status::Live;
    let marker = if head { "standing" } else { "        " };
    println!(
      "\nv{} {} {} {} {}",
      i + 1,
      sty.dim(marker),
      sty.id(kumbarium_handoff::short_id(&h.id)),
      sty.dim(&local_display(&h.created_at)),
      sty.dim(&h.agent_id)
    );
    println!("{}", h.content.lines().next().unwrap_or(""));
  }
  ExitCode::SUCCESS
}

/// Judge a pending briefing with the desk's verbs (approval
/// makes it THE standing head, superseding the live one: the
/// one-live-head invariant survives the desk).
pub(crate) fn judge_handoff(
  state: &mut tools::ServerState,
  id: &str,
  approving: bool,
  reason: Option<String>,
) -> ExitCode {
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
  let result = if approving {
    kumbarium_handoff::approve(conn, &full)
  } else {
    kumbarium_handoff::reject(conn, &full)
  };
  if let Err(e) = result {
    return fail(&e.to_string());
  }
  let mut detail = serde_json::json!({
    "id": full,
    "submitter": h.agent_id,
  });
  if let Some(reason) = &reason {
    detail["reason"] = serde_json::json!(reason);
  }
  let event = kumbarium_audit::Event {
    agent_id: "kumbarium-cli".into(),
    session_id: state.session_id.clone(),
    kind: if approving {
      kumbarium_audit::EventKind::Approve
    } else {
      kumbarium_audit::EventKind::Reject
    },
    scope: h.namespace.clone(),
    detail,
  };
  if let Err(e) = kumbarium_audit::append(&state.audit, &event) {
    return fail(&format!("judged, but audit append failed: {e}"));
  }
  if approving {
    println!(
      "approved briefing {}: now standing for {}",
      sty.id(kumbarium_handoff::short_id(&full)),
      h.namespace
    );
  } else {
    println!(
      "rejected briefing {} (kept for the record)",
      sty.id(kumbarium_handoff::short_id(&full))
    );
  }
  ExitCode::SUCCESS
}
