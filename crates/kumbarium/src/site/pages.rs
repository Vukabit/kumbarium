//! Page rendering: the shared frame (header, sidebar, search
//! view, help and settings panels) and each page kind. Shapes
//! follow the API-reference convention: an out-of-band row beside
//! the title, anchored
//! collapsible sections, item tables with summary lines,
//! deprecation-style banners, and a history page as the source
//! view.

use kumbarium_store::{Entry, Kind, short_id};

use super::super::diff;
use super::{
  KIND_ORDER, MARKER, Record, Shelf, Site, assets, esc, fact_url, history_url,
  matter_url, md, root_of, shelf_url,
};

pub(super) fn render(record: &Record) -> Site {
  let mut files = std::collections::BTreeMap::new();
  files.insert(MARKER.to_string(), marker_text(record));
  files.insert("static/kum.css".to_string(), assets::CSS.to_string());
  files.insert("static/kum.js".to_string(), assets::JS.to_string());
  files.insert("static/search-index.js".to_string(), search_index(record));
  files.insert("index.html".to_string(), index_page(record));
  files.insert("all.html".to_string(), all_page(record));
  for shelf in &record.shelves {
    let url = shelf_url(&shelf.path);
    files.insert(url.clone(), shelf_page(record, shelf, &url));
  }
  for e in record.facts.values() {
    let url = fact_url(&e.id);
    files.insert(url.clone(), fact_page(record, e, &url));
  }
  for (key, versions) in &record.chains {
    let url = history_url(key);
    files.insert(url.clone(), history_page(record, versions, &url));
  }
  for chain in record.matter_chains.values() {
    if let Some(head) = chain.last() {
      let url = matter_url(&head.id);
      files.insert(url.clone(), matter_page(record, chain, &url));
    }
  }
  Site {
    files,
    shelves: record.shelves.len(),
    facts: record.facts.len(),
  }
}

fn marker_text(record: &Record) -> String {
  format!(
    "kum doc build (D-054); replaced wholesale on every build\n\
     scope: {}\nhistory: {}\n",
    record.scope.as_deref().unwrap_or("(every shelf)"),
    if record.all { "included" } else { "heads only" }
  )
}

// ---- small pieces -----------------------------------------

/// Stored UTC timestamp as `YYYY-MM-DD HH:MM UTC`.
fn when(at: &str) -> String {
  let day = at.get(..10).unwrap_or(at);
  match at.get(11..16) {
    Some(time) => format!("{day} {time} UTC"),
    None => day.to_string(),
  }
}

fn day(at: &str) -> &str {
  at.get(..10).unwrap_or(at)
}

fn kind_label(k: Kind) -> &'static str {
  match k {
    Kind::Decision => "Decisions",
    Kind::ProjectState => "Project state",
    Kind::Preference => "Preferences",
    Kind::Reference => "References",
  }
}

fn kind_anchor(k: Kind) -> &'static str {
  match k {
    Kind::Decision => "decisions",
    Kind::ProjectState => "project-state",
    Kind::Preference => "preferences",
    Kind::Reference => "references",
  }
}

fn plural(n: usize, one: &str, many: &str) -> String {
  format!("{n} {}", if n == 1 { one } else { many })
}

