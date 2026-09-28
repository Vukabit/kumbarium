//! The library as a static site (D-054, refined by D-055): `kum
//! doc` renders the circulating record as browsable, offline
//! HTML, the way a documentation generator renders an API. This
//! module is the
//! pure half: gather the scoped record, render every page into a
//! path -> bytes map. Writing and opening belong to cli::doc.
//!
//! Every string here was written by an agent, so everything is
//! escaped (md.rs owns the only markdown-to-HTML path). The
//! output is deterministic: no build stamp, every listing sorted,
//! times shown as stored (UTC), so the same library and flags
//! produce the same bytes.

mod assets;
mod md;
mod pages;

use std::collections::{BTreeMap, BTreeSet};

use kumbarium_store::{Entry, Kind, short_id};

use super::tools::ServerState;

/// The marker file every build carries at its root: proof a
/// directory is a doc build (and so safe to replace wholesale).
pub const MARKER: &str = ".kumbarium-doc";

/// Kinds in shelf-page order: what was decided, where things
/// stand, how the human wants it, where to look.
const KIND_ORDER: [Kind; 4] = [
  Kind::Decision,
  Kind::ProjectState,
  Kind::Preference,
  Kind::Reference,
];

pub struct Options {
  /// Build this namespace and its descendants; None = every
  /// registered shelf.
  pub scope: Option<String>,
  /// Include superseded and retired entries (history).
  pub all: bool,
}

/// One rendered build: published path -> content.
pub struct Site {
  pub files: BTreeMap<String, String>,
  pub shelves: usize,
  pub facts: usize,
}

struct Shelf {
  path: String,
  description: String,
  facts: Vec<Entry>,
  tasks: Vec<kumbarium_docket::Task>,
  briefing: Option<kumbarium_handoff::Handoff>,
}

/// One typed edge as seen from a fact page.
struct Edge {
  rel: &'static str,
  outgoing: bool,
  other: String,
}

struct Record {
  scope: Option<String>,
  all: bool,
  shelves: Vec<Shelf>,
  /// Every fact in the build, by id.
  facts: BTreeMap<String, Entry>,
  /// id -> (title, summary line), for facts in the build.
  titles: BTreeMap<String, (String, String)>,
  edges: BTreeMap<String, Vec<Edge>>,
  /// Supersession chains touching the build, oldest first, keyed
  /// by their first id (the history page's name); circulating
  /// versions only.
  chains: BTreeMap<String, Vec<Entry>>,
  /// fact id (any version) -> its chain's key.
  chain_of: BTreeMap<String, String>,
  /// Intra-doc link targets: `D-054`, a `[[tag]]`, a short id
  /// (a fact or an open matter; the value is the page URL), a
  /// shelf's leaf name. A short id or leaf two share maps to
  /// None (ambiguous links never guess).
  d_numbers: BTreeMap<String, String>,
  tags: BTreeMap<String, String>,
  shorts: BTreeMap<String, Option<String>>,
  leaves: BTreeMap<String, Option<String>>,
  /// Each open matter's version chain, oldest first (regrades
  /// and rewordings), keyed by the matter's live id.
  matter_chains: BTreeMap<String, Vec<kumbarium_docket::Task>>,
}

/// Gather and render one build.
pub fn build(state: &mut ServerState, opts: &Options) -> Result<Site, String> {
  let record = gather(state, opts)?;
  Ok(pages::render(&record))
}

fn in_scope(path: &str, scope: Option<&str>) -> bool {
  match scope {
    None => true,
    Some(s) => {
      path == s
        || path
          .strip_prefix(s)
          .is_some_and(|rest| rest.starts_with('/'))
    }
  }
}

