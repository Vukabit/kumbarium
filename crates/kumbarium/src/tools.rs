//! The five MCP tools wired to store + librarian + audit. Every
//! call is audited after the operation; an audit failure fails
//! the call (audit completeness over availability, D-007's
//! trade, even in the synchronous writer).

use serde_json::{Value, json};

use super::config::Config;

/// Everything one server process holds: the two database
/// connections, the identity the client declared at initialize
/// time, and the effective tunables.
pub struct ServerState {
  pub library: kumbarium_store::Connection,
  pub audit: kumbarium_store::Connection,
  pub agent_id: String,
  pub cfg: Config,
}

impl ServerState {
  #[cfg(test)]
  pub fn in_memory() -> ServerState {
    ServerState {
      library: kumbarium_store::open_in_memory().unwrap(),
      audit: kumbarium_audit::open_in_memory().unwrap(),
      agent_id: "unknown-agent".into(),
      cfg: Config::default(),
    }
  }
}

/// The tools/list payload: name, description, input schema per
/// tool. Descriptions are written for the agent reading them.
pub fn list() -> Value {
  json!({ "tools": [
    {
      "name": "remember",
      "description": "Store a long-term memory in Kumbarium. \
  Use for durable facts worth recalling in future sessions: user \
  preferences, project decisions, standing constraints. The \
  namespace must already be registered (the user registers \
  namespaces; ask them if yours is missing). Send content whole: \
  the librarian splits oversized content into linked parts \
  itself.",
      "inputSchema": {
        "type": "object",
        "properties": {
          "namespace": {
            "type": "string",
            "description": "Registered namespace path, e.g. \
  'global' or 'project/my-app'. Project facts go in the project \
  namespace; cross-project facts in 'global'."
          },
          "kind": {
            "type": "string",
            "enum": [
              "preference", "project_state", "decision",
              "reference"
            ]
          },
          "content": {
            "type": "string",
            "description": "The fact, one self-contained \
  statement."
          },
          "tags": {
            "type": "array", "items": { "type": "string" }
          },
          "source": {
            "type": "string",
            "description": "Where this came from (session, \
  file, conversation)."
          },
          "links": {
            "type": "array",
            "description": "Edges from this new entry to \
  existing entries (e.g. rel 'continues' for part N of a split \
  memory, 'relates_to' for association).",
            "items": {
              "type": "object",
              "properties": {
                "id": { "type": "string" },
                "rel": {
                  "type": "string",
                  "enum": [
                    "continues", "relates_to", "duplicates",
                    "contradicts"
                  ]
                }
              },
              "required": ["id", "rel"]
            }
          }
        },
        "required": ["namespace", "kind", "content"]
      }
    },
    {
      "name": "link",
      "description": "Create a typed edge between two existing \
  memories: 'continues' (sequence parts), 'relates_to' \
  (association), 'duplicates' or 'contradicts' (flag for human \
  review). Idempotent. Ids may be any unique fragment (e.g. the \
  8-char short form).",
      "inputSchema": {
        "type": "object",
        "properties": {
          "from_id": { "type": "string" },
          "to_id": { "type": "string" },
          "rel": {
            "type": "string",
            "enum": [
              "continues", "relates_to", "duplicates",
              "contradicts"
            ]
          }
        },
        "required": ["from_id", "to_id", "rel"]
      }
    },
    {
      "name": "recall",
      "description": "Search Kumbarium for stored memories \
  relevant to a query. Searches the scope's namespace chain \
  (itself, ancestors, global), never sibling projects. Each hit \
  carries relevance (match strength) and confidence (fact \
  trustworthiness) scores.",
      "inputSchema": {
        "type": "object",
        "properties": {
          "query": { "type": "string" },
          "scope": {
            "type": "string",
            "description": "Namespace to search from, e.g. \
  'project/my-app' or 'global'."
          },
          "limit": { "type": "integer", "minimum": 1 }
        },
        "required": ["query", "scope"]
      }
    },
    {
      "name": "confirm",
      "description": "Record that a recalled memory proved \
  correct in use (you followed it and it worked). Evidence only: \
  it stamps last_confirmed_at and feeds the staleness and future \
  confidence signals; it does NOT change the confidence number. \
  The id may be any unique fragment.",
      "inputSchema": {
        "type": "object",
        "properties": { "id": { "type": "string" } },
        "required": ["id"]
      }
    },
    {
      "name": "supersede",
      "description": "Replace an outdated memory with a \
  corrected one. The old entry is chained forward, never \
  deleted. Use when a recalled fact turns out stale or wrong. \
  A split memory supersedes per PART (fix just the stale part); \
  oversized replacement content is split automatically.",
      "inputSchema": {
        "type": "object",
        "properties": {
          "old_id": { "type": "string" },
          "namespace": { "type": "string" },
          "kind": {
            "type": "string",
            "enum": [
              "preference", "project_state", "decision",
              "reference"
            ]
          },
          "content": { "type": "string" },
          "tags": {
            "type": "array", "items": { "type": "string" }
          },
          "source": { "type": "string" },
          "note": {
            "type": "string",
            "description": "One-line label for this version \
  ('typo fix'). Display metadata only: small noted changes \
  collapse in history, but collapse is decided by the measured \
  diff, never the note."
          }
        },
        "required": ["old_id", "namespace", "kind", "content"]
      }
    },
    {
      "name": "forget",
      "description": "Permanently delete a memory. Escape \
  hatch for wrong or sensitive content only; for routine \
  correction use supersede, which preserves history. The id may \
  be any unique fragment.",
      "inputSchema": {
        "type": "object",
        "properties": { "id": { "type": "string" } },
        "required": ["id"]
      }
    }
  ]})
}