/// The confidence band, painted like the terminal: green holds,
/// yellow is unproven, red is doubtful.
fn confidence_badge(e: &Entry) -> String {
  let band = if e.confidence >= 0.6 {
    "hi"
  } else if e.confidence >= 0.4 {
    "mid"
  } else {
    "lo"
  };
  format!(
    "<span class=\"conf {band}\" title=\"confidence\">{:.2}</span>",
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

fn tag_list(tags: &[String]) -> String {
  if tags.is_empty() {
    return String::new();
  }
  let t: Vec<String> = tags
    .iter()
    .map(|t| format!("<span class=\"tag\">{}</span>", esc(t)))
    .collect();
  format!("<span class=\"tags\">{}</span>", t.join(" "))
}

const COPY_ICON: &str = "<svg viewBox=\"0 0 16 16\" width=\"14\" height=\"14\" \
aria-hidden=\"true\"><rect x=\"5\" y=\"5\" width=\"9\" height=\"9\" rx=\"1.5\" \
fill=\"none\" stroke=\"currentColor\" stroke-width=\"1.4\"/>\
<path d=\"M11 3.5V3a1 1 0 0 0-1-1H3a1 1 0 0 0-1 1v7a1 1 0 0 0 1 1h.5\" \
fill=\"none\" \
stroke=\"currentColor\" stroke-width=\"1.4\"/></svg>";

const GEAR_ICON: &str = "<svg viewBox=\"0 0 16 16\" width=\"15\" height=\"15\" \
aria-hidden=\"true\"><circle cx=\"8\" cy=\"8\" r=\"2.2\" fill=\"none\" \
stroke=\"currentColor\" stroke-width=\"1.4\"/><path d=\"M8 1.5v2M8 12.5v2M1.5 \
8h2M12.5 8h2M3.4 3.4l1.4 1.4M11.2 11.2l1.4 1.4M3.4 12.6l1.4-1.4M11.2 \
4.8l1.4-1.4\" \
stroke=\"currentColor\" stroke-width=\"1.4\" stroke-linecap=\"round\"/></svg>";

/// A copy button: copies a ready-to-run command line.
fn copy_button(text: &str) -> String {
  format!(
    "<button class=\"copy\" type=\"button\" data-copy=\"{0}\" \
     title=\"Copy: {0}\" aria-label=\"Copy {0}\">{COPY_ICON}</button>",
    esc(text)
  )
}

/// An anchored, collapsible section.
fn section(id: &str, label: &str, count: Option<usize>, inner: &str) -> String {
  let count = count
    .map(|n| format!(" <span class=\"count\">{n}</span>"))
    .unwrap_or_default();
  format!(
    "<details class=\"sec\" id=\"{id}\" open>\n<summary><h2>{label}{count}\
     <a class=\"anchor\" href=\"#{id}\" aria-label=\"Link to this section\">\
     \u{a7}</a></h2></summary>\n{inner}</details>\n",
    id = esc(id),
  )
}

fn md_html(record: &Record, root: &str, text: &str) -> String {
  let resolve = |r: md::Ref<'_>| record.resolve(r);
  md::render(
    text,
    &md::Refs {
      root,
      resolve: &resolve,
    },
  )
}

fn md_inline(record: &Record, root: &str, text: &str) -> String {
  let resolve = |r: md::Ref<'_>| record.resolve(r);
  md::inline(
    text,
    &md::Refs {
      root,
      resolve: &resolve,
    },
  )
}

/// A reference to another fact: a link when it is in this
/// build, the short id marked external when it is not.
fn fact_ref(record: &Record, root: &str, id: &str) -> String {
  match record.facts.get(id) {
    Some(e) => format!(
      "<a class=\"id\" href=\"{root}{}\">{}</a> <a href=\"{root}{}\">{}</a> \
       <span class=\"dim\">{}</span>",
      fact_url(id),
      short_id(id),
      fact_url(id),
      esc(&record.title(e)),
      esc(&e.namespace),
    ),
    None => format!(
      "<span class=\"id\">{}</span> <span class=\"external\">not in this \
       build</span>",
      esc(short_id(id))
    ),
  }
}

/// Breadcrumbs: library / each namespace ancestor / the tail.
/// Only ancestors that are shelves in this build link; an
/// unregistered or out-of-scope parent is plain text.
fn crumbs(record: &Record, root: &str, ns: &str, tail: Option<&str>) -> String {
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
  if let Some(t) = tail {
    out.push_str(&format!(" / <span class=\"id\">{t}</span>"));
  }
  out.push_str("</p>\n");
  out
}

/// The title row: h1 on the left, the out-of-band links
/// (history, copy, collapse-all) on the right.
fn title_row(h1: &str, out_of_band: &str) -> String {
  format!(
    "<div class=\"title-row\"><h1>{h1}</h1><div class=\"oob\">{out_of_band}\
     <button id=\"kum-toggle-all\" type=\"button\" title=\"Collapse all \
     sections (+)\">[\u{2212}]</button></div></div>\n"
  )
}

// ---- the frame --------------------------------------------

struct Page<'a> {
  url: &'a str,
  title: String,
  shelf: Option<&'a str>,
  /// "On this page": (anchor, label, count).
  toc: Vec<(String, String, Option<usize>)>,
  /// A page-specific sidebar block (a fact's siblings).
  local: String,
  body: String,
}

fn frame(record: &Record, p: Page) -> String {
  let root = root_of(p.url);
  let mut side = String::new();
  if !p.toc.is_empty() {
    side.push_str("<div class=\"side-block\"><h2>On this page</h2><ul>\n");
    for (anchor, label, count) in &p.toc {
      side.push_str(&format!(
        "<li><a href=\"#{}\">{}</a>{}</li>\n",
        esc(anchor),
        esc(label),
        count
          .map(|n| format!("<span class=\"count\">{n}</span>"))
          .unwrap_or_default()
      ));
    }
    side.push_str("</ul></div>\n");
  }
  side.push_str(&p.local);
  side.push_str(&shelf_tree(record, &root, p.shelf));
  format!(
    "<!doctype html>\n<html lang=\"en\">\n<head>\n\
     <meta charset=\"utf-8\">\n\
     <meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">\n\
     <meta name=\"generator\" content=\"kum doc\">\n\
     <title>{title} \u{b7} kumbarium</title>\n\
     <script>try{{var t=localStorage.getItem(\"kum-theme\");\
     if(t&&t!==\"system\")document.documentElement\
     .setAttribute(\"data-theme\",t)\
     }}catch(e){{}}</script>\n\
     <link rel=\"stylesheet\" href=\"{root}static/kum.css\">\n\
     </head>\n<body data-root=\"{root}\">\n\
     <header class=\"top\">\n\
     <a class=\"brand\" href=\"{root}index.html\">kum<span>barium</span></a>\n\
     <div class=\"search\"><input id=\"kum-search\" type=\"search\" \
     placeholder=\"Search the shelves (S or /; kind: shelf: tag:)\" \
     autocomplete=\"off\" spellcheck=\"false\" aria-label=\"Search the \
     shelves\"></div>\n\
     <nav class=\"tools\"><a href=\"{root}all.html\" title=\"Every fact on \
     one page\">all facts</a>\
     <button id=\"kum-help-btn\" class=\"pop-btn\" type=\"button\" \
     title=\"Keyboard shortcuts (?)\" aria-label=\"Help\">?</button>\
     <button id=\"kum-settings-btn\" class=\"pop-btn\" type=\"button\" \
     title=\"Settings\" aria-label=\"Settings\">{GEAR_ICON}</button></nav>\n\
     {HELP}{SETTINGS}\
     </header>\n\
     <div class=\"frame\">\n\
     <nav class=\"side\" aria-label=\"Sidebar\">\n{side}</nav>\n\
     <main>\n<div id=\"main-content\">\n{body}</div>\n\
     <section id=\"search-view\" hidden></section>\n</main>\n</div>\n\
     <script src=\"{root}static/search-index.js\"></script>\n\
     <script src=\"{root}static/kum.js\"></script>\n\
     </body>\n</html>\n",
    title = esc(&p.title),
    body = p.body,
  )
}

const HELP: &str = "<div id=\"kum-help\" class=\"pop\" hidden>\n\
<h3>Keyboard</h3><dl class=\"keys\">\
<dt><kbd>S</kbd> <kbd>/</kbd></dt><dd>search</dd>\
<dt><kbd>?</kbd></dt><dd>this help</dd>\
<dt><kbd>\u{2191}</kbd> <kbd>\u{2193}</kbd></dt><dd>move through results</dd>\
<dt><kbd>\u{2190}</kbd> <kbd>\u{2192}</kbd></dt><dd>switch result tabs</dd>\
<dt><kbd>Enter</kbd></dt><dd>open the result</dd>\
<dt><kbd>+</kbd></dt><dd>collapse or expand every section</dd>\
<dt><kbd>Esc</kbd></dt><dd>close; clear the search</dd></dl>\n\
<h3>Search</h3><dl class=\"keys\">\
<dt><code>kind:decision</code></dt><dd>one kind: decision, state, preference, \
reference, matter, shelf</dd>\
<dt><code>decision:rollout</code></dt><dd>a kind and a term together</dd>\
<dt><code>shelf:ambyte</code></dt><dd>facts on shelves whose path contains \
it</dd>\
<dt><code>tag:d-054</code> <code>#d-054</code></dt><dd>facts with a tag \
starting with it</dd>\
<dt><code>1cb8e972</code></dt><dd>a short id ranks its fact first</dd></dl>\n\
</div>\n";

const SETTINGS: &str = "<div id=\"kum-settings\" class=\"pop\" hidden>\n\
<h3>Theme</h3><div class=\"themes\">\
<label><input type=\"radio\" name=\"theme\" value=\"system\"> system</label>\
<label><input type=\"radio\" name=\"theme\" value=\"paper\"> paper</label>\
<label><input type=\"radio\" name=\"theme\" value=\"slate\"> slate</label>\
<label><input type=\"radio\" name=\"theme\" value=\"lamp\"> lamp</label>\
</div>\n<p class=\"dim\">Remembered in this browser only.</p>\n</div>\n";

/// The shelf tree. An unregistered parent (`project` above
/// `project/a`) gets an unlinked group label, so a child never
/// reads as belonging to the shelf listed just before it.
fn shelf_tree(record: &Record, root: &str, current: Option<&str>) -> String {
  let mut nav = String::from(
    "<div class=\"side-block\"><h2>Shelves</h2><ul class=\"tree\">\n",
  );
  let mut grouped = std::collections::BTreeSet::new();
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
    let label = if depth == 0 {
      shelf.path.as_str()
    } else {
      segs[depth]
    };
    nav.push_str(&format!(
      "<li style=\"--depth:{depth}\"><a href=\"{root}{}\"{} title=\"{}\">{}</a>\
       <span class=\"count\">{}</span></li>\n",
      shelf_url(&shelf.path),
      if current == Some(shelf.path.as_str()) {
        " class=\"here\""
      } else {
        ""
      },
      esc(&shelf.path),
      esc(label),
      shelf.facts.len(),
    ));
  }
  nav.push_str("</ul></div>\n");
  nav
}

