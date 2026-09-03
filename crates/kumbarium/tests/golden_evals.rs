//! The golden-eval runner: replays evals/golden.toml against a
//! fresh in-memory store per case, using the librarian's chain
//! resolution, the way the MCP server will. Lexical cases must
//! put the expected entry at rank 1; `semantic = true` cases
//! (beyond FTS reach, D-011) are reported, never failed on.

use std::collections::HashMap;

use serde::Deserialize;

#[derive(Deserialize)]
struct Golden {
  #[serde(rename = "case")]
  cases: Vec<Case>,
}

#[derive(Deserialize)]
struct Case {
  name: String,
  scope: String,
  query: String,
  expect: String,
  #[serde(default)]
  semantic: bool,
  entries: Vec<Seed>,
}

#[derive(Deserialize)]
struct Seed {
  key: String,
  kind: String,
  ns: String,
  content: String,
  #[serde(default)]
  superseded_by: Option<String>,
}

fn load() -> Golden {
  let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../../evals/golden.toml");
  let raw = std::fs::read_to_string(path).expect("read golden.toml");
  toml::from_str(&raw).expect("parse golden.toml")
}

fn new_entry(seed: &Seed) -> kumbarium_store::NewEntry {
  kumbarium_store::NewEntry {
    namespace: seed.ns.clone(),
    kind: kumbarium_store::Kind::parse(&seed.kind)
      .unwrap_or_else(|| panic!("unknown kind {:?}", seed.kind)),
    content: seed.content.trim().to_string(),
    agent_id: "golden-eval".into(),
    source: "evals/golden.toml".into(),
    tags: Vec::new(),
    status: kumbarium_store::Status::Live,
  }
}

/// Seed one case into a fresh store; returns key -> entry id.
fn seed_case(
  conn: &mut kumbarium_store::Connection,
  case: &Case,
) -> HashMap<String, String> {
  for seed in &case.entries {
    if seed.ns != "global"
      && kumbarium_store::namespace_id(conn, &seed.ns)
        .unwrap()
        .is_none()
    {
      kumbarium_store::register_namespace(conn, &seed.ns, "eval").unwrap();
    }
  }
  let by_key: HashMap<&str, &Seed> =
    case.entries.iter().map(|s| (s.key.as_str(), s)).collect();
  let mut ids = HashMap::new();
  // Chained pairs: the old entry is remembered, then superseding
  // it CREATES the new one (the store owns chain writes).
  for seed in &case.entries {
    if let Some(new_key) = &seed.superseded_by {
      let old = kumbarium_store::remember(conn, &new_entry(seed)).unwrap();
      let new_seed = by_key
        .get(new_key.as_str())
        .unwrap_or_else(|| panic!("bad superseded_by {new_key:?}"));
      let new =
        kumbarium_store::supersede(conn, &old.id, &new_entry(new_seed), None)
          .unwrap();
      ids.insert(seed.key.clone(), old.id);
      ids.insert(new_key.clone(), new.id);
    }
  }
  // Everything not already created by a chain.
  for seed in &case.entries {
    if !ids.contains_key(&seed.key) {
      let e = kumbarium_store::remember(conn, &new_entry(seed)).unwrap();
      ids.insert(seed.key.clone(), e.id);
    }
  }
  ids
}

#[test]
fn golden_cases_rank_the_expected_entry_first() {
  let golden = load();
  assert!(!golden.cases.is_empty(), "golden set is empty");
  let mut failures = Vec::new();
  for case in &golden.cases {
    let mut conn = kumbarium_store::open_in_memory().unwrap();
    let ids = seed_case(&mut conn, case);
    let expect_id = ids
      .get(&case.expect)
      .unwrap_or_else(|| panic!("bad expect key in {}", case.name));
    let chain = kumbarium_librarian::namespace_chain(&case.scope).unwrap();
    let hits = kumbarium_store::recall(&conn, &case.query, &chain, 10).unwrap();
    let top_is_expected =
      hits.first().map(|h| h.entry.id == *expect_id) == Some(true);
    let outcome = if top_is_expected { "PASS" } else { "MISS" };
    let tag = if case.semantic { " (semantic)" } else { "" };
    eprintln!("golden {}: {outcome}{tag}", case.name);
    if !top_is_expected && !case.semantic {
      failures.push(case.name.clone());
    }
  }
  assert!(
    failures.is_empty(),
    "lexical golden cases missed: {failures:?}"
  );
}
