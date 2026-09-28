//! The agent lifecycle (D-056): the human's verbs over the actor
//! registry. Every act is witnessed; none rewrites the ledger. A
//! rename changes a display name, a merge records where a trail
//! now reads, a retire hides a row. Actors are never removed:
//! their sessions are on the chain for good.

use std::process::ExitCode;

use super::super::{open_stores, tools};
use super::term::*;

fn resolve(
  state: &tools::ServerState,
  typed: &str,
) -> Result<kumbarium_store::Actor, String> {
  kumbarium_store::resolve_actor(&state.library, typed)
    .map_err(|e| e.to_string())
}

fn check_new_name(name: &str) -> Result<(), String> {
  kumbarium_store::validate_actor_name(name).map_err(|e| e.to_string())?;
  if tools::reserved_agent_word(name) {
    return Err(format!(
      "{name:?} is reserved for the agent lifecycle; pick another name"
    ));
  }
  Ok(())
}

fn witness(
  state: &mut tools::ServerState,
  kind: kumbarium_audit::EventKind,
  detail: serde_json::Value,
) -> Result<(), String> {
  let event = kumbarium_audit::Event {
    agent_id: "kumbarium-cli".into(),
    session_id: state.session_id.clone(),
    kind,
    scope: String::new(),
    detail,
  };
  state
    .witness(&event)
    .map(|_| ())
    .map_err(|e| format!("done, but audit append failed: {e}"))
}

/// `kum agent add <name> [--human]`: register an actor ahead of
/// its first session. It binds only when asked for by name
/// (`kum serve --agent <name>` or KUMBARIUM_AGENT).
pub(crate) fn agent_add_cmd(rest: &[&str]) -> ExitCode {
  let (name, human) = match rest {
    [name] => (*name, false),
    [name, "--human"] | ["--human", name] => (*name, true),
    _ => return fail("usage: kumbarium agent add <name> [--human]"),
  };
  if let Err(e) = check_new_name(name) {
    return fail(&e);
  }
  let (_, mut state) = match open_stores() {
    Ok(v) => v,
    Err(e) => return fail(&e),
  };
  let kind = if human {
    kumbarium_store::ActorKind::Human
  } else {
    kumbarium_store::ActorKind::Agent
  };
  let new = kumbarium_store::NewActor {
    name: name.to_string(),
    kind,
    claimed: String::new(),
    workspace: String::new(),
    key: None,
  };
  let actor = match kumbarium_store::mint_actor(&state.library, &new, true) {
    Ok((a, _)) => a,
    Err(e) => return fail(&e.to_string()),
  };
  if let Err(e) = witness(
    &mut state,
    kumbarium_audit::EventKind::ActorMint,
    serde_json::json!({
      "id": actor.id,
      "name": actor.name,
      "kind": actor.kind.as_str(),
      "via": "registered",
    }),
  ) {
    return fail(&e);
  }
  println!(
    "registered {} ({}); pin a client to it with: kum serve --agent {}",
    actor.name,
    kumbarium_store::short_id(&actor.id),
    actor.name
  );
  ExitCode::SUCCESS
}

/// `kum agent rename <actor> <new>`: the display name only.
pub(crate) fn agent_rename_cmd(typed: &str, new_name: &str) -> ExitCode {
  if let Err(e) = check_new_name(new_name) {
    return fail(&e);
  }
  let (_, mut state) = match open_stores() {
    Ok(v) => v,
    Err(e) => return fail(&e),
  };
  let actor = match resolve(&state, typed) {
    Ok(a) => a,
    Err(e) => return fail(&e),
  };
  if let Err(e) =
    kumbarium_store::rename_actor(&state.library, &actor.id, new_name)
  {
    return fail(&e.to_string());
  }
  if let Err(e) = witness(
    &mut state,
    kumbarium_audit::EventKind::ActorRename,
    serde_json::json!({ "id": actor.id, "from": actor.name, "to": new_name }),
  ) {
    return fail(&e);
  }
  println!(
    "renamed {} to {new_name} (the id and every record stay)",
    actor.name
  );
  ExitCode::SUCCESS
}

/// `kum agent merge <from> <into>`: fold one actor's trail into
/// another's. A registry fact: every session keeps its bind.
pub(crate) fn agent_merge_cmd(from: &str, into: &str) -> ExitCode {
  let (_, mut state) = match open_stores() {
    Ok(v) => v,
    Err(e) => return fail(&e),
  };
  let (source, target) = match (resolve(&state, from), resolve(&state, into)) {
    (Ok(s), Ok(t)) => (s, t),
    (Err(e), _) | (_, Err(e)) => return fail(&e),
  };
  let survivor = match kumbarium_store::merge_actor(
    &state.library,
    &source.id,
    &target.id,
  ) {
    Ok(id) => id,
    Err(e) => return fail(&e.to_string()),
  };
  let survivor_name = kumbarium_store::actor_get(&state.library, &survivor)
    .map(|a| a.name)
    .unwrap_or_else(|_| target.name.clone());
  if let Err(e) = witness(
    &mut state,
    kumbarium_audit::EventKind::ActorMerge,
    serde_json::json!({
      "from_id": source.id,
      "into_id": survivor,
      "from": source.name,
      "into": survivor_name,
    }),
  ) {
    return fail(&e);
  }
  println!(
    "merged {} into {survivor_name}: its sessions now read as {survivor_name}'s \
     (the ledger is unchanged)",
    source.name
  );
  ExitCode::SUCCESS
}

/// `kum agent retire|unretire <actor>`: hide from (or return to)
/// the default roster; every record stays.
pub(crate) fn agent_retire_cmd(typed: &str, retire: bool) -> ExitCode {
  let (_, mut state) = match open_stores() {
    Ok(v) => v,
    Err(e) => return fail(&e),
  };
  let actor = match resolve(&state, typed) {
    Ok(a) => a,
    Err(e) => return fail(&e),
  };
  if actor.retired_at.is_some() == retire {
    println!(
      "{} is already {}",
      actor.name,
      if retire { "retired" } else { "on the roster" }
    );
    return ExitCode::SUCCESS;
  }
  if let Err(e) =
    kumbarium_store::set_actor_retired(&state.library, &actor.id, retire)
  {
    return fail(&e.to_string());
  }
  let kind = if retire {
    kumbarium_audit::EventKind::ActorRetire
  } else {
    kumbarium_audit::EventKind::ActorUnretire
  };
  if let Err(e) = witness(
    &mut state,
    kind,
    serde_json::json!({ "id": actor.id, "name": actor.name }),
  ) {
    return fail(&e);
  }
  if retire {
    println!(
      "retired {}: hidden from kum agents (--all shows it); every record stays",
      actor.name
    );
  } else {
    println!("unretired {}: back on the roster", actor.name);
  }
  ExitCode::SUCCESS
}
