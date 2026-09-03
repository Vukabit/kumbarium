//! The Claude Code auto-memory importer. Reads
//! `~/.claude/projects/*/memory/*.md`, maps frontmatter to
//! entries, [[wiki-links]] to relates_to edges (dangling ones
//! fall back to tags), and is idempotent via each entry's
//! `source` field. Dry-run by default; nothing is written
//! without --apply.

use std::path::{Path, PathBuf};

use serde_json::json;

use crate::tools::ServerState;

const AGENT_ID: &str = "claude-code-import";
const OVERSIZE_CHARS: usize = 1500;

/// One parsed memory file, pre-mapping.
struct MemoryFile {
  path: PathBuf,
  name: String,
  mtype: String,
  description: String,
  body: String,
  wiki_links: Vec<String>,
}

/// One row of the import plan.
struct Planned {
  file: MemoryFile,
  namespace: String,
  kind: kumbarium_store::Kind,
  already_imported: Option<String>, // existing entry id
}

pub struct Options {
  pub dirs: Vec<PathBuf>,
  pub apply: bool,
  /// memory-name -> namespace overrides (--map name=ns).
  pub map: Vec<(String, String)>,
}

/// Default source dirs: every `memory/` under
/// `~/.claude/projects/*`.
pub fn default_dirs() -> Vec<PathBuf> {
  let Some(base) = directories::BaseDirs::new() else {
    return Vec::new();
  };
  let projects = base.home_dir().join(".claude/projects");
  let Ok(read) = std::fs::read_dir(&projects) else {
    return Vec::new();
  };
  let mut dirs: Vec<PathBuf> = read
    .filter_map(|e| e.ok())
    .map(|e| e.path().join("memory"))
    .filter(|p| p.is_dir())
    .collect();
  dirs.sort();
  dirs
}

/// Run the import; returns the human-readable report lines.
pub fn run(
  state: &mut ServerState,
  opts: &Options,
) -> Result<Vec<String>, String> {
  let mut report = Vec::new();
  let mut plan: Vec<Planned> = Vec::new();
  for dir in &opts.dirs {
    collect(state, dir, opts, &mut plan, &mut report)?;
  }
  if plan.is_empty() {
    report.push("nothing to import".into());
    return Ok(report);
  }

  report.push(String::new());
  report.push(format!(
    "{:<28} {:<20} {:<13} {}",
    "memory", "namespace", "kind", "status"
  ));
  for p in &plan {
    let size = p.file.body.len();
    let status = match &p.already_imported {
      Some(id) => format!("SKIP (imported as {id})"),
      None if size > OVERSIZE_CHARS => {
        format!("import ({size} chars; consider splitting after)")
      }
      None => "import".into(),
    };
    report.push(format!(
      "{:<28} {:<20} {:<13} {}",
      p.file.name,
      p.namespace,
      p.kind.as_str(),
      status
    ));
  }

  if !opts.apply {
    report.push(String::new());
    report.push("dry run: nothing written; pass --apply".into());
    return Ok(report);
  }

  // First pass: create entries; remember dangling wiki-links as
  // tags only after resolution is known, so resolve first.
  let mut id_by_name: std::collections::HashMap<String, String> = plan
    .iter()
    .filter_map(|p| {
      p.already_imported
        .clone()
        .map(|id| (p.file.name.clone(), id))
    })
    .collect();
  let importable: Vec<&Planned> = plan
    .iter()
    .filter(|p| p.already_imported.is_none())
    .collect();
  let known: std::collections::HashSet<String> =
    plan.iter().map(|p| p.file.name.clone()).collect();
  let mut imported = 0usize;
  for p in &importable {
    let mut tags = vec![p.file.name.clone()];
    for l in &p.file.wiki_links {
      if !known.contains(l) {
        tags.push(l.clone()); // dangling -> tag fallback
      }
    }
    let entry = kumbarium_store::remember(
      &mut state.library,
      &kumbarium_store::NewEntry {
        namespace: p.namespace.clone(),
        kind: p.kind,
        content: format!("{}\n\n{}", p.file.description, p.file.body),
        agent_id: AGENT_ID.into(),
        source: source_of(&p.file.path),
        tags,
      },
    )
    .map_err(|e| format!("{}: {e}", p.file.name))?;
    id_by_name.insert(p.file.name.clone(), entry.id);
    imported += 1;
  }

  // Second pass: edges, now that every batch member has an id.
  let mut edges = 0usize;
  for p in &importable {
    let from = &id_by_name[&p.file.name];
    for l in &p.file.wiki_links {
      if let Some(to) = id_by_name.get(l)
        && from != to
      {
        kumbarium_store::link(
          &state.library,
          from,
          to,
          kumbarium_store::Rel::RelatesTo,
        )
        .map_err(|e| format!("linking {}: {e}", p.file.name))?;
        edges += 1;
      }
    }
  }

  let event = kumbarium_audit::Event {
    agent_id: AGENT_ID.into(),
    kind: kumbarium_audit::EventKind::Import,
    scope: "global".into(),
    detail: json!({
      "planned": plan.len(),
      "imported": imported,
      "edges": edges,
    }),
  };
  kumbarium_audit::append(&state.audit, &event)
    .map_err(|e| format!("audit append failed: {e}"))?;

  report.push(String::new());
  report.push(format!(
    "imported {imported} memories, {edges} relates_to edges \
     ({} skipped as already imported)",
    plan.len() - imported
  ));
  Ok(report)
}