fn gather(state: &mut ServerState, opts: &Options) -> Result<Record, String> {
  let scope = match &opts.scope {
    Some(raw) => {
      let s = kumbarium_librarian::normalize_namespace(raw);
      kumbarium_librarian::validate_namespace(&s)
        .map_err(|e| format!("invalid namespace: {e}"))?;
      Some(s)
    }
    None => None,
  };
  let registered =
    kumbarium_store::namespaces(&state.library).map_err(|e| e.to_string())?;
  let chosen: Vec<(String, String)> = registered
    .into_iter()
    .filter(|(path, _, _)| in_scope(path, scope.as_deref()))
    .map(|(path, desc, _)| (path, desc))
    .collect();
  if chosen.is_empty() {
    return Err(match scope {
      Some(s) => format!(
        "no registered namespace at or under {s:?}; the shelves: \
         kum namespace list"
      ),
      None => "no namespaces registered yet: kum namespace add <path>".into(),
    });
  }
  let names: BTreeSet<&str> = chosen.iter().map(|(p, _)| p.as_str()).collect();

  // Circulating record only: pending and rejected material has
  // not been admitted by the desk and never renders.
  let every: BTreeMap<String, Entry> =
    kumbarium_store::entries_in(&state.library, None, true)
      .map_err(|e| e.to_string())?
      .into_iter()
      .filter(|e| e.status == kumbarium_store::Status::Live)
      .map(|e| (e.id.clone(), e))
      .collect();
  let facts: BTreeMap<String, Entry> = every
    .values()
    .filter(|e| {
      names.contains(e.namespace.as_str())
        && (opts.all || (e.superseded_by.is_none() && e.retired_at.is_none()))
    })
    .map(|e| (e.id.clone(), e.clone()))
    .collect();

  let mut edges: BTreeMap<String, Vec<Edge>> = BTreeMap::new();
  let mut chains: BTreeMap<String, Vec<Entry>> = BTreeMap::new();
  let mut chain_of: BTreeMap<String, String> = BTreeMap::new();
  for (id, e) in &facts {
    let mut seen = BTreeSet::new();
    for l in kumbarium_store::links_of(&state.library, id)
      .map_err(|err| err.to_string())?
    {
      let outgoing = l.from_id == *id;
      let other = if outgoing { l.to_id } else { l.from_id };
      if seen.insert((l.rel.as_str(), outgoing, other.clone())) {
        edges.entry(id.clone()).or_default().push(Edge {
          rel: l.rel.as_str(),
          outgoing,
          other,
        });
      }
    }
    if chain_of.contains_key(id) {
      continue;
    }
    let chained = e.superseded_by.is_some()
      || kumbarium_store::predecessor_of(&state.library, id)
        .map_err(|err| err.to_string())?
        .is_some();
    if !chained {
      continue;
    }
    let versions: Vec<Entry> =
      kumbarium_store::version_history(&state.library, id)
        .map_err(|err| err.to_string())?
        .iter()
        .filter_map(|v| every.get(v).cloned())
        .collect();
    let Some(key) = versions.first().map(|v| v.id.clone()) else {
      continue;
    };
    for v in &versions {
      chain_of.insert(v.id.clone(), key.clone());
    }
    chains.insert(key, versions);
  }

  // The docket and the diary, opened only when their shelves
  // exist (a doc build never creates a section file).
  let ns_list: Vec<String> = names.iter().map(|s| s.to_string()).collect();
  let mut tasks: Vec<kumbarium_docket::Task> = Vec::new();
  let mut matter_chains = BTreeMap::new();
  if state.docket.is_some()
    || state.docket_path.exists()
    || state.docket_path.as_os_str().is_empty()
  {
    let conn = state.docket()?;
    tasks = kumbarium_docket::tasks_in(conn, Some(&ns_list), false)
      .map_err(|e| e.to_string())?;
    tasks.retain(|t| {
      t.status == kumbarium_docket::Status::Live
        && t.state == kumbarium_docket::TaskState::Open
        && t.superseded_by.is_none()
    });
    tasks.sort_by(|a, b| a.id.cmp(&b.id));
    for t in &tasks {
      let chain =
        kumbarium_docket::history(conn, &t.id).map_err(|e| e.to_string())?;
      matter_chains.insert(t.id.clone(), chain);
    }
  }
  let mut briefings = BTreeMap::new();
  if state.handoff.is_some()
    || state.handoff_path.exists()
    || state.handoff_path.as_os_str().is_empty()
  {
    let conn = state.handoff()?;
    for ns in &ns_list {
      if let Some(h) =
        kumbarium_handoff::standing(conn, ns).map_err(|e| e.to_string())?
      {
        briefings.insert(ns.clone(), h);
      }
    }
  }

  let shelves: Vec<Shelf> = chosen
    .into_iter()
    .map(|(path, description)| {
      let mut shelf_tasks: Vec<_> = tasks
        .iter()
        .filter(|t| t.namespace == path)
        .cloned()
        .collect();
      shelf_tasks.sort_by(|a, b| {
        b.severity.cmp(&a.severity).then_with(|| a.id.cmp(&b.id))
      });
      Shelf {
        facts: facts
          .values()
          .filter(|e| e.namespace == path)
          .cloned()
          .collect(),
        tasks: shelf_tasks,
        briefing: briefings.remove(&path),
        path,
        description,
      }
    })
    .collect();

  // Intra-doc targets. Facts iterate by id, so "first claim
  // wins" is deterministic.
  let mut titles = BTreeMap::new();
  let mut d_numbers = BTreeMap::new();
  let mut tags = BTreeMap::new();
  let mut shorts: BTreeMap<String, Option<String>> = BTreeMap::new();
  for (id, e) in &facts {
    titles.insert(id.clone(), md::title_and_summary(&e.content));
    for t in &e.tags {
      let t = t.to_ascii_lowercase();
      if let Some(d) = d_number(&t) {
        d_numbers.entry(d).or_insert_with(|| id.clone());
      }
      tags.entry(t).or_insert_with(|| id.clone());
    }
    let head: String = e.content.trim_start().chars().take(6).collect();
    if let Some(d) = d_number(&head) {
      d_numbers.entry(d).or_insert_with(|| id.clone());
    }
    shorts
      .entry(short_id(id).to_string())
      .and_modify(|v| *v = None)
      .or_insert_with(|| Some(fact_url(id)));
  }
  for id in matter_chains.keys() {
    shorts
      .entry(short_id(id).to_string())
      .and_modify(|v| *v = None)
      .or_insert_with(|| Some(matter_url(id)));
  }
  let mut leaves: BTreeMap<String, Option<String>> = BTreeMap::new();
  for s in &shelves {
    let leaf = s.path.rsplit('/').next().unwrap_or(&s.path).to_string();
    leaves
      .entry(leaf)
      .and_modify(|v| *v = None)
      .or_insert_with(|| Some(s.path.clone()));
  }

  Ok(Record {
    scope,
    all: opts.all,
    shelves,
    facts,
    titles,
    edges,
    chains,
    chain_of,
    d_numbers,
    tags,
    shorts,
    leaves,
    matter_chains,
  })
}

