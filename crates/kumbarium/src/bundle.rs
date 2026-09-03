//! Bundles (D-028): a shelf as one deterministic, hashed JSON
//! file, and the union-merge that imports one. Entries travel
//! with full provenance and chain pointers; confidence does NOT
//! travel (evidence is local; the receiving janitor re-earns the
//! number). Forked supersession never auto-resolves: the rival
//! head lands pending with a `contradicts` edge, and the desk
//! settles it.

use std::collections::{BTreeMap, BTreeSet};

use serde_json::{Value, json};

use super::tools::ServerState;

pub const FORMAT: u64 = 1;

/// Serialize one namespace's circulating record (status = live,
/// superseded ancestors and retired entries included; pending
/// and rejected material never travels) as pretty JSON.
pub fn export(
  state: &ServerState,
  scope: &str,
) -> Result<(String, usize), String> {
  let scope = &kumbarium_librarian::normalize_namespace(scope);
  kumbarium_librarian::validate_namespace(scope)
    .map_err(|e| format!("invalid namespace: {e}"))?;
  let known = kumbarium_store::namespace_id(&state.library, scope)
    .map_err(|e| e.to_string())?;
  if known.is_none() {
    return Err(format!("namespace {scope:?} is not registered"));
  }
  let mut entries =
    kumbarium_store::entries_in(&state.library, Some(scope), true)
      .map_err(|e| e.to_string())?;
  entries.retain(|e| e.status == kumbarium_store::Status::Live);
  entries.sort_by(|a, b| a.id.cmp(&b.id));
  let ids: BTreeSet<&str> = entries.iter().map(|e| e.id.as_str()).collect();

  let mut links: BTreeSet<(String, String, String)> = BTreeSet::new();
  for e in &entries {
    for l in kumbarium_store::links_of(&state.library, &e.id)
      .map_err(|err| err.to_string())?
    {
      if ids.contains(l.from_id.as_str()) && ids.contains(l.to_id.as_str()) {
        links.insert((l.from_id, l.to_id, l.rel.as_str().to_string()));
      }
    }
  }

  let count = entries.len();
  let body = body_value(
    scope,
    entries.iter().map(entry_value).collect(),
    links
      .iter()
      .map(|(f, t, r)| json!({ "from": f, "to": t, "rel": r }))
      .collect(),
  );
  let hash = kumbarium_util::sha256_hex(
    serde_json::to_string(&body)
      .map_err(|e| e.to_string())?
      .as_bytes(),
  );
  let mut file = BTreeMap::new();
  file.insert("kumbarium_bundle", json!(FORMAT));
  file.insert("exported_at", json!(kumbarium_util::now_iso8601()));
  file.insert("content_hash", json!(hash));
  file.insert("body", body);
  serde_json::to_string_pretty(&file)
    .map(|s| (s + "\n", count))
    .map_err(|e| e.to_string())
}

/// What one import concluded, for the caller's report and the
/// audit event.
#[derive(Debug)]
pub struct ImportSummary {
  pub scope: String,
  pub hash: String,
  pub planned: usize,
  pub imported: usize,
  pub skipped: usize,
  pub extended: usize,
  pub forks: Vec<(String, String)>,
}