fn collect(
  state: &ServerState,
  dir: &Path,
  opts: &Options,
  plan: &mut Vec<Planned>,
  report: &mut Vec<String>,
) -> Result<(), String> {
  let read = std::fs::read_dir(dir)
    .map_err(|e| format!("reading {}: {e}", dir.display()))?;
  let mut paths: Vec<PathBuf> = read
    .filter_map(|e| e.ok())
    .map(|e| e.path())
    .filter(|p| {
      p.extension().is_some_and(|x| x == "md")
        && p.file_name().is_some_and(|n| n != "MEMORY.md")
    })
    .collect();
  paths.sort();
  for path in paths {
    let raw = std::fs::read_to_string(&path)
      .map_err(|e| format!("reading {}: {e}", path.display()))?;
    let Some(file) = parse_memory(&path, &raw) else {
      report.push(format!(
        "warn: {} has no parseable frontmatter; skipped",
        path.display()
      ));
      continue;
    };
    let namespace = opts
      .map
      .iter()
      .find(|(name, _)| *name == file.name)
      .map(|(_, ns)| ns.clone())
      .unwrap_or_else(|| "global".to_string());
    if kumbarium_store::namespace_id(&state.library, &namespace)
      .map_err(|e| e.to_string())?
      .is_none()
    {
      return Err(format!(
        "namespace {namespace:?} (for {}) is not registered; \
         run: kumbarium namespace add {namespace}",
        file.name
      ));
    }
    let kind = match file.mtype.as_str() {
      "user" | "feedback" => kumbarium_store::Kind::Preference,
      "project" => kumbarium_store::Kind::ProjectState,
      "reference" => kumbarium_store::Kind::Reference,
      other => {
        report.push(format!(
          "warn: {} has unknown type {other:?}; storing as \
           reference",
          file.name
        ));
        kumbarium_store::Kind::Reference
      }
    };
    let already =
      kumbarium_store::find_by_source(&state.library, &source_of(&file.path))
        .map_err(|e| e.to_string())?
        .into_iter()
        .next();
    plan.push(Planned {
      file,
      namespace,
      kind,
      already_imported: already,
    });
  }
  Ok(())
}

fn source_of(path: &Path) -> String {
  format!("claude-memory:{}", path.display())
}

/// Parse the small frontmatter subset these files use (name,
/// description, metadata: type) plus the body and [[links]].
/// None when the frontmatter fence or name is missing.
fn parse_memory(path: &Path, raw: &str) -> Option<MemoryFile> {
  let rest = raw.strip_prefix("---")?;
  let end = rest.find("\n---")?;
  let fm = &rest[..end];
  let body = rest[end + 4..].trim().to_string();

  let mut name = None;
  let mut description = String::new();
  let mut mtype = String::new();
  let mut in_metadata = false;
  for line in fm.lines() {
    let indented = line.starts_with(' ') || line.starts_with('\t');
    let t = line.trim();
    if !indented {
      in_metadata = t == "metadata:" || t.starts_with("metadata:");
    }
    if let Some(v) = t.strip_prefix("name:") {
      name = Some(unquote(v));
    } else if let Some(v) = t.strip_prefix("description:") {
      description = unquote(v);
    } else if in_metadata
      && indented
      && let Some(v) = t.strip_prefix("type:")
    {
      mtype = unquote(v);
    }
  }
  Some(MemoryFile {
    path: path.to_path_buf(),
    name: name?,
    mtype,
    description,
    body: body.clone(),
    wiki_links: wiki_links(&body),
  })
}

fn unquote(v: &str) -> String {
  let v = v.trim();
  v.strip_prefix('"')
    .and_then(|s| s.strip_suffix('"'))
    .unwrap_or(v)
    .to_string()
}