// ---- pages ------------------------------------------------

fn index_page(record: &Record) -> String {
  let url = "index.html";
  let matters: usize = record.shelves.iter().map(|s| s.tasks.len()).sum();
  let briefed = record
    .shelves
    .iter()
    .filter(|s| s.briefing.is_some())
    .count();
  let h1 = match &record.scope {
    Some(s) => format!("The library: <code>{}</code>", esc(s)),
    None => "The library".into(),
  };
  let mut body = title_row(&h1, "");
  body.push_str(&format!(
    "<p class=\"lede\">{} \u{b7} {} \u{b7} {} \u{b7} {}</p>\n",
    plural(record.shelves.len(), "shelf", "shelves"),
    plural(record.facts.len(), "fact", "facts"),
    plural(matters, "open matter", "open matters"),
    plural(briefed, "standing briefing", "standing briefings"),
  ));
  if record.all {
    body.push_str(
      "<p class=\"note\">History included: superseded and retired facts \
       are on the shelves, badged.</p>\n",
    );
  }
  let mut table = String::from(
    "<div class=\"table-wrap\"><table class=\"list\">\n<thead><tr>\
     <th>shelf</th><th>about</th><th class=\"n\">facts</th>\
     <th class=\"n\">matters</th><th>briefing</th></tr></thead>\n<tbody>\n",
  );
  for shelf in &record.shelves {
    let depth = shelf.path.matches('/').count();
    table.push_str(&format!(
      "<tr><td style=\"--depth:{depth}\" class=\"tree\">\
       <a href=\"{}\">{}</a></td><td>{}</td><td class=\"n\">{}</td>\
       <td class=\"n\">{}</td><td>{}</td></tr>\n",
      shelf_url(&shelf.path),
      esc(&shelf.path),
      esc(&shelf.description),
      shelf.facts.len(),
      shelf.tasks.len(),
      match &shelf.briefing {
        Some(h) => format!("<span class=\"dim\">{}</span>", day(&h.updated_at)),
        None => String::new(),
      },
    ));
  }
  table.push_str("</tbody>\n</table></div>\n");
  body.push_str(&section(
    "shelves",
    "Shelves",
    Some(record.shelves.len()),
    &table,
  ));
  frame(
    record,
    Page {
      url,
      title: "The library".into(),
      shelf: None,
      toc: vec![(
        "shelves".into(),
        "Shelves".into(),
        Some(record.shelves.len()),
      )],
      local: String::new(),
      body,
    },
  )
}