/// Union-merge a bundle file's content (D-028). `as_pending`
/// forces every imported chain head into quarantine; fork rivals
/// land pending regardless.
pub fn import(
  state: &mut ServerState,
  text: &str,
  as_pending: bool,
) -> Result<ImportSummary, String> {
  let file: Value =
    serde_json::from_str(text).map_err(|e| format!("not JSON: {e}"))?;
  if file.get("kumbarium_bundle").and_then(Value::as_u64) != Some(FORMAT) {
    return Err("not a kumbarium bundle (format marker missing)".into());
  }
  let claimed = file
    .get("content_hash")
    .and_then(Value::as_str)
    .ok_or("bundle has no content_hash")?
    .to_string();
  let body = file.get("body").ok_or("bundle has no body")?;
  let actual = kumbarium_util::sha256_hex(
    serde_json::to_string(body)
      .map_err(|e| e.to_string())?
      .as_bytes(),
  );
  if actual != claimed {
    return Err(format!(
      "content hash mismatch: file claims {claimed} but the body \
       hashes to {actual}; refuse to import an altered bundle"
    ));
  }
  let scope = body
    .get("scope")
    .and_then(Value::as_str)
    .ok_or("bundle body has no scope")?
    .to_string();
  if kumbarium_store::namespace_id(&state.library, &scope)
    .map_err(|e| e.to_string())?
    .is_none()
  {
    return Err(format!(
      "namespace {scope:?} is not registered here; register it \
       first: kumbarium namespace add {scope}"
    ));
  }
  let raw_entries = body
    .get("entries")
    .and_then(Value::as_array)
    .ok_or("bundle body has no entries")?;
  let mut incoming: Vec<kumbarium_store::Entry> = Vec::new();
  for v in raw_entries {
    incoming.push(parse_entry(v)?);
  }
  let by_id: BTreeMap<&str, &kumbarium_store::Entry> =
    incoming.iter().map(|e| (e.id.as_str(), e)).collect();

  let mut summary = ImportSummary {
    scope: scope.clone(),
    hash: claimed,
    planned: incoming.len(),
    imported: 0,
    skipped: 0,
    extended: 0,
    forks: Vec::new(),
  };

  // Pass 1: insert missing entries (statuses decided below),
  // remembering which existed before for pointer reconciliation.
  let mut existed: BTreeMap<String, Option<String>> = BTreeMap::new();
  for e in &incoming {
    match kumbarium_store::get(&state.library, &e.id) {
      Ok(local) => {
        if local.content != e.content {
          return Err(
            kumbarium_store::StoreError::ContentDivergence(e.id.clone())
              .to_string(),
          );
        }
        existed.insert(e.id.clone(), local.superseded_by.clone());
        summary.skipped += 1;
      }
      Err(kumbarium_store::StoreError::EntryNotFound(_)) => {
        let mut fresh = e.clone();
        // Evidence is local: arrive at the neutral prior.
        fresh.confidence = 0.5;
        fresh.confidence_basis = None;
        if as_pending && e.superseded_by.is_none() {
          fresh.status = kumbarium_store::Status::Pending;
        }
        kumbarium_store::import_entry(&state.library, &fresh)
          .map_err(|err| err.to_string())?;
        summary.imported += 1;
      }
      Err(err) => return Err(err.to_string()),
    }
  }

  // Pass 2: pointer reconciliation for entries both sides know.
  for (id, local_next) in &existed {
    let bundle_next = &by_id[id.as_str()].superseded_by;
    match (local_next, bundle_next) {
      (None, Some(next)) => {
        // Bundle is ahead: fast-forward the local chain.
        kumbarium_store::extend_chain(&state.library, id, next)
          .map_err(|e| e.to_string())?;
        summary.extended += 1;
      }
      (Some(local), Some(theirs)) if local != theirs => {
        // Fork: both sides superseded the same entry. The rival
        // branch's head goes to the desk; the local head stays
        // live; a contradicts edge names the dispute (D-028).
        let rival_head = chain_head(&by_id, theirs);
        let local_head = local_chain_head(&state.library, local)?;
        kumbarium_store::quarantine(&state.library, &rival_head)
          .map_err(|e| e.to_string())?;
        kumbarium_store::link(
          &state.library,
          &rival_head,
          &local_head,
          kumbarium_store::Rel::Contradicts,
        )
        .map_err(|e| e.to_string())?;
        summary.forks.push((rival_head, local_head));
      }
      _ => {}
    }
  }

  // Pass 3: edges (idempotent; endpoints exist by construction).
  if let Some(links) = body.get("links").and_then(Value::as_array) {
    for l in links {
      let from = l.get("from").and_then(Value::as_str);
      let to = l.get("to").and_then(Value::as_str);
      let rel = l
        .get("rel")
        .and_then(Value::as_str)
        .and_then(kumbarium_store::Rel::parse);
      if let (Some(from), Some(to), Some(rel)) = (from, to, rel) {
        kumbarium_store::link(&state.library, from, to, rel)
          .map_err(|e| e.to_string())?;
      }
    }
  }
  Ok(summary)
}

fn chain_head(
  by_id: &BTreeMap<&str, &kumbarium_store::Entry>,
  start: &str,
) -> String {
  let mut cur = start.to_string();
  let mut hops = 0;
  while let Some(e) = by_id.get(cur.as_str()) {
    match &e.superseded_by {
      Some(next) if hops < 1000 => {
        cur = next.clone();
        hops += 1;
      }
      _ => break,
    }
  }
  cur
}