/// `d-054` / `D-054` (optionally followed by a non-digit) as the
/// canonical `D-054`.
fn d_number(s: &str) -> Option<String> {
  let b = s.as_bytes();
  let ok = b.len() >= 5
    && (b[0] == b'd' || b[0] == b'D')
    && b[1] == b'-'
    && b[2..5].iter().all(u8::is_ascii_digit)
    && b.get(5).is_none_or(|c| !c.is_ascii_digit());
  ok.then(|| format!("D-{}", &s[2..5]))
}

impl Record {
  fn has_shelf(&self, path: &str) -> bool {
    self.shelves.iter().any(|s| s.path == path)
  }

  fn title(&self, e: &Entry) -> String {
    match self.titles.get(&e.id) {
      Some((t, _)) => t.clone(),
      None => md::title_and_summary(&e.content).0,
    }
  }

  /// Resolve an intra-doc reference to a root-relative URL.
  fn resolve(&self, r: md::Ref<'_>) -> Option<String> {
    let fact = |id: &String| fact_url(id);
    match r {
      md::Ref::Wiki(name) => {
        let lower = name.to_ascii_lowercase();
        if let Some(d) = d_number(&lower).filter(|_| lower.len() == 5) {
          return self.d_numbers.get(&d).map(fact);
        }
        if let Some(id) = self.tags.get(&lower) {
          return Some(fact(id));
        }
        if self.has_shelf(&lower) {
          return Some(shelf_url(&lower));
        }
        if let Some(Some(path)) = self.leaves.get(&lower) {
          return Some(shelf_url(path));
        }
        self.shorts.get(&lower).cloned().flatten()
      }
      md::Ref::Token(tok) => {
        if tok.len() == 5
          && let Some(d) = d_number(tok).filter(|_| tok.starts_with('D'))
        {
          return self.d_numbers.get(&d).map(fact);
        }
        let hex =
          |s: &str| s.bytes().all(|b| b.is_ascii_hexdigit() || b == b'-');
        if tok.len() == 8 && hex(tok) {
          return self.shorts.get(tok).cloned().flatten();
        }
        if tok.len() == 36 && hex(tok) {
          if self.facts.contains_key(tok) {
            return Some(fact_url(tok));
          }
          if self.matter_chains.contains_key(tok) {
            return Some(matter_url(tok));
          }
        }
        if tok.contains('/') && self.has_shelf(tok) {
          return Some(shelf_url(tok));
        }
        None
      }
    }
  }
}