/// all.html: every fact on one page, by shelf and kind.
fn all_page(record: &Record) -> String {
  let url = "all.html";
  let mut body = title_row("All facts", "");
  body.push_str(&format!(
    "<p class=\"lede\">{} on {}</p>\n",
    plural(record.facts.len(), "fact", "facts"),
    plural(record.shelves.len(), "shelf", "shelves"),
  ));
  let mut toc = Vec::new();
  for shelf in record
    .shelves
    .iter()
    .filter(|s| !s.facts.is_empty() || !s.tasks.is_empty())
  {
    let mut inner = String::new();
    if !shelf.tasks.is_empty() {
      inner.push_str(
        "<h3 class=\"sub\">Open matters</h3>\n<ul class=\"compact\">\n",
      );
      for t in &shelf.tasks {
        inner.push_str(&format!(
          "<li><span class=\"sev {s}\">{s}</span> \
           <a class=\"id\" href=\"{}\">{}</a> <a href=\"{}\">{}</a></li>\n",
          matter_url(&t.id),
          short_id(&t.id),
          matter_url(&t.id),
          esc(&md::title_and_summary(&t.content).0),
          s = t.severity.as_str(),
        ));
      }
      inner.push_str("</ul>\n");
    }
    for kind in KIND_ORDER {
      let of_kind: Vec<&Entry> =
        shelf.facts.iter().filter(|e| e.kind == kind).collect();
      if of_kind.is_empty() {
        continue;
      }
      inner.push_str(&format!(
        "<h3 class=\"sub\">{}</h3>\n<ul class=\"compact\">\n",
        kind_label(kind)
      ));
      for e in of_kind {
        inner.push_str(&format!(
          "<li><a class=\"id\" href=\"{}\">{}</a> \
           <a href=\"{}\">{}</a>{}</li>\n",
          fact_url(&e.id),
          short_id(&e.id),
          fact_url(&e.id),
          esc(&record.title(e)),
          state_badges(e),
        ));
      }
      inner.push_str("</ul>\n");
    }
    body.push_str(&section(
      &shelf.path,
      &format!(
        "<a href=\"{}\"><code>{}</code></a>",
        shelf_url(&shelf.path),
        esc(&shelf.path)
      ),
      Some(shelf.facts.len()),
      &inner,
    ));
    toc.push((
      shelf.path.clone(),
      shelf.path.clone(),
      Some(shelf.facts.len()),
    ));
  }
  frame(
    record,
    Page {
      url,
      title: "All facts".into(),
      shelf: None,
      toc,
      local: String::new(),
      body,
    },
  )
}