/// Dispatch one tools/call. Returns (text blocks, is_error).
pub fn call(
  state: &mut ServerState,
  name: &str,
  args: &Value,
) -> (Vec<String>, bool) {
  let result = match name {
    "remember" => remember(state, args),
    "recall" => recall(state, args),
    "supersede" => supersede(state, args),
    "forget" => forget(state, args),
    "link" => link(state, args),
    "confirm" => confirm(state, args),
    other => Err(format!("unknown tool {other:?}")),
  };
  match result {
    Ok(blocks) => (blocks, false),
    Err(msg) => (vec![msg], true),
  }
}

fn remember(
  state: &mut ServerState,
  args: &Value,
) -> Result<Vec<String>, String> {
  let mut new = new_entry_args(args)?;
  new.agent_id = state.agent_id.clone();
  let ids = store_split(state, &new, None, None)?;
  let head = ids[0].clone();
  let mut linked = 0usize;
  if let Some(links) = args.get("links").and_then(Value::as_array) {
    for spec in links {
      let (to_frag, rel) = link_spec(spec, "id")?;
      let to_id = resolve(state, to_frag)?;
      kumbarium_store::link(&state.library, &head, &to_id, rel).map_err(
        |e| {
          format!(
            "entry {head} stored, but linking to {to_id} \
             failed: {}",
            describe_store_error(e)
          )
        },
      )?;
      linked += 1;
    }
  }
  audit(
    state,
    kumbarium_audit::EventKind::Remember,
    &new.namespace,
    json!({
      "id": head,
      "parts": ids.len(),
      "kind": new.kind.as_str(),
      "links": linked,
    }),
  )?;
  Ok(vec![render_stored("Remembered", &ids, &new, linked)])
}

/// Store `new`, splitting oversized content into parts chained
/// with `continues` edges (each later part points at its
/// predecessor). Part 1 is the head and, when superseding, the
/// entry that replaces `supersedes`. Returns the part ids in
/// order. The importer shares this path, so every write splits
/// identically regardless of origin.
pub(crate) fn store_split(
  state: &mut ServerState,
  new: &kumbarium_store::NewEntry,
  supersedes: Option<&str>,
  note: Option<&str>,
) -> Result<Vec<String>, String> {
  let parts = kumbarium_librarian::split_for_storage(
    &new.content,
    state.cfg.split_target,
  );
  let mut ids: Vec<String> = Vec::new();
  for part in parts {
    let part_entry = kumbarium_store::NewEntry {
      content: part,
      ..new.clone()
    };
    let stored = match (ids.is_empty(), supersedes) {
      (true, Some(old_id)) => kumbarium_store::supersede(
        &mut state.library,
        old_id,
        &part_entry,
        note,
      ),
      _ => kumbarium_store::remember(&mut state.library, &part_entry),
    }
    .map_err(describe_store_error)?;
    if let Some(prev) = ids.last() {
      kumbarium_store::link(
        &state.library,
        &stored.id,
        prev,
        kumbarium_store::Rel::Continues,
      )
      .map_err(describe_store_error)?;
    }
    ids.push(stored.id);
  }
  Ok(ids)
}

fn render_stored(
  verb: &str,
  ids: &[String],
  new: &kumbarium_store::NewEntry,
  linked: usize,
) -> String {
  let links = if linked > 0 {
    format!(" links={linked}")
  } else {
    String::new()
  };
  if ids.len() == 1 {
    format!(
      "{verb}. id={} namespace={} kind={}{links}",
      ids[0],
      new.namespace,
      new.kind.as_str()
    )
  } else {
    format!(
      "{verb} as {} linked parts (namespace={} kind={}{links}):\n{}",
      ids.len(),
      new.namespace,
      new.kind.as_str(),
      ids.join("\n")
    )
  }
}