/// Every `[[slug]]` in the text, deduplicated, order kept.
fn wiki_links(text: &str) -> Vec<String> {
  let mut out = Vec::new();
  let mut rest = text;
  while let Some(start) = rest.find("[[") {
    let after = &rest[start + 2..];
    let Some(end) = after.find("]]") else { break };
    let slug = after[..end].trim().to_string();
    if !slug.is_empty() && !slug.contains('\n') && !out.contains(&slug) {
      out.push(slug);
    }
    rest = &after[end + 2..];
  }
  out
}

#[cfg(test)]
mod tests {
  use super::*;

  const FIXTURE_A: &str = "---\n\
name: test-pref\n\
description: \"A synthetic preference\"\n\
metadata:\n  type: user\n---\n\n\
The user prefers tabs. See [[test-proj]] and [[missing-one]].\n";

  const FIXTURE_B: &str = "---\n\
name: test-proj\n\
description: A synthetic project note\n\
metadata:\n  type: project\n---\n\n\
The project uses SQLite.\n";

  fn fixture_dir() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("MEMORY.md"), "- index").unwrap();
    std::fs::write(dir.path().join("test-pref.md"), FIXTURE_A).unwrap();
    std::fs::write(dir.path().join("test-proj.md"), FIXTURE_B).unwrap();
    std::fs::write(dir.path().join("junk.md"), "no frontmatter").unwrap();
    dir
  }

  fn opts(dir: &Path, apply: bool) -> Options {
    Options {
      dirs: vec![dir.to_path_buf()],
      apply,
      map: vec![("test-proj".into(), "project/demo".into())],
    }
  }

  fn state_with_ns() -> ServerState {
    let state = ServerState::in_memory();
    kumbarium_store::register_namespace(&state.library, "project/demo", "test")
      .unwrap();
    state
  }

  #[test]
  fn dry_run_plans_without_writing() {
    let dir = fixture_dir();
    let mut state = state_with_ns();
    let report = run(&mut state, &opts(dir.path(), false)).unwrap();
    let text = report.join("\n");
    assert!(text.contains("dry run"));
    assert!(text.contains("test-pref"));
    assert!(text.contains("project/demo"));
    assert!(text.contains("no parseable frontmatter"));
    let n: i64 = state
      .library
      .query_row("SELECT count(*) FROM entries", [], |r| r.get(0))
      .unwrap();
    assert_eq!(n, 0, "dry run wrote nothing");
  }

  #[test]
  fn apply_imports_maps_and_links() {
    let dir = fixture_dir();
    let mut state = state_with_ns();
    let report = run(&mut state, &opts(dir.path(), true)).unwrap();
    let text = report.join("\n");
    assert!(text.contains("imported 2 memories, 1 relates_to edges"));
    // Kind + namespace mapping held.
    let (ns, kind): (String, String) = state
      .library
      .query_row(
        "SELECT n.path, e.kind FROM entries e
         JOIN namespaces n ON n.id = e.namespace_id
         WHERE e.content LIKE '%SQLite%'",
        [],
        |r| Ok((r.get(0)?, r.get(1)?)),
      )
      .unwrap();
    assert_eq!(ns, "project/demo");
    assert_eq!(kind, "project_state");
    // Resolved [[test-proj]] became an edge; dangling
    // [[missing-one]] became a tag.
    let edges: i64 = state
      .library
      .query_row(
        "SELECT count(*) FROM entry_links WHERE rel='relates_to'",
        [],
        |r| r.get(0),
      )
      .unwrap();
    assert_eq!(edges, 1);
    let tag: i64 = state
      .library
      .query_row(
        "SELECT count(*) FROM entry_tags WHERE tag='missing-one'",
        [],
        |r| r.get(0),
      )
      .unwrap();
    assert_eq!(tag, 1);
    // Audit trail recorded the run.
    let kind: String = state
      .audit
      .query_row(
        "SELECT kind FROM events ORDER BY at DESC LIMIT 1",
        [],
        |r| r.get(0),
      )
      .unwrap();
    assert_eq!(kind, "import");
  }

  #[test]
  fn reimport_skips_everything() {
    let dir = fixture_dir();
    let mut state = state_with_ns();
    run(&mut state, &opts(dir.path(), true)).unwrap();
    let report = run(&mut state, &opts(dir.path(), true)).unwrap();
    let text = report.join("\n");
    assert!(text.contains("imported 0 memories"));
    assert!(text.contains("SKIP"));
    let n: i64 = state
      .library
      .query_row("SELECT count(*) FROM entries", [], |r| r.get(0))
      .unwrap();
    assert_eq!(n, 2, "no duplicates on reimport");
  }

  #[test]
  fn unregistered_mapped_namespace_fails_loudly() {
    let dir = fixture_dir();
    let mut state = ServerState::in_memory(); // no project/demo
    let err = run(&mut state, &opts(dir.path(), false)).unwrap_err();
    assert!(err.contains("kumbarium namespace add project/demo"));
  }
}
