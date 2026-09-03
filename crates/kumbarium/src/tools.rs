//! The four MCP tools wired to store + librarian + audit. Every
//! call is audited after the operation; an audit failure fails
//! the call (audit completeness over availability, D-007's
//! trade, even in the synchronous writer).

use serde_json::{Value, json};

const RECALL_DEFAULT_LIMIT: usize = 8;

/// Everything one server process holds: the two database
/// connections and the identity the client declared at
/// initialize time.
pub struct ServerState {
  pub library: kumbarium_store::Connection,
  pub audit: kumbarium_store::Connection,
  pub agent_id: String,
}

impl ServerState {
  #[cfg(test)]
  pub fn in_memory() -> ServerState {
    ServerState {
      library: kumbarium_store::open_in_memory().unwrap(),
      audit: kumbarium_audit::open_in_memory().unwrap(),
      agent_id: "unknown-agent".into(),
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
  namespaces; ask them if yours is missing).",
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
          }
        },
        "required": ["namespace", "kind", "content"]
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
      "name": "supersede",
      "description": "Replace an outdated memory with a \
  corrected one. The old entry is chained forward, never \
  deleted. Use when a recalled fact turns out stale or wrong.",
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
          "source": { "type": "string" }
        },
        "required": ["old_id", "namespace", "kind", "content"]
      }
    },
    {
      "name": "forget",
      "description": "Permanently delete a memory. Escape \
  hatch for wrong or sensitive content only; for routine \
  correction use supersede, which preserves history.",
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
  let entry = kumbarium_store::remember(&mut state.library, &new)
    .map_err(describe_store_error)?;
  audit(
    state,
    kumbarium_audit::EventKind::Remember,
    &new.namespace,
    json!({ "id": entry.id, "kind": new.kind.as_str() }),
  )?;
  Ok(vec![format!(
    "Remembered. id={} namespace={} kind={}",
    entry.id,
    entry.namespace,
    entry.kind.as_str()
  )])
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
    .unwrap_or(RECALL_DEFAULT_LIMIT);
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
    blocks.push(render_hit(i + 1, hit));
  }
  Ok(blocks)
}

fn supersede(
  state: &mut ServerState,
  args: &Value,
) -> Result<Vec<String>, String> {
  let old_id = required_str(args, "old_id")?;
  let mut new = new_entry_args(args)?;
  new.agent_id = state.agent_id.clone();
  let entry = kumbarium_store::supersede(&mut state.library, old_id, &new)
    .map_err(describe_store_error)?;
  audit(
    state,
    kumbarium_audit::EventKind::Supersede,
    &new.namespace,
    json!({ "old_id": old_id, "new_id": entry.id }),
  )?;
  Ok(vec![format!(
    "Superseded {old_id}. New entry id={}",
    entry.id
  )])
}

fn forget(
  state: &mut ServerState,
  args: &Value,
) -> Result<Vec<String>, String> {
  let id = required_str(args, "id")?;
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

fn render_hit(rank: usize, hit: &kumbarium_store::Hit) -> String {
  let e = &hit.entry;
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
     relevance={:.2} confidence={:.2} ({})\n{}{}",
    e.id,
    e.namespace,
    e.kind.as_str(),
    scores.relevance,
    scores.confidence,
    scores.confidence_basis,
    e.content,
    tags
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