fn local_chain_head(
  conn: &kumbarium_store::Connection,
  start: &str,
) -> Result<String, String> {
  let history =
    kumbarium_store::version_history(conn, start).map_err(|e| e.to_string())?;
  Ok(history.last().cloned().unwrap_or_else(|| start.to_string()))
}

fn entry_value(e: &kumbarium_store::Entry) -> Value {
  json!({
    "id": e.id,
    "namespace": e.namespace,
    "kind": e.kind.as_str(),
    "content": e.content,
    "agent_id": e.agent_id,
    "source": e.source,
    "superseded_by": e.superseded_by,
    "note": e.note,
    "retired_at": e.retired_at,
    "created_at": e.created_at,
    "updated_at": e.updated_at,
    "tags": e.tags,
  })
}

fn parse_entry(v: &Value) -> Result<kumbarium_store::Entry, String> {
  let s = |key: &str| -> Result<String, String> {
    v.get(key)
      .and_then(Value::as_str)
      .map(str::to_string)
      .ok_or_else(|| format!("bundle entry missing {key:?}"))
  };
  let opt = |key: &str| -> Option<String> {
    v.get(key).and_then(Value::as_str).map(str::to_string)
  };
  let kind_raw = s("kind")?;
  let kind = kumbarium_store::Kind::parse(&kind_raw)
    .ok_or_else(|| format!("bundle entry has unknown kind {kind_raw:?}"))?;
  Ok(kumbarium_store::Entry {
    id: s("id")?,
    namespace: s("namespace")?,
    kind,
    content: s("content")?,
    agent_id: s("agent_id")?,
    source: opt("source").unwrap_or_default(),
    confidence: 0.5,
    confidence_basis: None,
    superseded_by: opt("superseded_by"),
    created_at: s("created_at")?,
    updated_at: s("updated_at")?,
    last_accessed_at: None,
    last_confirmed_at: None,
    retired_at: opt("retired_at"),
    note: opt("note"),
    status: kumbarium_store::Status::Live,
    tags: v
      .get("tags")
      .and_then(Value::as_array)
      .map(|a| {
        a.iter()
          .filter_map(Value::as_str)
          .map(str::to_string)
          .collect()
      })
      .unwrap_or_default(),
  })
}

fn body_value(scope: &str, entries: Vec<Value>, links: Vec<Value>) -> Value {
  json!({
    "scope": scope,
    "entries": entries,
    "links": links,
  })
}

#[cfg(test)]
mod tests {
  use super::*;

  fn seeded_state(scope: &str) -> ServerState {
    let mut state = ServerState::in_memory();
    kumbarium_store::register_namespace(&state.library, scope, "test").unwrap();
    state.agent_id = "bundle-test".into();
    let _ = &mut state;
    state
  }

  fn put(state: &mut ServerState, scope: &str, content: &str) -> String {
    kumbarium_store::remember(
      &mut state.library,
      &kumbarium_store::NewEntry {
        namespace: scope.into(),
        kind: kumbarium_store::Kind::Decision,
        content: content.into(),
        agent_id: "writer-a".into(),
        source: "test".into(),
        tags: vec!["t1".into()],
        status: kumbarium_store::Status::Live,
      },
    )
    .unwrap()
    .id
  }

  #[test]
  fn round_trip_is_id_identical_and_idempotent() {
    let mut a = seeded_state("project/bnd");
    put(&mut a, "project/bnd", "fact one about the grelvix");
    put(&mut a, "project/bnd", "fact two about the plorvane");
    let (text, count) = export(&a, "project/bnd").unwrap();
    assert_eq!(count, 2);

    let mut b = seeded_state("project/bnd");
    let s1 = import(&mut b, &text, false).unwrap();
    assert_eq!((s1.imported, s1.skipped), (2, 0));
    let s2 = import(&mut b, &text, false).unwrap();
    assert_eq!((s2.imported, s2.skipped), (0, 2), "re-import no-ops");
    let (text_b, _) = export(&b, "project/bnd").unwrap();
    // Same content hash: the shelf replicated exactly.
    let hash = |t: &str| {
      let v: Value = serde_json::from_str(t).unwrap();
      v["content_hash"].as_str().unwrap().to_string()
    };
    assert_eq!(hash(&text), hash(&text_b));
  }

