//! The MCP tools wired to the shelves + librarian + audit. Every
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
  /// Minted by the librarian, one per serve process (D-044):
  /// sessions are minted, agents are claimed. Two sessions of
  /// the same agent name are different holders in the reading
  /// room.
  pub session_id: String,
  pub cfg: Config,
  /// The docket shelf, opened lazily: the file does not exist
  /// until the section is first used (D-033).
  pub docket: Option<kumbarium_docket::Connection>,
  /// Empty path = in-memory (tests, dry runs).
  pub docket_path: std::path::PathBuf,
  /// The handoff shelf, same lazy discipline.
  pub handoff: Option<kumbarium_handoff::Connection>,
  pub handoff_path: std::path::PathBuf,
  /// The restricted stacks, same lazy discipline. NOTHING on
  /// this shelf is ever served (the standing exception to
  /// D-037); access is pull-only through secret_read.
  pub secrets: Option<kumbarium_secrets::Connection>,
  pub secrets_path: std::path::PathBuf,
  /// The reading room (D-043): lazy like every shelf.
  pub leases: Option<kumbarium_leases::Connection>,
  pub leases_path: std::path::PathBuf,
  /// Scopes whose standing briefing this session has already
  /// been served (the FIRST recall in a scope prepends it,
  /// D-036; later recalls stay clean).
  pub served_handoffs: std::collections::HashSet<String>,
}

impl ServerState {
  /// The docket connection, opening the shelf on first use.
  pub fn docket(&mut self) -> Result<&kumbarium_docket::Connection, String> {
    if self.docket.is_none() {
      let conn = if self.docket_path.as_os_str().is_empty() {
        kumbarium_docket::open_in_memory()
      } else {
        kumbarium_docket::open(&self.docket_path)
      }
      .map_err(|e| e.to_string())?;
      self.docket = Some(conn);
    }
    Ok(self.docket.as_ref().expect("just opened"))
  }

  /// The secrets connection, opening the shelf on first use.
  pub fn secrets(&mut self) -> Result<&kumbarium_secrets::Connection, String> {
    if self.secrets.is_none() {
      let conn = if self.secrets_path.as_os_str().is_empty() {
        kumbarium_secrets::open_in_memory()
      } else {
        kumbarium_secrets::open(&self.secrets_path)
      }
      .map_err(|e| e.to_string())?;
      self.secrets = Some(conn);
    }
    Ok(self.secrets.as_ref().expect("just opened"))
  }

  /// The reading room's connection, opening the shelf on first
  /// use.
  pub fn leases(&mut self) -> Result<&kumbarium_leases::Connection, String> {
    if self.leases.is_none() {
      let conn = if self.leases_path.as_os_str().is_empty() {
        kumbarium_leases::open_in_memory()
      } else {
        kumbarium_leases::open(&self.leases_path)
      }
      .map_err(|e| e.to_string())?;
      self.leases = Some(conn);
    }
    Ok(self.leases.as_ref().expect("just opened"))
  }

  /// The handoff connection, opening the shelf on first use.
  pub fn handoff(&mut self) -> Result<&kumbarium_handoff::Connection, String> {
    if self.handoff.is_none() {
      let conn = if self.handoff_path.as_os_str().is_empty() {
        kumbarium_handoff::open_in_memory()
      } else {
        kumbarium_handoff::open(&self.handoff_path)
      }
      .map_err(|e| e.to_string())?;
      self.handoff = Some(conn);
    }
    Ok(self.handoff.as_ref().expect("just opened"))
  }