// ---- paths ------------------------------------------------

/// A namespace segment as a directory name. Registered segments
/// are [a-z0-9._-]; a leading dot is escaped so `.` and `..`
/// can never walk out of the build.
fn segment_dir(seg: &str) -> String {
  if seg.starts_with('.') {
    format!("_{seg}")
  } else {
    seg.to_string()
  }
}

fn shelf_url(ns: &str) -> String {
  let dirs: Vec<String> = ns.split('/').map(segment_dir).collect();
  format!("shelf/{}/index.html", dirs.join("/"))
}

fn fact_url(id: &str) -> String {
  format!("fact/{id}.html")
}

fn matter_url(id: &str) -> String {
  format!("matter/{id}.html")
}

fn history_url(key: &str) -> String {
  format!("history/{key}.html")
}

/// The prefix that climbs from a page back to the build root.
fn root_of(url: &str) -> String {
  "../".repeat(url.matches('/').count())
}

fn esc(s: &str) -> String {
  let mut out = String::with_capacity(s.len());
  for c in s.chars() {
    match c {
      '&' => out.push_str("&amp;"),
      '<' => out.push_str("&lt;"),
      '>' => out.push_str("&gt;"),
      '"' => out.push_str("&quot;"),
      '\'' => out.push_str("&#39;"),
      c => out.push(c),
    }
  }
  out
}

#[cfg(test)]
mod tests {
  use super::*;

  fn state_with(shelves: &[&str]) -> ServerState {
    let state = ServerState::in_memory();
    for s in shelves {
      // `global` ships registered; every other shelf is added.
      if *s != "global" {
        kumbarium_store::register_namespace(&state.library, s, "a test shelf")
          .unwrap();
      }
    }
    state
  }

  fn put_tagged(
    state: &mut ServerState,
    ns: &str,
    content: &str,
    tags: &[&str],
  ) -> String {
    kumbarium_store::remember(
      &mut state.library,
      &kumbarium_store::NewEntry {
        namespace: ns.into(),
        kind: Kind::Decision,
        content: content.into(),
        agent_id: "writer-a".into(),
        source: "test".into(),
        tags: tags.iter().map(|t| t.to_string()).collect(),
        status: kumbarium_store::Status::Live,
        actor_id: None,
      },
    )
    .unwrap()
    .id
  }

  fn put(state: &mut ServerState, ns: &str, content: &str) -> String {
    put_tagged(state, ns, content, &["t1"])
  }

  fn supersede(state: &mut ServerState, old: &str, content: &str) -> String {
    kumbarium_store::supersede(
      &mut state.library,
      old,
      &kumbarium_store::NewEntry {
        namespace: "global".into(),
        kind: Kind::Decision,
        content: content.into(),
        agent_id: "writer-b".into(),
        source: "test".into(),
        tags: vec![],
        status: kumbarium_store::Status::Live,
        actor_id: None,
      },
      None,
    )
    .unwrap()
    .id
  }

  fn opts(scope: Option<&str>) -> Options {
    Options {
      scope: scope.map(str::to_string),
      all: false,
    }
  }