fn link(state: &mut ServerState, args: &Value) -> Result<Vec<String>, String> {
  let from_id = resolve(state, required_str(args, "from_id")?)?;
  let (to_frag, rel) = link_spec(args, "to_id")?;
  let to_id = resolve(state, to_frag)?;
  let (from_id, to_id) = (from_id.as_str(), to_id.as_str());
  kumbarium_store::link(&state.library, from_id, to_id, rel)
    .map_err(describe_store_error)?;
  let scope = kumbarium_store::get(&state.library, from_id)
    .map(|e| e.namespace)
    .unwrap_or_default();
  audit(
    state,
    kumbarium_audit::EventKind::Link,
    &scope,
    json!({
      "from_id": from_id,
      "to_id": to_id,
      "rel": rel.as_str(),
    }),
  )?;
  Ok(vec![format!("Linked {from_id} {} {to_id}.", rel.as_str())])
}

/// Resolve an id or unique fragment against the library
/// (git-style; listings show the 8-char short form).
fn resolve(state: &ServerState, fragment: &str) -> Result<String, String> {
  kumbarium_store::resolve_id(&state.library, fragment)
    .map_err(describe_store_error)
}

/// Pull (target id, rel) out of a link object; `id_key` names
/// the field holding the target ("id" on remember, "to_id" on
/// the link tool).
fn link_spec<'a>(
  spec: &'a Value,
  id_key: &str,
) -> Result<(&'a str, kumbarium_store::Rel), String> {
  let to_id = required_str(spec, id_key)?;
  let rel_raw = required_str(spec, "rel")?;
  let rel = kumbarium_store::Rel::parse(rel_raw).ok_or_else(|| {
    format!(
      "unknown rel {rel_raw:?}; one of continues, relates_to, \
         duplicates, contradicts"
    )
  })?;
  Ok((to_id, rel))
}

fn recall(
  state: &mut ServerState,
  args: &Value,
) -> Result<Vec<String>, String> {
  let query = required_str(args, "query")?;
  let scope = required_str(args, "scope")?;
  let limit = args
    .get("limit")
    .and_then(Value::as_u64)
    .map(|v| v as usize)
    .unwrap_or(state.cfg.recall_default_limit);
  let chain = kumbarium_librarian::namespace_chain(scope)
    .map_err(|e| format!("invalid scope: {e}"))?;
  let hits = kumbarium_store::recall(&state.library, query, &chain, limit)
    .map_err(describe_store_error)?;
  let ids: Vec<&str> = hits.iter().map(|h| h.entry.id.as_str()).collect();
  audit(
    state,
    kumbarium_audit::EventKind::Recall,
    scope,
    json!({ "query": query, "returned": ids }),
  )?;
  if hits.is_empty() {
    return Ok(vec![format!(
      "No memories matched {query:?} in scope {scope}."
    )]);
  }
  let mut blocks = vec![format!("{} memor(y/ies) found:", hits.len())];
  for (i, hit) in hits.iter().enumerate() {
    blocks.push(render_hit(&state.library, i + 1, hit));
  }
  Ok(blocks)
}

fn supersede(
  state: &mut ServerState,
  args: &Value,
) -> Result<Vec<String>, String> {
  let old_id = resolve(state, required_str(args, "old_id")?)?;
  let mut new = new_entry_args(args)?;
  new.agent_id = state.agent_id.clone();
  let note = args
    .get("note")
    .and_then(Value::as_str)
    .and_then(kumbarium_librarian::sanitize_note);
  let ids = store_split(state, &new, Some(&old_id), note.as_deref())?;
  audit(
    state,
    kumbarium_audit::EventKind::Supersede,
    &new.namespace,
    json!({
      "old_id": old_id,
      "new_id": ids[0],
      "parts": ids.len(),
      "note": note,
    }),
  )?;
  Ok(vec![format!(
    "Superseded {old_id}. {}",
    render_stored("Stored", &ids, &new, 0)
  )])
}

fn confirm(
  state: &mut ServerState,
  args: &Value,
) -> Result<Vec<String>, String> {
  let id = resolve(state, required_str(args, "id")?)?;
  kumbarium_store::confirm(&state.library, &id)
    .map_err(describe_store_error)?;
  let scope = kumbarium_store::get(&state.library, &id)
    .map(|e| e.namespace)
    .unwrap_or_default();
  audit(
    state,
    kumbarium_audit::EventKind::Confirm,
    &scope,
    json!({ "id": id }),
  )?;
  Ok(vec![format!(
    "Confirmed {id}: evidence recorded (last_confirmed_at)."
  )])
}