  #[test]
  fn tampering_is_refused() {
    let mut a = seeded_state("project/bnd");
    put(&mut a, "project/bnd", "fact about the grelvix");
    let (text, _) = export(&a, "project/bnd").unwrap();
    let altered = text.replace("grelvix", "grelvox");
    let mut b = seeded_state("project/bnd");
    let err = import(&mut b, &altered, false).unwrap_err();
    assert!(err.contains("hash mismatch"), "{err}");
  }

  #[test]
  fn pending_flag_routes_heads_to_the_desk() {
    let mut a = seeded_state("project/bnd");
    put(&mut a, "project/bnd", "an unreviewed visiting fact");
    let (text, _) = export(&a, "project/bnd").unwrap();
    let mut b = seeded_state("project/bnd");
    import(&mut b, &text, true).unwrap();
    let inbox = kumbarium_store::pending_in(&b.library).unwrap();
    assert_eq!(inbox.len(), 1);
    let hits = kumbarium_store::recall(
      &b.library,
      "visiting fact",
      &["project/bnd".into()],
      5,
    )
    .unwrap();
    assert!(hits.is_empty(), "pending never circulates");
  }

  #[test]
  fn chain_fast_forward_extends_local() {
    let mut a = seeded_state("project/bnd");
    let old = put(&mut a, "project/bnd", "the grelvix budget is 3");
    let (behind, _) = export(&a, "project/bnd").unwrap();
    // A moves ahead after the export.
    kumbarium_store::supersede(
      &mut a.library,
      &old,
      &kumbarium_store::NewEntry {
        namespace: "project/bnd".into(),
        kind: kumbarium_store::Kind::Decision,
        content: "the grelvix budget is 4".into(),
        agent_id: "writer-a".into(),
        source: "test".into(),
        tags: vec![],
        status: kumbarium_store::Status::Live,
      },
      None,
    )
    .unwrap();
    let (ahead, _) = export(&a, "project/bnd").unwrap();
    // B has the old bundle, then receives the newer one.
    let mut b = seeded_state("project/bnd");
    import(&mut b, &behind, false).unwrap();
    let s = import(&mut b, &ahead, false).unwrap();
    assert_eq!(s.extended, 1, "chain fast-forwarded");
    let hits = kumbarium_store::recall(
      &b.library,
      "grelvix budget",
      &["project/bnd".into()],
      5,
    )
    .unwrap();
    assert_eq!(hits.len(), 1);
    assert!(hits[0].entry.content.contains("is 4"));
  }

  #[test]
  fn forked_supersession_goes_to_the_desk() {
    let mut a = seeded_state("project/bnd");
    let base = put(&mut a, "project/bnd", "the grelvix budget is 3");
    let (shared, _) = export(&a, "project/bnd").unwrap();
    // Both sides supersede the same base, differently.
    let revise = |state: &mut ServerState, content: &str| {
      kumbarium_store::supersede(
        &mut state.library,
        &base,
        &kumbarium_store::NewEntry {
          namespace: "project/bnd".into(),
          kind: kumbarium_store::Kind::Decision,
          content: content.into(),
          agent_id: "writer-a".into(),
          source: "test".into(),
          tags: vec![],
          status: kumbarium_store::Status::Live,
        },
        None,
      )
      .unwrap()
      .id
    };
    let mut b = seeded_state("project/bnd");
    import(&mut b, &shared, false).unwrap();
    let a_head = revise(&mut a, "the grelvix budget is 4");
    let b_head = revise(&mut b, "the grelvix budget is 5");
    let (from_a, _) = export(&a, "project/bnd").unwrap();
    let s = import(&mut b, &from_a, false).unwrap();
    assert_eq!(s.forks.len(), 1);
    let (rival, local) = &s.forks[0];
    assert_eq!(rival, &a_head);
    assert_eq!(local, &b_head);
    // Local head still circulates; rival waits at the desk.
    let hits = kumbarium_store::recall(
      &b.library,
      "grelvix budget",
      &["project/bnd".into()],
      5,
    )
    .unwrap();
    assert_eq!(hits.len(), 1);
    assert!(hits[0].entry.content.contains("is 5"));
    let inbox = kumbarium_store::pending_in(&b.library).unwrap();
    assert_eq!(inbox.len(), 1);
    assert_eq!(&inbox[0].id, rival);
    let links = kumbarium_store::links_of(&b.library, rival).unwrap();
    assert!(links.iter().any(|l| {
      l.rel == kumbarium_store::Rel::Contradicts && &l.to_id == local
    }));
  }
}