fn shelf_page(record: &Record, shelf: &Shelf, url: &str) -> String {
  let root = root_of(url);
  let mut body = crumbs(record, &root, &shelf.path, None);
  body.push_str(&title_row(
    &format!("<code>{}</code>", esc(&shelf.path)),
    &copy_button(&format!("kum brief {}", shelf.path)),
  ));
  if !shelf.description.is_empty() {
    body.push_str(&format!(
      "<p class=\"lede\">{}</p>\n",
      esc(&shelf.description)
    ));
  }
  let mut toc = Vec::new();

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
    let mut inner = String::from("<ul class=\"compact\">\n");
    for c in &children {
      inner.push_str(&format!(
        "<li><a href=\"{root}{}\"><code>{}</code></a> \
         <span class=\"dim\">{}</span></li>\n",
        shelf_url(&c.path),
        esc(&c.path),
        plural(c.facts.len(), "fact", "facts"),
      ));
    }
    inner.push_str("</ul>\n");
    body.push_str(&section(
      "within",
      "Shelves within",
      Some(children.len()),
      &inner,
    ));
    toc.push((
      "within".into(),
      "Shelves within".into(),
      Some(children.len()),
    ));
  }

  if let Some(h) = &shelf.briefing {
    let inner = format!(
      "<p class=\"meta\">left by <b>{}</b> \u{b7} {} \u{b7} <span \
       class=\"id\">{}</span></p>\n<div class=\"docblock briefing\">{}</div>\n",
      esc(&h.agent_id),
      when(&h.updated_at),
      short_id(&h.id),
      md_html(record, &root, &h.content),
    );
    body.push_str(&section("briefing", "Standing briefing", None, &inner));
    toc.push(("briefing".into(), "Standing briefing".into(), None));
  }

  if !shelf.tasks.is_empty() {
    let mut inner = String::from("<ul class=\"matters\">\n");
    for t in &shelf.tasks {
      inner.push_str(&format!(
        "<li><span class=\"sev {s}\">{s}</span> <a class=\"id\" \
         href=\"{root}{}\">{}</a> {}{}</li>\n",
        matter_url(&t.id),
        short_id(&t.id),
        md_inline(record, &root, &t.content),
        match &t.goal {
          Some(g) => format!(" <span class=\"dim\">goal {}</span>", esc(g)),
          None => String::new(),
        },
        s = t.severity.as_str(),
      ));
    }
    inner.push_str("</ul>\n");
    body.push_str(&section(
      "matters",
      "Open matters",
      Some(shelf.tasks.len()),
      &inner,
    ));
    toc.push((
      "matters".into(),
      "Open matters".into(),
      Some(shelf.tasks.len()),
    ));
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
    let mut inner = String::from(
      "<div class=\"table-wrap\"><table class=\"items\"><tbody>\n",
    );
    for e in &of_kind {
      let (title, summary) =
        record.titles.get(&e.id).cloned().unwrap_or_default();
      inner.push_str(&format!(
        "<tr><td class=\"item-id\">\
         <a class=\"id\" href=\"{root}{u}\">{}</a></td>\
         <td class=\"item-main\">\
         <a class=\"item-title\" href=\"{root}{u}\">{}</a>{}\
         {}{}</td><td class=\"item-conf\">{}</td></tr>\n",
        short_id(&e.id),
        esc(&title),
        state_badges(e),
        if summary.is_empty() {
          String::new()
        } else {
          format!("<div class=\"summary\">{}</div>", esc(&summary))
        },
        if e.tags.is_empty() {
          String::new()
        } else {
          format!("<div>{}</div>", tag_list(&e.tags))
        },
        confidence_badge(e),
        u = fact_url(&e.id),
      ));
    }
    inner.push_str("</tbody></table></div>\n");
    body.push_str(&section(
      kind_anchor(kind),
      kind_label(kind),
      Some(of_kind.len()),
      &inner,
    ));
    toc.push((
      kind_anchor(kind).into(),
      kind_label(kind).into(),
      Some(of_kind.len()),
    ));
  }
  frame(
    record,
    Page {
      url,
      title: shelf.path.clone(),
      shelf: Some(&shelf.path),
      toc,
      local: String::new(),
      body,
    },
  )
}

/// The fact's content minus a leading heading that is already
/// its h1.
fn body_without_title(content: &str) -> &str {
  let trimmed = content.trim_start();
  let first_end = trimmed.find('\n').unwrap_or(trimmed.len());
  if md::heading(trimmed[..first_end].trim()).is_some() {
    &trimmed[first_end..]
  } else {
    content
  }
}

