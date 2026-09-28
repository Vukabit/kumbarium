//! The library as a static site (D-054): `kum doc` renders the
//! circulating record as browsable, offline HTML, the way
//! `cargo doc` renders a crate. This module is the pure half:
//! gather the scoped record, render every page into a path ->
//! bytes map. Writing and opening belong to cli::doc.
//!
//! Every string here was written by an agent, so everything is
//! escaped and fact content renders as preformatted text. The
//! output is deterministic: no build stamp, every listing sorted,
//! times shown as stored (UTC), so the same library and flags
//! produce the same bytes.

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
  shelves: Vec<Shelf>,
  /// Every fact in the build, by id.
  facts: BTreeMap<String, Entry>,
  edges: BTreeMap<String, Vec<Edge>>,
  /// id -> the id it superseded (one step back).
  predecessors: BTreeMap<String, String>,
}

impl Record {
  fn has_shelf(&self, path: &str) -> bool {
    self.shelves.iter().any(|s| s.path == path)
  }
}

/// Gather and render one build.
pub fn build(state: &mut ServerState, opts: &Options) -> Result<Site, String> {
  let record = gather(state, opts)?;
  Ok(render(&record, opts))
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
  let mut entries = kumbarium_store::entries_in(&state.library, None, true)
    .map_err(|e| e.to_string())?;
  entries.retain(|e| {
    e.status == kumbarium_store::Status::Live
      && names.contains(e.namespace.as_str())
      && (opts.all || (e.superseded_by.is_none() && e.retired_at.is_none()))
  });
  entries.sort_by(|a, b| a.id.cmp(&b.id));
  let facts: BTreeMap<String, Entry> =
    entries.into_iter().map(|e| (e.id.clone(), e)).collect();

  let mut edges: BTreeMap<String, Vec<Edge>> = BTreeMap::new();
  let mut predecessors = BTreeMap::new();
  for id in facts.keys() {
    let mut seen = BTreeSet::new();
    for l in kumbarium_store::links_of(&state.library, id)
      .map_err(|e| e.to_string())?
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
    if let Some(prev) = kumbarium_store::predecessor_of(&state.library, id)
      .map_err(|e| e.to_string())?
    {
      predecessors.insert(id.clone(), prev);
    }
  }

  // The docket and the diary, opened only when their shelves
  // exist (a doc build never creates a section file).
  let ns_list: Vec<String> = names.iter().map(|s| s.to_string()).collect();
  let mut tasks: Vec<kumbarium_docket::Task> = Vec::new();
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

  let shelves = chosen
    .into_iter()
    .map(|(path, description)| {
      let mut shelf_tasks: Vec<_> = tasks
        .iter()
        .filter(|t| t.namespace == path)
        .cloned()
        .collect();
      shelf_tasks.sort_by(|a, b| {
        severity_rank(b.severity)
          .cmp(&severity_rank(a.severity))
          .then_with(|| a.id.cmp(&b.id))
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
  Ok(Record {
    shelves,
    facts,
    edges,
    predecessors,
  })
}

fn severity_rank(s: kumbarium_docket::Severity) -> u8 {
  match s {
    kumbarium_docket::Severity::Low => 0,
    kumbarium_docket::Severity::Normal => 1,
    kumbarium_docket::Severity::High => 2,
    kumbarium_docket::Severity::Urgent => 3,
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

/// The prefix that climbs from a page back to the build root.
fn root_of(url: &str) -> String {
  "../".repeat(url.matches('/').count())
}

// ---- rendering --------------------------------------------

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

/// A fact's title: its first non-empty line, markdown heading
/// marks stripped, cut at 90 chars.
fn title_of(content: &str) -> String {
  let line = content
    .lines()
    .map(|l| l.trim().trim_start_matches('#').trim())
    .find(|l| !l.is_empty())
    .unwrap_or("(empty)");
  let mut t: String = line.chars().take(90).collect();
  if line.chars().count() > 90 {
    t.push('\u{2026}');
  }
  t
}

/// Stored UTC timestamp as `YYYY-MM-DD HH:MM UTC`.
fn when(at: &str) -> String {
  let day = at.get(..10).unwrap_or(at);
  match at.get(11..16) {
    Some(time) => format!("{day} {time} UTC"),
    None => day.to_string(),
  }
}

fn kind_label(k: Kind) -> &'static str {
  match k {
    Kind::Decision => "decisions",
    Kind::ProjectState => "project state",
    Kind::Preference => "preferences",
    Kind::Reference => "references",
  }
}

/// The confidence band, painted like the terminal: green holds,
/// yellow is unproven, red is doubtful.
fn band(confidence: f64) -> &'static str {
  if confidence >= 0.6 {
    "hi"
  } else if confidence >= 0.4 {
    "mid"
  } else {
    "lo"
  }
}

fn confidence_badge(e: &Entry) -> String {
  format!(
    "<span class=\"conf {}\" title=\"confidence\">{:.2}</span>",
    band(e.confidence),
    e.confidence
  )
}

fn state_badges(e: &Entry) -> String {
  let mut out = String::new();
  if e.superseded_by.is_some() {
    out.push_str(" <span class=\"badge old\">superseded</span>");
  }
  if e.retired_at.is_some() {
    out.push_str(" <span class=\"badge old\">retired</span>");
  }
  out
}

fn render(record: &Record, opts: &Options) -> Site {
  let mut files = BTreeMap::new();
  files.insert(MARKER.to_string(), marker_text(opts));
  files.insert("static/kum.css".to_string(), CSS.to_string());
  files.insert("static/search.js".to_string(), SEARCH_JS.to_string());
  files.insert("static/index.js".to_string(), search_index(record));

  files.insert("index.html".to_string(), index_page(record, opts));
  for shelf in &record.shelves {
    let url = shelf_url(&shelf.path);
    files.insert(url.clone(), shelf_page(record, shelf, &url));
  }
  for e in record.facts.values() {
    let url = fact_url(&e.id);
    files.insert(url.clone(), fact_page(record, e, &url));
  }
  Site {
    files,
    shelves: record.shelves.len(),
    facts: record.facts.len(),
  }
}

fn marker_text(opts: &Options) -> String {
  format!(
    "kum doc build (D-054); replaced wholesale on every build\n\
     scope: {}\nhistory: {}\n",
    opts.scope.as_deref().unwrap_or("(every shelf)"),
    if opts.all { "included" } else { "heads only" }
  )
}

/// The frame every page shares: header with the search box, the
/// shelf tree as a sidebar, the body.
fn page(
  record: &Record,
  url: &str,
  title: &str,
  current_shelf: Option<&str>,
  body: &str,
) -> String {
  let root = root_of(url);
  let mut nav = String::new();
  // An unregistered parent (`project` above `project/a`) gets an
  // unlinked group label, so a child never reads as belonging
  // to the shelf listed just before it.
  let mut grouped: BTreeSet<String> = BTreeSet::new();
  for shelf in &record.shelves {
    let segs: Vec<&str> = shelf.path.split('/').collect();
    for i in 0..segs.len().saturating_sub(1) {
      let prefix = segs[..=i].join("/");
      if !record.has_shelf(&prefix) && grouped.insert(prefix) {
        nav.push_str(&format!(
          "<li class=\"group\" style=\"--depth:{i}\">{}/</li>\n",
          esc(segs[i])
        ));
      }
    }
    let depth = segs.len() - 1;
    let leaf = shelf.path.rsplit('/').next().unwrap_or(&shelf.path);
    let here = current_shelf == Some(shelf.path.as_str());
    nav.push_str(&format!(
      "<li style=\"--depth:{depth}\"><a href=\"{root}{}\"{} title=\"{}\">{}</a>\
       <span class=\"count\">{}</span></li>\n",
      shelf_url(&shelf.path),
      if here { " class=\"here\"" } else { "" },
      esc(&shelf.path),
      esc(if depth == 0 { &shelf.path } else { leaf }),
      shelf.facts.len(),
    ));
  }
  format!(
    "<!doctype html>\n<html lang=\"en\">\n<head>\n\
     <meta charset=\"utf-8\">\n\
     <meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">\n\
     <meta name=\"generator\" content=\"kum doc\">\n\
     <title>{title} \u{b7} kumbarium</title>\n\
     <link rel=\"stylesheet\" href=\"{root}static/kum.css\">\n\
     </head>\n<body data-root=\"{root}\">\n\
     <header class=\"top\">\n\
     <a class=\"brand\" href=\"{root}index.html\">kum<span>barium</span></a>\n\
     <div class=\"search\"><input id=\"kum-search\" type=\"search\" \
     placeholder=\"Search the shelves (press /)\" autocomplete=\"off\" \
     spellcheck=\"false\" aria-label=\"Search the shelves\">\n\
     <div id=\"kum-results\" hidden></div></div>\n\
     </header>\n\
     <div class=\"frame\">\n\
     <nav class=\"shelves\" aria-label=\"Shelves\"><h2>shelves</h2>\n<ul>\n{nav}</ul></nav>\n\
     <main>\n{body}</main>\n</div>\n\
     <script src=\"{root}static/index.js\"></script>\n\
     <script src=\"{root}static/search.js\"></script>\n\
     </body>\n</html>\n",
    title = esc(title),
  )
}

fn index_page(record: &Record, opts: &Options) -> String {
  let url = "index.html";
  let facts = record.facts.len();
  let matters: usize = record.shelves.iter().map(|s| s.tasks.len()).sum();
  let briefed = record
    .shelves
    .iter()
    .filter(|s| s.briefing.is_some())
    .count();
  let mut body = String::new();
  body.push_str(&format!(
    "<h1>{}</h1>\n<p class=\"lede\">{} \u{b7} {} \u{b7} {} \u{b7} {}</p>\n",
    match &opts.scope {
      Some(s) => format!("The library: <code>{}</code>", esc(s)),
      None => "The library".into(),
    },
    plural(record.shelves.len(), "shelf", "shelves"),
    plural(facts, "fact", "facts"),
    plural(matters, "open matter", "open matters"),
    plural(briefed, "standing briefing", "standing briefings"),
  ));
  if opts.all {
    body.push_str(
      "<p class=\"note\">History included: superseded and retired \
       facts are on the shelves, badged.</p>\n",
    );
  }
  body.push_str(
    "<table class=\"list\">\n<thead><tr><th>shelf</th><th>about</th>\
     <th class=\"n\">facts</th><th class=\"n\">matters</th>\
     <th>briefing</th></tr></thead>\n<tbody>\n",
  );
  for shelf in &record.shelves {
    let depth = shelf.path.matches('/').count();
    body.push_str(&format!(
      "<tr><td style=\"--depth:{depth}\" class=\"tree\"><a href=\"{}\">{}</a></td>\
       <td>{}</td><td class=\"n\">{}</td><td class=\"n\">{}</td><td>{}</td></tr>\n",
      shelf_url(&shelf.path),
      esc(&shelf.path),
      esc(&shelf.description),
      shelf.facts.len(),
      shelf.tasks.len(),
      match &shelf.briefing {
        Some(h) => format!("<span class=\"dim\">{}</span>", when(&h.updated_at)),
        None => String::new(),
      },
    ));
  }
  body.push_str("</tbody>\n</table>\n");
  page(record, url, "The library", None, &body)
}

fn plural(n: usize, one: &str, many: &str) -> String {
  format!("{n} {}", if n == 1 { one } else { many })
}

fn fact_row(e: &Entry, root: &str) -> String {
  let tags = if e.tags.is_empty() {
    String::new()
  } else {
    let t: Vec<String> = e
      .tags
      .iter()
      .map(|t| format!("<span class=\"tag\">{}</span>", esc(t)))
      .collect();
    format!(" <span class=\"tags\">{}</span>", t.join(" "))
  };
  format!(
    "<li><a class=\"id\" href=\"{root}{}\">{}</a> \
     <a href=\"{root}{}\">{}</a> {}{}{}</li>\n",
    fact_url(&e.id),
    short_id(&e.id),
    fact_url(&e.id),
    esc(&title_of(&e.content)),
    confidence_badge(e),
    state_badges(e),
    tags,
  )
}

fn shelf_page(record: &Record, shelf: &Shelf, url: &str) -> String {
  let root = root_of(url);
  let mut body = String::new();
  body.push_str(&crumbs(record, &root, &shelf.path, None));
  body.push_str(&format!("<h1><code>{}</code></h1>\n", esc(&shelf.path)));
  if !shelf.description.is_empty() {
    body.push_str(&format!(
      "<p class=\"lede\">{}</p>\n",
      esc(&shelf.description)
    ));
  }

  let children: Vec<&Shelf> = record
    .shelves
    .iter()
    .filter(|s| {
      s.path
        .strip_prefix(&shelf.path)
        .and_then(|r| r.strip_prefix('/'))
        .is_some_and(|r| !r.contains('/'))
    })
    .collect();
  if !children.is_empty() {
    body.push_str("<h2>shelves within</h2>\n<ul class=\"plain\">\n");
    for c in children {
      body.push_str(&format!(
        "<li><a href=\"{root}{}\">{}</a> <span class=\"dim\">{}</span></li>\n",
        shelf_url(&c.path),
        esc(&c.path),
        plural(c.facts.len(), "fact", "facts"),
      ));
    }
    body.push_str("</ul>\n");
  }

  if let Some(h) = &shelf.briefing {
    body.push_str(&format!(
      "<h2>standing briefing</h2>\n<p class=\"meta\">left by \
       <b>{}</b> \u{b7} {} \u{b7} <span class=\"id\">{}</span></p>\n\
       <pre class=\"body briefing\">{}</pre>\n",
      esc(&h.agent_id),
      when(&h.updated_at),
      short_id(&h.id),
      esc(&h.content),
    ));
  }

  if !shelf.tasks.is_empty() {
    body.push_str("<h2>open matters</h2>\n<ul class=\"matters\">\n");
    for t in &shelf.tasks {
      body.push_str(&format!(
        "<li><span class=\"sev {s}\">{s}</span> <span class=\"id\">{}</span> {}{}</li>\n",
        short_id(&t.id),
        esc(&t.content),
        match &t.goal {
          Some(g) => format!(" <span class=\"dim\">goal {}</span>", esc(g)),
          None => String::new(),
        },
        s = t.severity.as_str(),
      ));
    }
    body.push_str("</ul>\n");
  }

  if shelf.facts.is_empty() {
    body.push_str("<p class=\"dim\">No facts on this shelf.</p>\n");
  }
  for kind in KIND_ORDER {
    let of_kind: Vec<&Entry> =
      shelf.facts.iter().filter(|e| e.kind == kind).collect();
    if of_kind.is_empty() {
      continue;
    }
    body.push_str(&format!(
      "<h2>{} <span class=\"count\">{}</span></h2>\n<ul class=\"facts\">\n",
      kind_label(kind),
      of_kind.len()
    ));
    for e in of_kind {
      body.push_str(&fact_row(e, &root));
    }
    body.push_str("</ul>\n");
  }
  page(record, url, &shelf.path, Some(&shelf.path), &body)
}

/// Breadcrumbs: library / each namespace ancestor / the fact.
/// Only ancestors that are shelves in this build link; an
/// unregistered or out-of-scope parent is plain text.
fn crumbs(record: &Record, root: &str, ns: &str, fact: Option<&str>) -> String {
  let mut out =
    format!("<p class=\"crumbs\"><a href=\"{root}index.html\">library</a>");
  let mut acc = String::new();
  for seg in ns.split('/') {
    if !acc.is_empty() {
      acc.push('/');
    }
    acc.push_str(seg);
    if record.has_shelf(&acc) {
      out.push_str(&format!(
        " / <a href=\"{root}{}\">{}</a>",
        shelf_url(&acc),
        esc(seg)
      ));
    } else {
      out.push_str(&format!(" / {}", esc(seg)));
    }
  }
  if let Some(f) = fact {
    out.push_str(&format!(" / <span class=\"id\">{f}</span>"));
  }
  out.push_str("</p>\n");
  out
}

/// A reference to another fact: a link when it is in this
/// build, the short id marked external when it is not.
fn fact_ref(record: &Record, root: &str, id: &str) -> String {
  match record.facts.get(id) {
    Some(e) => format!(
      "<a class=\"id\" href=\"{root}{}\">{}</a> {} <span class=\"dim\">{}</span>",
      fact_url(id),
      short_id(id),
      esc(&title_of(&e.content)),
      esc(&e.namespace),
    ),
    None => format!(
      "<span class=\"id\">{}</span> <span class=\"external\">not in this build</span>",
      esc(short_id(id))
    ),
  }
}

fn fact_page(record: &Record, e: &Entry, url: &str) -> String {
  let root = root_of(url);
  let mut body = String::new();
  body.push_str(&crumbs(record, &root, &e.namespace, Some(short_id(&e.id))));
  body.push_str(&format!(
    "<h1>{}</h1>\n<p class=\"meta\"><span class=\"kind\">{}</span> \
     {}{}</p>\n",
    esc(&title_of(&e.content)),
    e.kind.as_str(),
    confidence_badge(e),
    state_badges(e),
  ));
  body.push_str(&format!("<pre class=\"body\">{}</pre>\n", esc(&e.content)));

  let mut rows: Vec<(&str, String)> = vec![
    ("id", format!("<code>{}</code>", esc(&e.id))),
    (
      "shelf",
      format!(
        "<a href=\"{root}{}\">{}</a>",
        shelf_url(&e.namespace),
        esc(&e.namespace)
      ),
    ),
    ("written by", esc(&e.agent_id)),
    ("source", esc(&e.source)),
    ("created", when(&e.created_at)),
    ("updated", when(&e.updated_at)),
  ];
  if let Some(at) = &e.last_confirmed_at {
    rows.push(("confirmed", when(at)));
  }
  if let Some(at) = &e.retired_at {
    rows.push(("retired", when(at)));
  }
  rows.push((
    "confidence",
    match &e.confidence_basis {
      Some(basis) => format!("{} {}", confidence_badge(e), esc(basis)),
      None => format!(
        "{} <span class=\"dim\">neutral prior, no janitor pass yet</span>",
        confidence_badge(e)
      ),
    },
  ));
  if !e.tags.is_empty() {
    let t: Vec<String> = e
      .tags
      .iter()
      .map(|t| format!("<span class=\"tag\">{}</span>", esc(t)))
      .collect();
    rows.push(("tags", t.join(" ")));
  }
  if let Some(note) = &e.note {
    rows.push(("note", esc(note)));
  }
  body.push_str("<dl class=\"facts-meta\">\n");
  for (k, v) in rows {
    body.push_str(&format!("<dt>{k}</dt><dd>{v}</dd>\n"));
  }
  body.push_str("</dl>\n");

  let prev = record.predecessors.get(&e.id);
  if prev.is_some() || e.superseded_by.is_some() {
    body.push_str("<h2>chain</h2>\n<ul class=\"plain\">\n");
    if let Some(p) = prev {
      body.push_str(&format!(
        "<li><span class=\"rel\">supersedes</span> {}</li>\n",
        fact_ref(record, &root, p)
      ));
    }
    if let Some(next) = &e.superseded_by {
      body.push_str(&format!(
        "<li><span class=\"rel\">superseded by</span> {}</li>\n",
        fact_ref(record, &root, next)
      ));
    }
    body.push_str("</ul>\n");
  }

  if let Some(edges) = record.edges.get(&e.id) {
    body.push_str("<h2>edges</h2>\n<ul class=\"plain\">\n");
    for edge in edges {
      let rel = edge.rel.replace('_', " ");
      let label = if edge.outgoing {
        rel
      } else {
        format!("\u{2190} {rel}")
      };
      body.push_str(&format!(
        "<li><span class=\"rel{}\">{}</span> {}</li>\n",
        if edge.rel == "contradicts" {
          " warn"
        } else {
          ""
        },
        esc(&label),
        fact_ref(record, &root, &edge.other)
      ));
    }
    body.push_str("</ul>\n");
  }
  page(
    record,
    url,
    &title_of(&e.content),
    Some(&e.namespace),
    &body,
  )
}

/// The search index as a script (window data, never fetched), so
/// search works from file:// in every browser.
fn search_index(record: &Record) -> String {
  let mut rows = Vec::new();
  for shelf in &record.shelves {
    rows.push(serde_json::json!({
      "i": "",
      "u": shelf_url(&shelf.path),
      "t": shelf.path,
      "n": shelf.description,
      "k": "shelf",
      "x": "",
    }));
  }
  for e in record.facts.values() {
    let text: String = e.content.chars().take(4000).collect();
    rows.push(serde_json::json!({
      "i": short_id(&e.id),
      "u": fact_url(&e.id),
      "t": title_of(&e.content),
      "n": e.namespace,
      "k": e.kind.as_str(),
      "x": format!("{} {}", text, e.tags.join(" ")).to_lowercase(),
    }));
  }
  let json = serde_json::to_string(&rows).unwrap_or_else(|_| "[]".into());
  format!("window.KUM_INDEX = {json};\n")
}

const SEARCH_JS: &str = r#"// kum doc search: substring match over the prebuilt index.
// Results are built with textContent only; nothing from the
// index is ever parsed as HTML.
(function () {
  var root = document.body.getAttribute("data-root") || "";
  var input = document.getElementById("kum-search");
  var out = document.getElementById("kum-results");
  if (!input || !out) return;
  var index = window.KUM_INDEX || [];

  function clear() {
    while (out.firstChild) out.removeChild(out.firstChild);
  }
  function span(cls, text) {
    var s = document.createElement("span");
    s.className = cls;
    s.textContent = text;
    return s;
  }
  function run() {
    var q = input.value.trim().toLowerCase();
    clear();
    if (!q) { out.hidden = true; return; }
    var terms = q.split(/\s+/);
    var hits = [];
    for (var i = 0; i < index.length; i++) {
      var d = index[i];
      var title = d.t.toLowerCase();
      var hay = title + " " + d.n.toLowerCase() + " " + d.k + " " + d.i + " " + d.x;
      var score = 0, ok = true;
      for (var j = 0; j < terms.length; j++) {
        if (hay.indexOf(terms[j]) < 0) { ok = false; break; }
        score += title.indexOf(terms[j]) >= 0 ? 4 : 1;
        if (d.i === terms[j]) score += 10;
      }
      if (ok) hits.push([score, d]);
    }
    hits.sort(function (a, b) {
      return b[0] - a[0] || (a[1].u < b[1].u ? -1 : 1);
    });
    hits.slice(0, 40).forEach(function (h) {
      var d = h[1];
      var a = document.createElement("a");
      a.href = root + d.u;
      a.className = "hit";
      a.appendChild(span("kind", d.k));
      if (d.i) a.appendChild(span("id", d.i));
      a.appendChild(span("title", d.t));
      a.appendChild(span("dim", d.k === "shelf" ? d.n : d.n));
      out.appendChild(a);
    });
    if (!hits.length) {
      var p = document.createElement("p");
      p.className = "none";
      p.textContent = "nothing on the shelves matches";
      out.appendChild(p);
    }
    out.hidden = false;
  }
  input.addEventListener("input", run);
  input.addEventListener("keydown", function (e) {
    if (e.key === "Enter") {
      var first = out.querySelector("a.hit");
      if (first) window.location.href = first.href;
    }
  });
  document.addEventListener("keydown", function (e) {
    if (e.key === "/" && document.activeElement !== input) {
      e.preventDefault();
      input.focus();
    } else if (e.key === "Escape" && document.activeElement === input) {
      input.value = "";
      run();
      input.blur();
    }
  });
  var m = window.location.search.match(/[?&]search=([^&]*)/);
  if (m) {
    try { input.value = decodeURIComponent(m[1].replace(/\+/g, " ")); } catch (_) {}
    run();
  }
})();
"#;

const CSS: &str = r##"/* kum doc: the terminal palette, on paper. */
:root {
  --bg: #fbfaf7;
  --panel: #f2efe8;
  --ink: #23211d;
  --dim: #7a746a;
  --rule: #e2ddd2;
  --accent: #8a5a2b;
  --cyan: #0f7c86;
  --green: #2f7d32;
  --yellow: #9a6b00;
  --red: #b3261e;
  --magenta: #8e3a8e;
  --mono: ui-monospace, "SF Mono", Menlo, Consolas, monospace;
  --sans: -apple-system, BlinkMacSystemFont, "Segoe UI", Inter, sans-serif;
}
@media (prefers-color-scheme: dark) {
  :root {
    --bg: #17161a;
    --panel: #1f1e23;
    --ink: #e7e3da;
    --dim: #948d80;
    --rule: #302e35;
    --accent: #d9a066;
    --cyan: #56c2cc;
    --green: #7cc47f;
    --yellow: #e0b44c;
    --red: #ef7a72;
    --magenta: #d18ad1;
  }
}
* { box-sizing: border-box; }
html, body { margin: 0; }
body {
  background: var(--bg);
  color: var(--ink);
  font: 15px/1.55 var(--sans);
}
a { color: var(--accent); text-decoration: none; }
a:hover { text-decoration: underline; }
code, pre, .id, .brand, .shelves, .tag, .kind, .conf, .sev, .badge, .rel {
  font-family: var(--mono);
}
.top {
  position: sticky; top: 0; z-index: 5;
  display: flex; align-items: center; gap: 20px;
  padding: 10px 20px;
  background: var(--panel);
  border-bottom: 1px solid var(--rule);
}
.brand { font-weight: 700; font-size: 17px; color: var(--ink); }
.brand span { color: var(--dim); font-weight: 400; }
.search { position: relative; flex: 1; max-width: 640px; }
#kum-search {
  width: 100%; padding: 7px 12px;
  font: 14px var(--mono);
  color: var(--ink); background: var(--bg);
  border: 1px solid var(--rule); border-radius: 6px;
}
#kum-search:focus { outline: 2px solid var(--accent); outline-offset: -1px; }
#kum-results {
  position: absolute; left: 0; right: 0; top: calc(100% + 4px);
  max-height: 70vh; overflow-y: auto;
  background: var(--bg); border: 1px solid var(--rule); border-radius: 6px;
  box-shadow: 0 8px 24px rgba(0,0,0,.12);
}
#kum-results .hit {
  display: flex; gap: 10px; align-items: baseline;
  padding: 7px 12px; color: var(--ink);
  border-bottom: 1px solid var(--rule);
}
#kum-results .hit:hover { background: var(--panel); text-decoration: none; }
#kum-results .title { flex: 1; min-width: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
#kum-results .none { margin: 0; padding: 10px 12px; color: var(--dim); }
.frame { display: flex; align-items: flex-start; }
.shelves {
  position: sticky; top: 53px;
  flex: 0 0 250px; max-height: calc(100vh - 53px); overflow-y: auto;
  padding: 18px 12px 24px 20px; font-size: 13px;
  border-right: 1px solid var(--rule);
}
.shelves h2 { margin: 0 0 8px; font-size: 11px; letter-spacing: .08em; text-transform: uppercase; color: var(--dim); }
.shelves ul { list-style: none; margin: 0; padding: 0; }
.shelves li {
  display: flex; justify-content: space-between; gap: 8px;
  padding: 2px 0 2px calc(var(--depth) * 14px);
}
.shelves li a { color: var(--ink); overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
.shelves li.group { color: var(--dim); padding-top: 8px; }
.shelves li a.here { color: var(--accent); font-weight: 700; }
.count { color: var(--dim); font-weight: 400; font-size: .85em; }
main { flex: 1; min-width: 0; max-width: 980px; padding: 22px 32px 64px; }
h1 { font-size: 26px; line-height: 1.25; margin: 4px 0 8px; overflow-wrap: anywhere; }
h1 code { font-size: .95em; }
h2 {
  font-size: 13px; letter-spacing: .08em; text-transform: uppercase;
  color: var(--dim); margin: 30px 0 10px;
  padding-bottom: 5px; border-bottom: 1px solid var(--rule);
}
.lede { color: var(--dim); margin: 0 0 16px; }
.note { background: var(--panel); padding: 8px 12px; border-radius: 6px; }
.crumbs { font-size: 13px; color: var(--dim); margin: 0 0 4px; font-family: var(--mono); }
.meta { color: var(--dim); display: flex; gap: 8px; flex-wrap: wrap; align-items: center; }
.dim { color: var(--dim); }
.id { color: var(--cyan); font-size: .92em; }
a.id:hover { text-decoration: underline; }
.kind { color: var(--magenta); font-size: .85em; }
.conf { font-size: .8em; padding: 0 5px; border-radius: 4px; border: 1px solid currentColor; }
.conf.hi { color: var(--green); }
.conf.mid { color: var(--yellow); }
.conf.lo { color: var(--red); }
.badge { font-size: .75em; padding: 0 5px; border-radius: 4px; background: var(--panel); color: var(--dim); }
.tag { font-size: .78em; color: var(--dim); }
.tag::before { content: "#"; }
.sev { font-size: .78em; padding: 0 5px; border-radius: 4px; background: var(--panel); }
.sev.urgent { color: var(--red); }
.sev.high { color: var(--yellow); }
.sev.normal { color: var(--ink); }
.sev.low { color: var(--dim); }
.rel { color: var(--dim); font-size: .85em; margin-right: 6px; }
.rel.warn { color: var(--red); font-weight: 700; }
.external { color: var(--dim); font-style: italic; font-size: .85em; }
pre.body {
  background: var(--panel); border: 1px solid var(--rule); border-radius: 6px;
  padding: 14px 16px; margin: 16px 0;
  white-space: pre-wrap; overflow-wrap: anywhere;
  font-size: 13.5px; line-height: 1.55;
}
ul.facts, ul.matters, ul.plain { list-style: none; padding: 0; margin: 0; }
ul.facts li, ul.matters li, ul.plain li { padding: 5px 0; border-bottom: 1px dashed var(--rule); overflow-wrap: anywhere; }
ul.facts li .id { margin-right: 4px; }
table.list { width: 100%; border-collapse: collapse; font-size: 14px; }
table.list th { text-align: left; font-size: 11px; letter-spacing: .08em; text-transform: uppercase; color: var(--dim); font-weight: 600; border-bottom: 1px solid var(--rule); padding: 6px 8px; }
table.list td { padding: 6px 8px; border-bottom: 1px solid var(--rule); vertical-align: top; }
table.list .n { text-align: right; font-variant-numeric: tabular-nums; }
td.tree { padding-left: calc(8px + var(--depth) * 16px); font-family: var(--mono); white-space: nowrap; }
dl.facts-meta { display: grid; grid-template-columns: max-content 1fr; gap: 4px 16px; margin: 0; font-size: 14px; }
dl.facts-meta dt { color: var(--dim); }
dl.facts-meta dd { margin: 0; overflow-wrap: anywhere; }
@media (max-width: 760px) {
  .frame { display: flex; flex-direction: column; }
  .frame main { order: 1; max-width: none; width: 100%; }
  .shelves { order: 2; width: 100%; flex-basis: auto; border-top: 1px solid var(--rule); position: static; max-height: none; border-right: 0; border-bottom: 1px solid var(--rule); padding: 12px 16px; }
  main { padding: 16px 16px 48px; }
  .top { padding: 10px 16px; gap: 12px; }
  table.list th:nth-child(2), table.list td:nth-child(2) { display: none; }
  dl.facts-meta { grid-template-columns: 1fr; }
  dl.facts-meta dd { margin-bottom: 6px; }
}
"##;

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

  fn put(state: &mut ServerState, ns: &str, content: &str) -> String {
    kumbarium_store::remember(
      &mut state.library,
      &kumbarium_store::NewEntry {
        namespace: ns.into(),
        kind: Kind::Decision,
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
    let id = put(&mut s, "global", "<script>alert('x')</script> & more");
    let site = build(&mut s, &opts(None)).unwrap();
    for (path, text) in &site.files {
      if path.ends_with(".html") {
        assert!(!text.contains("<script>alert"), "raw script in {path}");
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
  fn history_only_with_all() {
    let mut s = state_with(&["global"]);
    let old = put(&mut s, "global", "the old truth");
    let new = kumbarium_store::supersede(
      &mut s.library,
      &old,
      &kumbarium_store::NewEntry {
        namespace: "global".into(),
        kind: Kind::Decision,
        content: "the new truth".into(),
        agent_id: "writer-b".into(),
        source: "test".into(),
        tags: vec![],
        status: kumbarium_store::Status::Live,
      },
      None,
    )
    .unwrap()
    .id;
    let heads = build(&mut s, &opts(None)).unwrap();
    assert_eq!(heads.facts, 1);
    assert!(heads.files[&fact_url(&new)].contains("not in this build"));
    let all = build(
      &mut s,
      &Options {
        scope: None,
        all: true,
      },
    )
    .unwrap();
    assert_eq!(all.facts, 2);
    assert!(all.files[&fact_url(&old)].contains("superseded by"));
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
  fn dot_segments_cannot_escape_the_build() {
    assert_eq!(shelf_url("a/../b"), "shelf/a/_../b/index.html");
    assert_eq!(root_of("shelf/a/b/index.html"), "../../../");
    assert_eq!(root_of("index.html"), "");
  }
}