  #[cfg(test)]
  pub fn in_memory() -> ServerState {
    ServerState {
      library: kumbarium_store::open_in_memory().unwrap(),
      audit: kumbarium_audit::open_in_memory().unwrap(),
      agent_id: "unknown-agent".into(),
      session_id: kumbarium_util::generate_id(),
      cfg: Config::default(),
      docket: None,
      docket_path: std::path::PathBuf::new(),
      handoff: None,
      handoff_path: std::path::PathBuf::new(),
      served_handoffs: std::collections::HashSet::new(),
      secrets: None,
      secrets_path: std::path::PathBuf::new(),
      leases: None,
      leases_path: std::path::PathBuf::new(),
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
      "name": "task_file",
      "description": "File a task on the docket: a matter to be \
  done, on a registered namespace shelf. Severity is YOUR \
  judgment, and it has consequences: low = someday, normal = \
  the default, high = soon, urgent = INTERRUPTS the next \
  session's start (reserve it for production risk and \
  deadline-critical work). An optional goal (YYYY-MM-DD) is a \
  target date the library watches; an overdue goal also \
  interrupts. One self-contained statement per task; detail \
  belongs in memory. Tasks are claims of work owed, carrying \
  their filer's authority and nothing more.",
      "inputSchema": {
        "type": "object",
        "properties": {
          "namespace": { "type": "string" },
          "content": {
            "type": "string",
            "description": "The matter, one self-contained \
  statement."
          },
          "severity": {
            "type": "string",
            "enum": ["low", "normal", "high", "urgent"]
          },
          "goal": {
            "type": "string",
            "description": "Optional target date, YYYY-MM-DD."
          },
          "source": { "type": "string" }
        },
        "required": ["namespace", "content"]
      }
    },
    {
      "name": "task_update",
      "description": "Update a docket task. Pass state 'done' \
  when the work is complete (a claim, witnessed) or 'dropped' \
  when overtaken by events; or regrade with a new severity, \
  goal, or content (the old version chains forward, never \
  deleted). The id may be any unique fragment.",
      "inputSchema": {
        "type": "object",
        "properties": {
          "id": { "type": "string" },
          "state": {
            "type": "string",
            "enum": ["done", "dropped"]
          },
          "severity": {
            "type": "string",
            "enum": ["low", "normal", "high", "urgent"]
          },
          "goal": {
            "type": "string",
            "description": "New target date YYYY-MM-DD, or the \
  empty string to clear the goal."
          },
          "content": { "type": "string" },
          "note": {
            "type": "string",
            "description": "One line on why (the regrade or the \
  drop)."
          }
        },
        "required": ["id"]
      }
    },
    {
      "name": "handoff_write",
      "description": "Leave the standing briefing for a \
  namespace: what is mid-flight, decided-but-unfinished, and \
  sharp-edged, for the NEXT session in this scope. Writing \
  replaces the previous briefing (its history is kept). Do this \
  before ending substantive work. The next session receives it \
  automatically with its first recall.",
      "inputSchema": {
        "type": "object",
        "properties": {
          "namespace": { "type": "string" },
          "content": {
            "type": "string",
            "description": "The briefing, prose with judgment; \
  multi-line welcome."
          }
        },
        "required": ["namespace", "content"]
      }
    },
    {
      "name": "lease_take",
      "description": "Reserve your working area in the reading \
  room: namespace + a short resource label (a crate, a file \
  area, a subsystem). NEVER blocks: if another agent holds an \
  overlapping lease you are told, loudly, and both stand; \
  coordinate instead of colliding. The lease renews itself on \
  any activity of yours and lapses quietly when you go idle, so \
  releasing is a courtesy, not a duty. Take one when starting \
  substantive work on a distinct area.",
      "inputSchema": {
        "type": "object",
        "properties": {
          "namespace": { "type": "string" },
          "resource": {
            "type": "string",
            "description": "What you are working on, one short \
  label, e.g. 'crates/kumbarium-store' or 'docs'."
          },
          "note": {
            "type": "string",
            "description": "Optional one-liner on what you are \
  doing there."
          }
        },
        "required": ["namespace", "resource"]
      }
    },
    {
      "name": "lease_release",
      "description": "Release your own reading-room lease when \
  you finish with an area (a courtesy; idle leases lapse on \
  their own). You can only release your own.",
      "inputSchema": {
        "type": "object",
        "properties": {
          "namespace": { "type": "string" },
          "resource": { "type": "string" }
        },
        "required": ["namespace", "resource"]
      }
    },
    {
      "name": "secret_read",
      "description": "Read a credential from the restricted \
  stacks, by namespace and name. Works only if the human has \
  granted YOUR identity access to that secret; a refusal names \
  the grant command so you can ask them. Every call is \
  witnessed either way. NEVER write credential VALUES into \
  memories, tasks, or briefings; this shelf exists so you do \
  not have to.",
      "inputSchema": {
        "type": "object",
        "properties": {
          "namespace": { "type": "string" },
          "name": { "type": "string" }
        },
        "required": ["namespace", "name"]
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
    "task_file" => task_file(state, args),
    "task_update" => task_update(state, args),
    "handoff_write" => handoff_write(state, args),
    "secret_read" => secret_read(state, args),
    "lease_take" => lease_take(state, args),
    "lease_release" => lease_release(state, args),
    other => Err(format!("unknown tool {other:?}")),
  };
  // Any witnessed activity renews the caller's reading-room
  // cards: the ledger is the heartbeat (D-043). The shelf is
  // never created just to renew.
  if result.is_ok() && (state.leases.is_some() || state.leases_path.exists()) {
    let now = kumbarium_util::now_ms();
    let ttl = state.cfg.leases_ttl_minutes;
    let agent = state.agent_id.clone();
    let session = state.session_id.clone();
    if let Ok(conn) = state.leases() {
      let _ = kumbarium_leases::renew_for_session(
        conn,
        kumbarium_leases::Holder {
          agent_id: &agent,
          session_id: &session,
        },
        now,
        ttl,
      );
    }
  }
  match result {
    Ok(blocks) => (blocks, false),
    Err(msg) => (vec![msg], true),
  }
}

/// What this identity's writes become (D-027): pending when the
/// agent is quarantined by name or by default_mode, live
/// otherwise. Policy, not self-assessment: the writer never
/// chooses.
fn write_status(state: &ServerState) -> kumbarium_store::Status {
  let quarantined = state.cfg.approvals_default_pending
    || state
      .cfg
      .approvals_pending_agents
      .iter()
      .any(|a| a == &state.agent_id);
  if quarantined {
    kumbarium_store::Status::Pending
  } else {
    kumbarium_store::Status::Live
  }
}

fn remember(
  state: &mut ServerState,
  args: &Value,
) -> Result<Vec<String>, String> {
  let mut new = new_entry_args(args)?;
  new.agent_id = state.agent_id.clone();
  new.status = write_status(state);
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
  // A quarantined write must say so: the agent cannot recall it
  // until a human approves, and silence here would read as loss.
  let links = if new.status == kumbarium_store::Status::Pending {
    format!("{links} status=pending (awaiting human approval)")
  } else {
    links
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
  let scope =
    kumbarium_librarian::normalize_namespace(required_str(args, "scope")?);
  let scope = scope.as_str();
  let limit = args
    .get("limit")
    .and_then(Value::as_u64)
    .map(|v| v as usize)
    .unwrap_or(state.cfg.recall_default_limit);
  let chain = kumbarium_librarian::namespace_chain(scope)
    .map_err(|e| format!("invalid scope: {e}"))?;
  let hits = kumbarium_store::recall(&state.library, query, &chain, limit)
    .map_err(describe_store_error)?;
  // Served first, literally (D-036, D-037): the FIRST recall
  // this session makes in a scope carries the opening frame:
  // the standing briefing, then the matters that MUST interrupt
  // (urgent severity, or any open matter whose goal has passed:
  // the creep machinery surfacing to agents too). Later recalls
  // stay clean. Pending rows are never served.
  let mut briefing = None;
  let mut matters = None;
  let mut room = None;
  let mut matters_served = 0usize;
  let mut leases_served = 0usize;
  if !state.served_handoffs.contains(scope) {
    let handoff_reachable = state.handoff.is_some()
      || state.handoff_path.as_os_str().is_empty()
      || state.handoff_path.exists();
    if handoff_reachable {
      let conn = state.handoff()?;
      if let Ok(Some(h)) = kumbarium_handoff::standing(conn, scope) {
        briefing = Some(format!(
          "STANDING HANDOFF for {} (left by {} at {}):\n{}\n---",
          h.namespace, h.agent_id, h.created_at, h.content
        ));
      }
    }
    let docket_reachable = state.docket.is_some()
      || state.docket_path.as_os_str().is_empty()
      || state.docket_path.exists();
    if docket_reachable {
      let conn = state.docket()?;
      if let Ok(open) = kumbarium_docket::tasks_in(conn, Some(&chain), false) {
        let now = kumbarium_util::now_ms();
        let overdue = |t: &kumbarium_docket::Task| {
          t.goal
            .as_deref()
            .and_then(|g| {
              kumbarium_util::parse_iso8601_ms(&format!("{g}T00:00:00.000Z"))
            })
            .map(|ms| ms < now)
            .unwrap_or(false)
        };
        let must: Vec<_> = open
          .iter()
          .filter(|t| {
            t.severity == kumbarium_docket::Severity::Urgent || overdue(t)
          })
          .take(5)
          .collect();
        if !must.is_empty() {
          matters_served = must.len();
          let mut block =
            format!("OPEN MATTERS for {scope} demanding attention:");
          for t in &must {
            let goal = t
              .goal
              .as_deref()
              .map(|g| format!(" (goal {g})"))
              .unwrap_or_default();
            block.push_str(&format!(
              "\n- [{}] id={} {}{goal}",
              t.severity.as_str(),
              &t.id[t.id.len().saturating_sub(8)..],
              t.content.lines().next().unwrap_or("")
            ));
          }
          if open.len() > must.len() {
            block.push_str(&format!(
              "\n(+{} more open matters on the docket)",
              open.len() - must.len()
            ));
          }
          block.push_str("\n---");
          matters = Some(block);
        }
      }
    }
    // The reading room rides too (D-043): who is at work in
    // this chain, so occupancy is learned without a tool to
    // forget. The agent's own cards are not news to it.
    let leases_reachable = state.leases.is_some()
      || state.leases_path.as_os_str().is_empty()
      || state.leases_path.exists();
    if leases_reachable {
      let now = kumbarium_util::now_ms();
      let ttl = state.cfg.leases_ttl_minutes;
      let me = state.agent_id.clone();
      let conn = state.leases()?;
      let mut cards = Vec::new();
      for ns in &chain {
        if let Ok(mut v) = kumbarium_leases::active_in(conn, Some(ns), now, ttl)
        {
          cards.append(&mut v);
        }
      }
      let my_session = state.session_id.clone();
      cards.retain(|l| !(l.agent_id == me && l.session_id == my_session));
      if !cards.is_empty() {
        leases_served = cards.len();
        let mut block = format!(
          "THE READING ROOM for {scope} (agents at work; \
           coordinate, avoid collisions):"
        );
        for l in &cards {
          let same = if l.agent_id == me {
            " [another session of you]"
          } else {
            ""
          };
          block.push_str(&format!(
            "\n- {} (session {}){same} holds {}/{} (since {})",
            l.agent_id,
            kumbarium_leases::short_id(&l.session_id),
            l.namespace,
            l.resource,
            l.taken_at
          ));
        }
        block.push_str("\n---");
        room = Some(block);
      }
    }
    state.served_handoffs.insert(scope.to_string());
  }
  let ids: Vec<&str> = hits.iter().map(|h| h.entry.id.as_str()).collect();
  audit(
    state,
    kumbarium_audit::EventKind::Recall,
    scope,
    json!({
      "query": query,
      "returned": ids,
      "handoff_served": briefing.is_some(),
      "matters_served": matters_served,
      "leases_served": leases_served,
    }),
  )?;
  let mut blocks = Vec::new();
  if let Some(b) = briefing {
    blocks.push(b);
  }
  if let Some(m) = matters {
    blocks.push(m);
  }
  if let Some(r) = room {
    blocks.push(r);
  }
  if hits.is_empty() {
    blocks.push(format!("No memories matched {query:?} in scope {scope}."));
    return Ok(blocks);
  }
  blocks.push(format!("{} memor(y/ies) found:", hits.len()));
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
  // Policy status; the store still forces pending when the
  // superseded entry is itself pending (D-027).
  new.status = write_status(state);
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
  // The janitor's stored basis explains the number it set;
  // before any pass, fall back to the confirm/created stamps.
  if let Some(basis) = &e.confidence_basis {
    return basis.clone();
  }
  let day = |s: &str| s.get(..10).unwrap_or(s).to_string();
  match &e.last_confirmed_at {
    Some(at) => format!("confirmed {}", day(at)),
    None => {
      format!("never confirmed; created {}", day(&e.created_at))
    }
  }
}

/// The master key for a keystore-sealed shelf, or None for a
/// plaintext-mode shelf; Blocked and Absent refuse per D-039.
pub(crate) fn secrets_key(
  state: &mut ServerState,
) -> Result<Option<[u8; kumbarium_secrets::KEY_LEN]>, String> {
  let mode = kumbarium_secrets::sealing_mode(state.secrets()?)
    .map_err(|e| e.to_string())?;
  match mode {
    Some(kumbarium_secrets::Sealing::Plaintext) => Ok(None),
    Some(kumbarium_secrets::Sealing::Keystore) | None => {
      match super::keystore::master_key() {
        super::keystore::Keystore::Present(key) => Ok(Some(key)),
        super::keystore::Keystore::Absent => Err(
          "no platform keystore substrate; this shelf cannot \
           unseal here"
            .into(),
        ),
        super::keystore::Keystore::Blocked(why) => Err(format!(
          "keystore blocked ({why}); refusing rather than \
           downgrading"
        )),
      }
    }
  }
}

fn lease_take(
  state: &mut ServerState,
  args: &Value,
) -> Result<Vec<String>, String> {
  let namespace =
    kumbarium_librarian::normalize_namespace(required_str(args, "namespace")?);
  kumbarium_librarian::validate_namespace(&namespace)
    .map_err(|e| format!("invalid namespace: {e}"))?;
  if kumbarium_store::namespace_id(&state.library, &namespace)
    .map_err(describe_store_error)?
    .is_none()
  {
    return Err(format!(
      "namespace {namespace:?} is not registered (the user \
       registers namespaces; ask them if yours is missing)"
    ));
  }
  let resource = required_str(args, "resource")?.trim().to_string();
  if resource.is_empty() || resource.len() > 200 || resource.contains('\n') {
    return Err("resource must be one short label".into());
  }
  let note = args.get("note").and_then(Value::as_str);
  let now = kumbarium_util::now_ms();
  let ttl = state.cfg.leases_ttl_minutes;
  let agent = state.agent_id.clone();
  let session = state.session_id.clone();
  let (card, others) = {
    let conn = state.leases()?;
    kumbarium_leases::take(
      conn,
      &namespace,
      &resource,
      kumbarium_leases::Holder {
        agent_id: &agent,
        session_id: &session,
      },
      note,
      now,
      ttl,
    )
    .map_err(|e| e.to_string())?
  };
  audit(
    state,
    kumbarium_audit::EventKind::LeaseTake,
    &namespace,
    json!({
      "id": card.id,
      "resource": resource,
      "overlapping": others.len(),
    }),
  )?;
  let mut blocks = vec![format!(
    "Lease taken on {namespace}/{resource} (id {}, lapses \
     after {ttl} idle minutes; your activity renews it).",
    kumbarium_leases::short_id(&card.id)
  )];
  if !others.is_empty() {
    let mut warn =
      String::from("WARNING: you are not alone at this table. Also held by:");
    for l in &others {
      let same = if l.agent_id == agent {
        " [ANOTHER SESSION OF YOU]"
      } else {
        ""
      };
      warn.push_str(&format!(
        "\n- {} (session {}){same} since {}{}",
        l.agent_id,
        kumbarium_leases::short_id(&l.session_id),
        l.taken_at,
        l.note
          .as_deref()
          .map(|n| format!(": {n}"))
          .unwrap_or_default()
      ));
    }
    warn.push_str(
      "\nCoordinate before touching the same files; the lease \
       warns, it does not protect.",
    );
    blocks.push(warn);
  }
  Ok(blocks)
}

fn lease_release(
  state: &mut ServerState,
  args: &Value,
) -> Result<Vec<String>, String> {
  let namespace =
    kumbarium_librarian::normalize_namespace(required_str(args, "namespace")?);
  kumbarium_librarian::validate_namespace(&namespace)
    .map_err(|e| format!("invalid namespace: {e}"))?;
  let resource = required_str(args, "resource")?.trim().to_string();
  let now = kumbarium_util::now_ms();
  let ttl = state.cfg.leases_ttl_minutes;
  let agent = state.agent_id.clone();
  let session = state.session_id.clone();
  let card = {
    let conn = state.leases()?;
    kumbarium_leases::release(
      conn,
      &namespace,
      &resource,
      kumbarium_leases::Holder {
        agent_id: &agent,
        session_id: &session,
      },
      now,
      ttl,
    )
    .map_err(|e| e.to_string())?
  };
  audit(
    state,
    kumbarium_audit::EventKind::LeaseRelease,
    &namespace,
    json!({ "id": card.id, "resource": resource }),
  )?;
  Ok(vec![format!(
    "Released {namespace}/{resource}. The table is yours no \
     longer."
  )])
}

fn secret_read(
  state: &mut ServerState,
  args: &Value,
) -> Result<Vec<String>, String> {
  let namespace =
    kumbarium_librarian::normalize_namespace(required_str(args, "namespace")?);
  kumbarium_librarian::validate_namespace(&namespace)
    .map_err(|e| format!("invalid namespace: {e}"))?;
  let name = required_str(args, "name")?.to_string();
  let agent = state.agent_id.clone();
  let granted =
    kumbarium_secrets::check_grant(state.secrets()?, &namespace, &name, &agent)
      .map_err(|e| e.to_string())?;
  // Witness BEFORE the value moves: if the append fails, the
  // value is withheld (fail-closed, D-038). Refusals are
  // events too.
  audit(
    state,
    kumbarium_audit::EventKind::SecretRead,
    &namespace,
    json!({ "name": name, "granted": granted }),
  )?;
  if !granted {
    return Err(format!(
      "no grant: your identity ({agent}) is not granted \
       {namespace}/{name}. Ask the human to run: kumbarium \
       secret grant {namespace} {name} {agent}"
    ));
  }
  let key = secrets_key(state)?;
  let value = kumbarium_secrets::read_secret(
    state.secrets()?,
    &namespace,
    &name,
    key.as_ref(),
  )
  .map_err(|e| e.to_string())?;
  let text = String::from_utf8_lossy(&value).to_string();
  Ok(vec![format!(
    "Secret {namespace}/{name} (do not store this value \
     anywhere in the library):\n{text}"
  )])
}

fn handoff_write(
  state: &mut ServerState,
  args: &Value,
) -> Result<Vec<String>, String> {
  let namespace =
    kumbarium_librarian::normalize_namespace(required_str(args, "namespace")?);
  kumbarium_librarian::validate_namespace(&namespace)
    .map_err(|e| format!("invalid namespace: {e}"))?;
  if kumbarium_store::namespace_id(&state.library, &namespace)
    .map_err(describe_store_error)?
    .is_none()
  {
    return Err(format!(
      "namespace {namespace:?} is not registered (the user \
       registers namespaces; ask them if yours is missing)"
    ));
  }
  let content = required_str(args, "content")?.to_string();
  let status = match task_write_status(state) {
    kumbarium_docket::Status::Pending => kumbarium_handoff::Status::Pending,
    _ => kumbarium_handoff::Status::Live,
  };
  let agent = state.agent_id.clone();
  let h = kumbarium_handoff::write_handoff(
    state.handoff()?,
    &namespace,
    &content,
    &agent,
    "",
    status,
  )
  .map_err(|e| e.to_string())?;
  audit(
    state,
    kumbarium_audit::EventKind::HandoffWrite,
    &namespace,
    json!({ "id": h.id }),
  )?;
  let mut line = format!(
    "Briefing left for {namespace} (id={}). The next session's \
     first recall in this scope will receive it.",
    h.id
  );
  if h.status == kumbarium_handoff::Status::Pending {
    line = format!(
      "Briefing recorded pending (id={}): it awaits human \
       approval and will NOT be served until approved.",
      h.id
    );
  }
  Ok(vec![line])
}

fn validate_goal(goal: &str) -> Result<(), String> {
  let ok = goal.len() == 10
    && kumbarium_util::parse_iso8601_ms(&format!("{goal}T00:00:00.000Z"))
      .is_some();
  if ok {
    Ok(())
  } else {
    Err(format!("invalid goal {goal:?}; use YYYY-MM-DD"))
  }
}

fn task_write_status(state: &ServerState) -> kumbarium_docket::Status {
  let quarantined = state.cfg.approvals_default_pending
    || state
      .cfg
      .approvals_pending_agents
      .iter()
      .any(|a| a == &state.agent_id);
  if quarantined {
    kumbarium_docket::Status::Pending
  } else {
    kumbarium_docket::Status::Live
  }
}

fn task_file(
  state: &mut ServerState,
  args: &Value,
) -> Result<Vec<String>, String> {
  let namespace =
    kumbarium_librarian::normalize_namespace(required_str(args, "namespace")?);
  kumbarium_librarian::validate_namespace(&namespace)
    .map_err(|e| format!("invalid namespace: {e}"))?;
  if kumbarium_store::namespace_id(&state.library, &namespace)
    .map_err(describe_store_error)?
    .is_none()
  {
    return Err(format!(
      "namespace {namespace:?} is not registered (the user \
       registers namespaces; ask them if yours is missing)"
    ));
  }
  let content = required_str(args, "content")?.to_string();
  let severity = match args.get("severity").and_then(Value::as_str) {
    Some(raw) => kumbarium_docket::Severity::parse(raw)
      .ok_or_else(|| format!("unknown severity {raw:?}"))?,
    None => kumbarium_docket::Severity::Normal,
  };
  let goal = match args.get("goal").and_then(Value::as_str) {
    Some(g) if !g.is_empty() => {
      validate_goal(g)?;
      Some(g.to_string())
    }
    _ => None,
  };
  let new = kumbarium_docket::NewTask {
    namespace: namespace.clone(),
    content,
    agent_id: state.agent_id.clone(),
    source: args
      .get("source")
      .and_then(Value::as_str)
      .unwrap_or_default()
      .to_string(),
    severity,
    goal: goal.clone(),
    status: task_write_status(state),
  };
  let task = kumbarium_docket::file_task(state.docket()?, &new)
    .map_err(|e| e.to_string())?;
  audit(
    state,
    kumbarium_audit::EventKind::TaskFile,
    &namespace,
    json!({
      "id": task.id,
      "severity": task.severity.as_str(),
      "goal": task.goal,
    }),
  )?;
  let mut line = format!(
    "Filed. id={} severity={} namespace={}",
    task.id,
    task.severity.as_str(),
    task.namespace
  );
  if let Some(goal) = &task.goal {
    line.push_str(&format!(" goal={goal}"));
  }
  if task.status == kumbarium_docket::Status::Pending {
    line.push_str(" status=pending (awaiting human approval)");
  }
  // Echo the scope's open docket so filing shows its neighbors.
  let chain = kumbarium_librarian::namespace_chain(&namespace)
    .map_err(|e| format!("invalid scope: {e}"))?;
  let open = kumbarium_docket::tasks_in(state.docket()?, Some(&chain), false)
    .map_err(|e| e.to_string())?;
  line.push_str(&format!("\nOpen matters in scope: {}", open.len()));
  for t in open.iter().take(8) {
    line.push_str(&format!(
      "\n- [{}] id={} {}",
      t.severity.as_str(),
      &t.id[t.id.len().saturating_sub(8)..],
      t.content.lines().next().unwrap_or("")
    ));
  }
  Ok(vec![line])
}

fn task_update(
  state: &mut ServerState,
  args: &Value,
) -> Result<Vec<String>, String> {
  let frag = required_str(args, "id")?;
  let id = kumbarium_docket::resolve_id(state.docket()?, frag)
    .map_err(|e| e.to_string())?;
  let note = args
    .get("note")
    .and_then(Value::as_str)
    .and_then(kumbarium_librarian::sanitize_note);
  if let Some(target) = args.get("state").and_then(Value::as_str) {
    let to = match target {
      "done" => kumbarium_docket::TaskState::Done,
      "dropped" => kumbarium_docket::TaskState::Dropped,
      other => return Err(format!("unknown state {other:?}")),
    };
    let task =
      kumbarium_docket::get(state.docket()?, &id).map_err(|e| e.to_string())?;
    kumbarium_docket::set_state(state.docket()?, &id, to, note.as_deref())
      .map_err(|e| e.to_string())?;
    let kind = if to == kumbarium_docket::TaskState::Done {
      kumbarium_audit::EventKind::TaskDone
    } else {
      kumbarium_audit::EventKind::TaskDrop
    };
    audit(
      state,
      kind,
      &task.namespace,
      json!({ "id": id, "note": note }),
    )?;
    return Ok(vec![format!(
      "Recorded: task {} {}.",
      &id[id.len().saturating_sub(8)..],
      to.as_str()
    )]);
  }
  let mut edit = kumbarium_docket::TaskEdit {
    note: note.clone(),
    ..Default::default()
  };
  if let Some(raw) = args.get("severity").and_then(Value::as_str) {
    edit.severity = Some(
      kumbarium_docket::Severity::parse(raw)
        .ok_or_else(|| format!("unknown severity {raw:?}"))?,
    );
  }
  if let Some(goal) = args.get("goal").and_then(Value::as_str) {
    if goal.is_empty() {
      edit.goal = Some(None);
    } else {
      validate_goal(goal)?;
      edit.goal = Some(Some(goal.to_string()));
    }
  }
  if let Some(content) = args.get("content").and_then(Value::as_str) {
    edit.content = Some(content.to_string());
  }
  if edit.severity.is_none() && edit.goal.is_none() && edit.content.is_none() {
    return Err(
      "nothing to update: pass state, severity, goal, or content".into(),
    );
  }
  let agent = state.agent_id.clone();
  let task =
    kumbarium_docket::supersede_task(state.docket()?, &id, &edit, &agent)
      .map_err(|e| e.to_string())?;
  audit(
    state,
    kumbarium_audit::EventKind::TaskUpdate,
    &task.namespace,
    json!({
      "old_id": id,
      "new_id": task.id,
      "severity": task.severity.as_str(),
      "goal": task.goal,
      "note": note,
    }),
  )?;
  Ok(vec![format!(
    "Regraded. id={} severity={} goal={}",
    task.id,
    task.severity.as_str(),
    task.goal.as_deref().unwrap_or("none")
  )])
}

fn new_entry_args(args: &Value) -> Result<kumbarium_store::NewEntry, String> {
  let namespace =
    kumbarium_librarian::normalize_namespace(required_str(args, "namespace")?);
  kumbarium_librarian::validate_namespace(&namespace)
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
    // The dispatching tool overwrites agent_id with the declared
    // client identity, and status with the write policy for that
    // identity (D-027), before any store write.
    agent_id: String::new(),
    status: kumbarium_store::Status::Live,
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
    session_id: state.session_id.clone(),
    kind,
    scope: scope.to_string(),
    detail,
  };
  kumbarium_audit::append(&state.audit, &event)
    .map(|_| ())
    .map_err(|e| format!("operation applied but audit append failed: {e}"))
}