fn fact_page(record: &Record, e: &Entry, url: &str) -> String {
  let root = root_of(url);
  let short = short_id(&e.id);
  let chain_key = record.chain_of.get(&e.id);

  let mut oob = String::new();
  if let Some(key) = chain_key {
    oob.push_str(&format!(
      "<a class=\"src\" href=\"{root}{}\" title=\"Every version of this \
       fact\">history</a>",
      history_url(key)
    ));
  }
  oob.push_str(&copy_button(&format!("kum show {short}")));

  let mut body = crumbs(record, &root, &e.namespace, Some(short));
  body.push_str(&title_row(&esc(&record.title(e)), &oob));
  body.push_str(&format!(
    "<p class=\"meta\"><span class=\"kind\">{}</span> {}{} <span \
     class=\"dim\">by {} \u{b7} {}</span>{}</p>\n",
    e.kind.as_str(),
    confidence_badge(e),
    state_badges(e),
    esc(&e.agent_id),
    day(&e.created_at),
    if e.tags.is_empty() {
      String::new()
    } else {
      format!(" {}", tag_list(&e.tags))
    },
  ));

  // Deprecation-style banners.
  if let Some(next) = &e.superseded_by {
    let at = chain_key
      .and_then(|k| record.chains.get(k))
      .and_then(|vs| vs.iter().find(|v| &v.id == next))
      .map(|v| format!(" on {}", day(&v.created_at)))
      .unwrap_or_default();
    body.push_str(&format!(
      "<div class=\"banner superseded\"><b>Superseded</b>{at} by {}. This \
       version is kept as history.</div>\n",
      fact_ref(record, &root, next)
    ));
  }
  if let Some(at) = &e.retired_at {
    body.push_str(&format!(
      "<div class=\"banner retired\"><b>Retired</b> {}: true and kept on \
       record, no longer suggested.</div>\n",
      day(at)
    ));
  }
  let edges = record.edges.get(&e.id);
  let disputes: Vec<String> = edges
    .into_iter()
    .flatten()
    .filter(|x| x.rel == "contradicts")
    .map(|x| {
      format!(
        "<li>{} {}</li>",
        if x.outgoing {
          "contradicts"
        } else {
          "contradicted by"
        },
        fact_ref(record, &root, &x.other)
      )
    })
    .collect();
  if !disputes.is_empty() {
    body.push_str(&format!(
      "<div class=\"banner disputed\"><b>Disputed.</b> The desk has not \
       settled this:<ul>{}</ul></div>\n",
      disputes.join("")
    ));
  }

  let mut toc = vec![("content".to_string(), "Content".to_string(), None)];
  body.push_str(&section(
    "content",
    "Content",
    None,
    &format!(
      "<div class=\"docblock\">{}</div>\n",
      md_html(record, &root, body_without_title(&e.content))
    ),
  ));

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
  if let Some(note) = &e.note {
    rows.push(("note", esc(note)));
  }
  let mut dl = String::from("<dl class=\"about\">\n");
  for (k, v) in rows {
    dl.push_str(&format!("<dt>{k}</dt><dd>{v}</dd>\n"));
  }
  dl.push_str("</dl>\n");
  body.push_str(&section("about", "About", None, &dl));
  toc.push(("about".into(), "About".into(), None));

  if let Some(versions) = chain_key.and_then(|k| record.chains.get(k)) {
    let mut inner = String::from("<ol class=\"chain\">\n");
    for (n, v) in versions.iter().enumerate() {
      let this = v.id == e.id;
      let label = if record.facts.contains_key(&v.id) && !this {
        format!(
          "<a class=\"id\" href=\"{root}{}\">{}</a>",
          fact_url(&v.id),
          short_id(&v.id)
        )
      } else {
        format!("<span class=\"id\">{}</span>", short_id(&v.id))
      };
      inner.push_str(&format!(
        "<li{}><a href=\"{root}{}#v{}\">v{}</a> {label} <span \
         class=\"dim\">{} \u{b7} {}</span>{}</li>\n",
        if this { " class=\"here\"" } else { "" },
        history_url(chain_key.map(String::as_str).unwrap_or_default()),
        n + 1,
        n + 1,
        day(&v.created_at),
        esc(&v.agent_id),
        if this {
          " <span class=\"badge\">this version</span>"
        } else {
          ""
        },
      ));
    }
    inner.push_str("</ol>\n");
    body.push_str(&section("chain", "Chain", Some(versions.len()), &inner));
    toc.push(("chain".into(), "Chain".into(), Some(versions.len())));
  }

  if let Some(edges) = edges {
    let mut inner = String::from("<ul class=\"edges\">\n");
    for edge in edges {
      let rel = edge.rel.replace('_', " ");
      let label = if edge.outgoing {
        rel
      } else {
        format!("\u{2190} {rel}")
      };
      inner.push_str(&format!(
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
    inner.push_str("</ul>\n");
    body.push_str(&section("edges", "Edges", Some(edges.len()), &inner));
    toc.push(("edges".into(), "Edges".into(), Some(edges.len())));
  }

  frame(
    record,
    Page {
      url,
      title: record.title(e),
      shelf: Some(&e.namespace),
      toc,
      local: siblings(record, &root, e),
      body,
    },
  )
}

/// An open matter's page: the matter in full, its grade and goal,
/// who filed it, and every version of it (regrades, rewordings).
fn matter_page(
  record: &Record,
  chain: &[kumbarium_docket::Task],
  url: &str,
) -> String {
  let root = root_of(url);
  let t = chain.last().expect("chains are never empty");
  let short = short_id(&t.id);
  let (title, _) = md::title_and_summary(&t.content);
  let mut body = crumbs(record, &root, &t.namespace, Some(short));
  body.push_str(&title_row(
    &esc(&title),
    &copy_button(&format!("kum task history {short}")),
  ));
  body.push_str(&format!(
    "<p class=\"meta\"><span class=\"kind\">open matter</span> <span \
     class=\"sev {s}\">{s}</span>{} <span class=\"dim\">filed by {} \u{b7} \
     {}</span></p>\n",
    match &t.goal {
      Some(g) => format!(" <span class=\"badge\">goal {}</span>", esc(g)),
      None => String::new(),
    },
    esc(&chain[0].agent_id),
    day(&chain[0].created_at),
    s = t.severity.as_str(),
  ));
  let mut toc = vec![("matter".to_string(), "The matter".to_string(), None)];
  body.push_str(&section(
    "matter",
    "The matter",
    None,
    &format!(
      "<div class=\"docblock\">{}</div>\n",
      md_html(record, &root, &t.content)
    ),
  ));
  let mut rows: Vec<(&str, String)> = vec![
    ("id", format!("<code>{}</code>", esc(&t.id))),
    (
      "shelf",
      format!(
        "<a href=\"{root}{}\">{}</a>",
        shelf_url(&t.namespace),
        esc(&t.namespace)
      ),
    ),
    ("severity", t.severity.as_str().to_string()),
    (
      "goal",
      t.goal
        .as_deref()
        .map(esc)
        .unwrap_or_else(|| "none (someday)".into()),
    ),
    ("filed by", esc(&chain[0].agent_id)),
    ("filed", when(&chain[0].created_at)),
    ("updated", when(&t.updated_at)),
  ];
  if !t.source.is_empty() {
    rows.push(("source", esc(&t.source)));
  }
  if let Some(note) = &t.note {
    rows.push(("note", esc(note)));
  }
  let mut dl = String::from("<dl class=\"about\">\n");
  for (k, v) in rows {
    dl.push_str(&format!("<dt>{k}</dt><dd>{v}</dd>\n"));
  }
  dl.push_str("</dl>\n");
  body.push_str(&section("about", "About", None, &dl));
  toc.push(("about".into(), "About".into(), None));
  if chain.len() > 1 {
    let mut inner = String::from("<ol class=\"chain\">\n");
    for (n, v) in chain.iter().enumerate() {
      let prev = n.checked_sub(1).map(|i| &chain[i]);
      let mut changed = Vec::new();
      if let Some(p) = prev {
        if p.severity != v.severity {
          changed.push(format!(
            "severity {} \u{2192} {}",
            p.severity.as_str(),
            v.severity.as_str()
          ));
        }
        if p.goal != v.goal {
          changed.push(format!(
            "goal {} \u{2192} {}",
            p.goal.as_deref().unwrap_or("none"),
            v.goal.as_deref().unwrap_or("none")
          ));
        }
        if p.content != v.content {
          changed.push("reworded".into());
        }
      } else {
        changed.push("filed".into());
      }
      inner.push_str(&format!(
        "<li{}><span class=\"id\">{}</span> <span class=\"dim\">{} \u{b7} \
         {}</span> {}{}</li>\n",
        if v.id == t.id { " class=\"here\"" } else { "" },
        short_id(&v.id),
        day(&v.created_at),
        esc(&v.agent_id),
        esc(&changed.join(", ")),
        v.note
          .as_ref()
          .map(|n| format!(" <span class=\"dim\">({})</span>", esc(n)))
          .unwrap_or_default(),
      ));
    }
    inner.push_str("</ol>\n");
    body.push_str(&section("chain", "Chain", Some(chain.len()), &inner));
    toc.push(("chain".into(), "Chain".into(), Some(chain.len())));
  }
  frame(
    record,
    Page {
      url,
      title,
      shelf: Some(&t.namespace),
      toc,
      local: String::new(),
      body,
    },
  )
}

/// The sidebar's sibling list: the fact's shelf-mates by kind,
/// this one marked.
fn siblings(record: &Record, root: &str, e: &Entry) -> String {
  let Some(shelf) = record.shelves.iter().find(|s| s.path == e.namespace)
  else {
    return String::new();
  };
  let mut out = format!(
    "<div class=\"side-block siblings\">\
     <h2>In <a href=\"{root}{}\">{}</a></h2>\n",
    shelf_url(&shelf.path),
    esc(&shelf.path)
  );
  for kind in KIND_ORDER {
    let of_kind: Vec<&Entry> =
      shelf.facts.iter().filter(|f| f.kind == kind).collect();
    if of_kind.is_empty() {
      continue;
    }
    out.push_str(&format!("<h3>{}</h3><ul>\n", kind_label(kind)));
    for f in of_kind {
      out.push_str(&format!(
        "<li><a href=\"{root}{}\"{} title=\"{t}\">{t}</a></li>\n",
        fact_url(&f.id),
        if f.id == e.id { " class=\"here\"" } else { "" },
        t = esc(&record.title(f)),
      ));
    }
    out.push_str("</ul>\n");
  }
  out.push_str("</div>\n");
  out
}

/// A fact's source view: every circulating version, newest first,
/// each with its content and the change from the one before.
fn history_page(record: &Record, versions: &[Entry], url: &str) -> String {
  let root = root_of(url);
  let head = versions.last().expect("chains are never empty");
  let title = record.title(head);
  let mut body = crumbs(record, &root, &head.namespace, Some("history"));
  body.push_str(&title_row(&format!("History: {}", esc(&title)), ""));
  body.push_str(&format!(
    "<p class=\"lede\">{} \u{b7} first written {} \u{b7} latest {}</p>\n",
    plural(versions.len(), "version", "versions"),
    day(&versions[0].created_at),
    day(&head.created_at),
  ));
  let mut toc = Vec::new();
  for (n, v) in versions.iter().enumerate().rev() {
    let anchor = format!("v{}", n + 1);
    let mut inner = format!(
      "<p class=\"meta\"><span class=\"id\">{}</span> <span class=\"dim\">by \
       {} \u{b7} {}</span>{}{}{}</p>\n",
      short_id(&v.id),
      esc(&v.agent_id),
      when(&v.created_at),
      if v.superseded_by.is_none() {
        " <span class=\"badge now\">current</span>"
      } else {
        ""
      },
      state_badges(v)
        .replace(" <span class=\"badge old\">superseded</span>", ""),
      if record.facts.contains_key(&v.id) {
        format!(" <a href=\"{root}{}\">fact page</a>", fact_url(&v.id))
      } else {
        String::new()
      },
    );
    if let Some(note) = &v.note {
      inner.push_str(&format!("<p class=\"note\">{}</p>\n", esc(note)));
    }
    inner.push_str(&format!(
      "<div class=\"docblock\">{}</div>\n",
      md_html(record, &root, &v.content)
    ));
    if n > 0 {
      let mut diff_html = String::new();
      for (sign, line) in diff::lines(&versions[n - 1].content, &v.content) {
        let cls = match sign {
          '+' => "add",
          '-' => "del",
          _ => "same",
        };
        diff_html.push_str(&format!(
          "<span class=\"{cls}\">{sign} {}</span>\n",
          esc(&line)
        ));
      }
      inner.push_str(&format!(
        "<details class=\"fold\"><summary>Changes from v{}</summary>\
         <pre class=\"diff\">{diff_html}</pre></details>\n",
        n
      ));
    }
    body.push_str(&section(
      &anchor,
      &format!(
        "v{} <span class=\"dim\">{}</span>",
        n + 1,
        day(&v.created_at)
      ),
      None,
      &inner,
    ));
    toc.push((
      anchor.clone(),
      format!("v{} \u{b7} {}", n + 1, day(&v.created_at)),
      None,
    ));
  }
  frame(
    record,
    Page {
      url,
      title: format!("History: {title}"),
      shelf: Some(&head.namespace),
      toc,
      local: String::new(),
      body,
    },
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
      "s": shelf.description,
      "n": shelf.path,
      "k": "shelf",
      "g": "",
      "x": "",
    }));
  }
  for e in record.facts.values() {
    let (title, summary) =
      record.titles.get(&e.id).cloned().unwrap_or_default();
    let text: String = md::plain(&e.content).chars().take(4000).collect();
    rows.push(serde_json::json!({
      "i": short_id(&e.id),
      "u": fact_url(&e.id),
      "t": title,
      "s": summary,
      "n": e.namespace,
      "k": e.kind.as_str(),
      "g": e.tags.join(" ").to_lowercase(),
      "x": text.to_lowercase(),
    }));
  }
  for chain in record.matter_chains.values() {
    let Some(t) = chain.last() else { continue };
    let (title, summary) = md::title_and_summary(&t.content);
    rows.push(serde_json::json!({
      "i": short_id(&t.id),
      "u": matter_url(&t.id),
      "t": title,
      "s": summary,
      "n": t.namespace,
      "k": "matter",
      "g": "",
      "x": format!("{} {}", t.severity.as_str(), md::plain(&t.content))
        .to_lowercase(),
    }));
  }
  let json = serde_json::to_string(&rows).unwrap_or_else(|_| "[]".into());
  format!("window.KUM_INDEX = {json};\n")
}