  #[test]
  fn bare_build_covers_every_shelf_and_fact() {
    let mut s = state_with(&["global", "project/a", "project/b"]);
    let a = put(&mut s, "project/a", "the grelvix runs hot");
    let b = put(&mut s, "project/b", "the plorvane is cold");
    let site = build(&mut s, &opts(None)).unwrap();
    assert_eq!((site.shelves, site.facts), (3, 2));
    for path in [
      "index.html",
      "all.html",
      MARKER,
      "shelf/global/index.html",
      "shelf/project/a/index.html",
      "shelf/project/b/index.html",
      &fact_url(&a),
      &fact_url(&b),
    ] {
      assert!(site.files.contains_key(path), "missing {path}");
    }
  }

  #[test]
  fn scope_narrows_to_the_subtree_and_edges_leaving_it_go_external() {
    let mut s = state_with(&["project", "project/a", "projectx", "global"]);
    let a = put(&mut s, "project/a", "inside the build");
    let g = put(&mut s, "global", "outside the build");
    kumbarium_store::link(&s.library, &a, &g, kumbarium_store::Rel::RelatesTo)
      .unwrap();
    let site = build(&mut s, &opts(Some("project"))).unwrap();
    // `project` and `project/a`, never the lookalike `projectx`.
    assert_eq!(site.shelves, 2);
    assert!(!site.files.contains_key("shelf/projectx/index.html"));
    assert!(!site.files.contains_key(&fact_url(&g)));
    let page = &site.files[&fact_url(&a)];
    assert!(page.contains("not in this build"));
  }

  #[test]
  fn unknown_scope_is_refused() {
    let mut s = state_with(&["global"]);
    let err = build(&mut s, &opts(Some("project/nope"))).err().unwrap();
    assert!(err.contains("no registered namespace"), "{err}");
  }

  #[test]
  fn content_is_escaped_everywhere() {
    let mut s = state_with(&["global"]);
    let id = put_tagged(
      &mut s,
      "global",
      "<script>alert('x')</script> & more",
      &["<b>tag</b>"],
    );
    let site = build(&mut s, &opts(None)).unwrap();
    for (path, text) in &site.files {
      if path.ends_with(".html") {
        assert!(!text.contains("<script>alert"), "raw script in {path}");
        assert!(!text.contains("<b>tag"), "raw tag in {path}");
      }
    }
    assert!(site.files[&fact_url(&id)].contains("&lt;script&gt;alert"));
  }

  #[test]
  fn builds_are_byte_identical() {
    let mut s = state_with(&["global", "project/a"]);
    put(&mut s, "global", "one");
    put(&mut s, "project/a", "two");
    let first = build(&mut s, &opts(None)).unwrap().files;
    let second = build(&mut s, &opts(None)).unwrap().files;
    assert_eq!(first, second);
  }

  #[test]
  fn history_pages_and_banners() {
    let mut s = state_with(&["global"]);
    let old = put(&mut s, "global", "the old truth");
    let new = supersede(&mut s, &old, "the new truth");
    let heads = build(&mut s, &opts(None)).unwrap();
    assert_eq!(heads.facts, 1);
    // The chain's history page renders even for a heads build:
    // it is the fact's source view.
    let hist = history_url(&old);
    assert!(heads.files.contains_key(&hist));
    assert!(heads.files[&hist].contains("the old truth"));
    assert!(heads.files[&fact_url(&new)].contains(&hist));
    let all = build(
      &mut s,
      &Options {
        scope: None,
        all: true,
      },
    )
    .unwrap();
    assert_eq!(all.facts, 2);
    let old_page = &all.files[&fact_url(&old)];
    assert!(old_page.contains("class=\"banner superseded\""));
  }

  #[test]
  fn contradicts_raises_a_disputed_banner() {
    let mut s = state_with(&["global"]);
    let a = put(&mut s, "global", "the sky is green");
    let b = put(&mut s, "global", "the sky is blue");
    kumbarium_store::link(
      &s.library,
      &b,
      &a,
      kumbarium_store::Rel::Contradicts,
    )
    .unwrap();
    let site = build(&mut s, &opts(None)).unwrap();
    for id in [&a, &b] {
      assert!(site.files[&fact_url(id)].contains("class=\"banner disputed\""));
    }
  }