fn forget(
  state: &mut ServerState,
  args: &Value,
) -> Result<Vec<String>, String> {
  let id = resolve(state, required_str(args, "id")?)?;
  let id = id.as_str();
  let entry =
    kumbarium_store::get(&state.library, id).map_err(describe_store_error)?;
  kumbarium_store::forget(&mut state.library, id)
    .map_err(describe_store_error)?;
  audit(
    state,
    kumbarium_audit::EventKind::Forget,
    &entry.namespace,
    json!({ "id": id }),
  )?;
  Ok(vec![format!("Forgot {id} (permanently deleted).")])
}

fn render_hit(
  conn: &kumbarium_store::Connection,
  rank: usize,
  hit: &kumbarium_store::Hit,
) -> String {
  let e = &hit.entry;
  let edges = kumbarium_store::links_of(conn, &e.id).unwrap_or_default();
  let links = edges
    .iter()
    .map(|l| {
      if l.from_id == e.id {
        format!("{} -> {}", l.rel.as_str(), l.to_id)
      } else {
        format!("{} <- {}", l.rel.as_str(), l.from_id)
      }
    })
    .collect::<Vec<_>>()
    .join("; ");
  let links = if links.is_empty() {
    String::new()
  } else {
    format!("\nlinks: {links}")
  };
  // Provisional bm25 -> 0..=1 mapping until the librarian owns
  // full ranking: monotonic in match strength, never out of
  // range thanks to the clamp.
  let strength = hit.bm25.abs();
  let scores = kumbarium_librarian::Scores {
    relevance: strength / (strength + 5.0),
    confidence: e.confidence,
    confidence_basis: confidence_basis(e),
  }
  .clamped();
  let tags = if e.tags.is_empty() {
    String::new()
  } else {
    format!("\ntags: {}", e.tags.join(", "))
  };
  format!(
    "[{rank}] id={} namespace={} kind={}\n\
     relevance={:.2} confidence={:.2} ({})\n{}{}{}",
    e.id,
    e.namespace,
    e.kind.as_str(),
    scores.relevance,
    scores.confidence,
    scores.confidence_basis,
    e.content,
    tags,
    links
  )
}

fn confidence_basis(e: &kumbarium_store::Entry) -> String {
  let day = |s: &str| s.get(..10).unwrap_or(s).to_string();
  match &e.last_confirmed_at {
    Some(at) => format!("confirmed {}", day(at)),
    None => {
      format!("never confirmed; created {}", day(&e.created_at))
    }
  }
}

fn new_entry_args(args: &Value) -> Result<kumbarium_store::NewEntry, String> {
  let namespace = required_str(args, "namespace")?;
  kumbarium_librarian::validate_namespace(namespace)
    .map_err(|e| format!("invalid namespace: {e}"))?;
  let kind_raw = required_str(args, "kind")?;
  let kind = kumbarium_store::Kind::parse(kind_raw).ok_or_else(|| {
    format!(
      "unknown kind {kind_raw:?}; one of preference, \
         project_state, decision, reference"
    )
  })?;
  let content = required_str(args, "content")?;
  let tags = args
    .get("tags")
    .and_then(Value::as_array)
    .map(|a| {
      a.iter()
        .filter_map(Value::as_str)
        .map(str::to_string)
        .collect()
    })
    .unwrap_or_default();
  let source = args
    .get("source")
    .and_then(Value::as_str)
    .unwrap_or("")
    .to_string();
  Ok(kumbarium_store::NewEntry {
    namespace: namespace.to_string(),
    kind,
    content: content.to_string(),
    // The dispatching tool overwrites this with the declared
    // client identity before any store write.
    agent_id: String::new(),
    source,
    tags,
  })
}

fn required_str<'a>(args: &'a Value, key: &str) -> Result<&'a str, String> {
  args
    .get(key)
    .and_then(Value::as_str)
    .filter(|s| !s.trim().is_empty())
    .ok_or_else(|| format!("missing required argument {key:?}"))
}

fn describe_store_error(e: kumbarium_store::StoreError) -> String {
  match &e {
    kumbarium_store::StoreError::NamespaceNotRegistered(ns) => {
      format!(
        "namespace {ns:?} is not registered; ask the user to \
         run: kumbarium namespace add {ns}"
      )
    }
    _ => e.to_string(),
  }
}

fn audit(
  state: &ServerState,
  kind: kumbarium_audit::EventKind,
  scope: &str,
  detail: Value,
) -> Result<(), String> {
  let event = kumbarium_audit::Event {
    agent_id: state.agent_id.clone(),
    kind,
    scope: scope.to_string(),
    detail,
  };
  kumbarium_audit::append(&state.audit, &event)
    .map(|_| ())
    .map_err(|e| format!("operation applied but audit append failed: {e}"))
}