  #[test]
  fn intra_doc_links_resolve() {
    let mut s = state_with(&["global", "project/a"]);
    let d =
      put_tagged(&mut s, "global", "D-054: the site decision", &["d-054"]);
    let w = put_tagged(&mut s, "global", "a named fact", &["no-em-dashes"]);
    let src = put(
      &mut s,
      "project/a",
      &format!(
        "see D-054, [[no-em-dashes]], {} and project/a.",
        short_id(&w)
      ),
    );
    let site = build(&mut s, &opts(None)).unwrap();
    let page = &site.files[&fact_url(&src)];
    assert!(page.contains(&format!("href=\"../{}\">D-054</a>", fact_url(&d))));
    assert!(
      page.contains(&format!("href=\"../{}\">no-em-dashes</a>", fact_url(&w)))
    );
    assert!(page.contains(&format!(
      "href=\"../{}\">{}</a>",
      fact_url(&w),
      short_id(&w)
    )));
    assert!(
      page.contains("href=\"../shelf/project/a/index.html\">project/a</a>")
    );
  }

  #[test]
  fn unregistered_parents_group_and_never_link() {
    let mut s = state_with(&["global", "project/a"]);
    let id = put(&mut s, "project/a", "a fact");
    let site = build(&mut s, &opts(None)).unwrap();
    let page = &site.files[&fact_url(&id)];
    assert!(
      page.contains("<li class=\"group\" style=\"--depth:0\">project/</li>")
    );
    assert!(
      !page.contains("shelf/project/index.html"),
      "dead crumb link"
    );
  }

  #[test]
  fn search_index_carries_summaries_and_tags() {
    let mut s = state_with(&["global"]);
    put_tagged(
      &mut s,
      "global",
      "Title here. The summary line.",
      &["alpha"],
    );
    let site = build(&mut s, &opts(None)).unwrap();
    let index = &site.files["static/search-index.js"];
    assert!(index.contains("\"s\":\"The summary line.\""));
    assert!(index.contains("\"g\":\"alpha\""));
  }

  #[test]
  fn open_matters_get_pages_links_and_search_entries() {
    let mut s = state_with(&["global", "project/a"]);
    let task = {
      let conn = s.docket().unwrap();
      let t = kumbarium_docket::file_task(
        conn,
        &kumbarium_docket::NewTask {
          namespace: "project/a".into(),
          content: "Rotate the grelvix key. Before the audit.".into(),
          agent_id: "writer-a".into(),
          source: "test".into(),
          severity: kumbarium_docket::Severity::High,
          goal: Some("2026-10-01".into()),
          status: kumbarium_docket::Status::Live,
        },
      )
      .unwrap();
      t.id
    };
    let mentions = put(
      &mut s,
      "project/a",
      &format!("see {} for the rotation", short_id(&task)),
    );
    let site = build(&mut s, &opts(None)).unwrap();
    let page = &site.files[&matter_url(&task)];
    assert!(page.contains("Rotate the grelvix key."));
    assert!(page.contains("goal 2026-10-01"));
    assert!(page.contains(&format!("kum task history {}", short_id(&task))));
    let shelf = &site.files["shelf/project/a/index.html"];
    assert!(
      shelf.contains(&format!("href=\"../../../{}\"", matter_url(&task)))
    );
    assert!(site.files[&fact_url(&mentions)].contains(&format!(
      "href=\"../{}\">{}</a>",
      matter_url(&task),
      short_id(&task)
    )));
    assert!(site.files["static/search-index.js"].contains("\"k\":\"matter\""));
  }

  #[test]
  fn dot_segments_cannot_escape_the_build() {
    assert_eq!(shelf_url("a/../b"), "shelf/a/_../b/index.html");
    assert_eq!(root_of("shelf/a/b/index.html"), "../../../");
    assert_eq!(root_of("index.html"), "");
  }
}
