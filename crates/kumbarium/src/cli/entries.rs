//! Commands over the collection: browse, inspect, search, and
//! the human lifecycle verbs (retire, revert, move, confirm).

use std::process::ExitCode;

use super::super::{diff, markdown, open_stores, style, tools};
use super::term::*;

pub(crate) fn list_entries(namespace: Option<&str>, all: bool) -> ExitCode {
  let (_, state) = match open_stores() {
    Ok(v) => v,
    Err(e) => return fail(&e),
  };
  let entries =
    match kumbarium_store::entries_in(&state.library, namespace, all) {
      Ok(entries) => entries,
      Err(e) => return fail(&e.to_string()),
    };
  if entries.is_empty() {
    println!("no entries");
    return ExitCode::SUCCESS;
  }
  let sty = style::Style::detect();
  println!(
    "{}",
    sty.dim(
      "id        created     kind          namespace            \
       content"
    )
  );
  // Set-aware order: entries arrive newest-first; the first time
  // a continues-set appears, all its visible members are emitted
  // together in chain order, so a group sits at its newest
  // member's position and parts always read 1..n.
  let first_line = |content: &str| -> String {
    content
      .lines()
      .next()
      .unwrap_or("")
      .chars()
      .take(48)
      .collect()
  };
  let by_id: std::collections::HashMap<&str, &kumbarium_store::Entry> =
    entries.iter().map(|e| (e.id.as_str(), e)).collect();
  let mut emitted: std::collections::HashSet<&str> =
    std::collections::HashSet::new();
  for e in &entries {
    if emitted.contains(e.id.as_str()) {
      continue;
    }
    let chain = kumbarium_store::continues_chain(&state.library, &e.id)
      .map(|(chain, _)| chain)
      .unwrap_or_else(|_| vec![e.id.clone()]);
    let n = chain.len();
    let set_title = if n > 1 {
      kumbarium_store::get(&state.library, &chain[0])
        .map(|head| first_line(&head.content))
        .ok()
    } else {
      None
    };
    for (i, id) in chain.iter().enumerate() {
      let Some(m) = by_id.get(id.as_str()) else {
        continue;
      };
      emitted.insert(m.id.as_str());
      let local = local_display(&m.created_at);
      let day = local.get(..10).unwrap_or(&local);
      let dead = if m.superseded_by.is_some() {
        sty.red(" [superseded]")
      } else if m.retired_at.is_some() {
        sty.yellow(" [retired]")
      } else {
        String::new()
      };
      let part = if n > 1 {
        sty.dim(&format!(" ({}/{n})", i + 1))
      } else {
        String::new()
      };
      let title = set_title.clone().unwrap_or_else(|| first_line(&m.content));
      println!(
        "{}  {day}  {} {:<20} {title}{part}{dead}",
        sty.id(kumbarium_store::short_id(&m.id)),
        sty.kind(&format!("{:<13}", m.kind.as_str())),
        m.namespace
      );
    }
  }
  println!(
    "{}",
    sty.dim(&format!(
      "({} entries; ids are short forms, any unique fragment \
       works)",
      entries.len()
    ))
  );
  ExitCode::SUCCESS
}

pub(crate) fn show_entry(id: &str, full: bool) -> ExitCode {
  let (_, mut state) = match open_stores() {
    Ok(v) => v,
    Err(e) => return fail(&e),
  };
  let full_id = match kumbarium_store::resolve_id(&state.library, id) {
    Ok(full_id) => full_id,
    Err(kumbarium_store::StoreError::EntryNotFound(_)) => {
      // Ids are building-wide names: fall through to the docket.
      return match super::docket::show_task(&mut state, id) {
        Ok(code) => code,
        Err(e) => fail(&e),
      };
    }
    Err(err) => return fail(&err.to_string()),
  };
  let e = match kumbarium_store::get(&state.library, &full_id) {
    Ok(e) => e,
    Err(err) => return fail(&err.to_string()),
  };
  let sty = style::Style::detect();
  println!(
    "id:         {} (short: {})",
    e.id,
    sty.id(kumbarium_store::short_id(&e.id))
  );
  println!("namespace:  {}", e.namespace);
  println!("kind:       {}", sty.kind(e.kind.as_str()));
  println!("agent:      {}", e.agent_id);
  if !e.source.is_empty() {
    println!("source:     {}", e.source);
  }
  if e.status != kumbarium_store::Status::Live {
    println!("status:     {}", sty.yellow(e.status.as_str()));
  }
  match &e.confidence_basis {
    Some(basis) => {
      println!("confidence: {:.2} ({basis})", e.confidence)
    }
    None => println!("confidence: {:.2}", e.confidence),
  }
  println!("created:    {}", local_display(&e.created_at));
  println!("updated:    {}", local_display(&e.updated_at));
  if let Some(at) = &e.last_accessed_at {
    println!("accessed:   {}", local_display(at));
  }
  if let Some(at) = &e.last_confirmed_at {
    println!("confirmed:  {}", local_display(at));
  }
  if let Some(at) = &e.retired_at {
    println!("retired:    {}", sty.yellow(&local_display(at)));
  }
  if let Some(note) = &e.note {
    println!("note:       {}", sty.dim(note));
  }
  if !e.tags.is_empty() {
    println!("tags:       {}", e.tags.join(", "));
  }
  if let Some(new) = &e.superseded_by {
    println!("superseded by: {}", sty.yellow(new));
  }
  match kumbarium_store::predecessor_of(&state.library, &full_id) {
    Ok(Some(old)) => println!("supersedes: {old}"),
    Ok(None) => {}
    Err(err) => return fail(&err.to_string()),
  }
  match kumbarium_store::links_of(&state.library, &full_id) {
    Ok(links) => {
      for l in links {
        if l.from_id == e.id {
          println!("link:       {} -> {}", l.rel.as_str(), sty.id(&l.to_id));
        } else {
          println!("link:       {} <- {}", l.rel.as_str(), sty.id(&l.from_id));
        }
      }
    }
    Err(err) => return fail(&err.to_string()),
  }
  let (chain, branched) =
    match kumbarium_store::continues_chain(&state.library, &full_id) {
      Ok(v) => v,
      Err(err) => return fail(&err.to_string()),
    };
  let n = chain.len();
  if n > 1 && !full {
    let pos = chain.iter().position(|c| *c == full_id).unwrap_or(0) + 1;
    let note = if branched { "; chain BRANCHES" } else { "" };
    println!(
      "set:        {}",
      sty.yellow(&format!("part {pos} of {n}{note} (--full stitches)"))
    );
  }
  if full && n > 1 {
    if branched {
      println!(
        "{}",
        sty.yellow("warning: continues chain branches; showing mint order")
      );
    }
    for (i, part_id) in chain.iter().enumerate() {
      let part = match kumbarium_store::get(&state.library, part_id) {
        Ok(part) => part,
        Err(err) => return fail(&err.to_string()),
      };
      println!(
        "\n{}\n{}",
        sty.bold(&format!(
          "-- part {}/{n}  {} --",
          i + 1,
          kumbarium_store::short_id(part_id)
        )),
        markdown::render(&part.content, &sty)
      );
    }
  } else {
    println!("\n{}", markdown::render(&e.content, &sty));
  }
  ExitCode::SUCCESS
}

/// Retire (or restore) an entry: human-only lifecycle verb,
/// immediate because fully reversible; audited either way.
pub(crate) fn retire_cmd(id: &str, retiring: bool) -> ExitCode {
  let (_, state) = match open_stores() {
    Ok(v) => v,
    Err(e) => return fail(&e),
  };
  let sty = style::Style::detect();
  let full = match kumbarium_store::resolve_id(&state.library, id) {
    Ok(f) => f,
    Err(e) => return fail(&e.to_string()),
  };
  let result = if retiring {
    kumbarium_store::retire(&state.library, &full)
  } else {
    kumbarium_store::unretire(&state.library, &full)
  };
  if let Err(e) = result {
    return fail(&e.to_string());
  }
  let entry = match kumbarium_store::get(&state.library, &full) {
    Ok(e) => e,
    Err(e) => return fail(&e.to_string()),
  };
  let kind = if retiring {
    kumbarium_audit::EventKind::Retire
  } else {
    kumbarium_audit::EventKind::Unretire
  };
  let event = kumbarium_audit::Event {
    agent_id: "kumbarium-cli".into(),
    kind,
    scope: entry.namespace.clone(),
    detail: serde_json::json!({ "id": full }),
  };
  if let Err(e) = kumbarium_audit::append(&state.audit, &event) {
    return fail(&format!("done, but audit append failed: {e}"));
  }
  let short = kumbarium_store::short_id(&full);
  if retiring {
    println!(
      "retired {} (kept in history; `kum unretire {short}` \
       restores)",
      sty.id(short)
    );
  } else {
    println!("restored {} to suggestions", sty.id(short));
  }
  ExitCode::SUCCESS
}

pub(crate) fn history_cmd(id: &str, with_diff: bool, all: bool) -> ExitCode {
  let (_, state) = match open_stores() {
    Ok(v) => v,
    Err(e) => return fail(&e),
  };
  let sty = style::Style::detect();
  if kumbarium_store::resolve_id(&state.library, id).is_err() {
    // Ids are building-wide names: a task chain renders through
    // the docket's own history view.
    return super::docket::task_history_cmd(id);
  }
  let versions = match resolve_history(&state, id) {
    Ok(v) => v,
    Err(e) => return fail(&e),
  };
  let n = versions.len();
  // Collapse-eligible: noted AND measurably small vs its
  // predecessor. The diff decides; the note only informs.
  let changed: Vec<usize> = versions
    .iter()
    .enumerate()
    .map(|(i, e)| {
      if i == 0 {
        usize::MAX
      } else {
        diff::lines(&versions[i - 1].content, &e.content)
          .iter()
          .filter(|(c, _)| *c != ' ')
          .count()
      }
    })
    .collect();
  let collapsed = |i: usize| -> bool {
    // The live head never collapses: the current truth is
    // always shown in full.
    !all
      && i + 1 != n
      && versions[i].note.is_some()
      // The note informs, the diff decides (config:
      // history.collapse_max_changed_lines).
      && changed[i] <= state.cfg.collapse_max_changed_lines
  };
  println!(
    "{}",
    sty.dim(
      "version    id        created     agent                 \
       bytes"
    )
  );
  let mut hidden = 0usize;
  for (i, e) in versions.iter().enumerate().rev() {
    if collapsed(i) {
      hidden += 1;
      println!(
        "{}",
        sty.dim(&format!(
          "v{:<2}        {}  {:?} ({} lines changed)",
          i + 1,
          kumbarium_store::short_id(&e.id),
          e.note.as_deref().unwrap_or(""),
          changed[i]
        ))
      );
      continue;
    }
    let live = if i + 1 == n { " (live)" } else { "" };
    let ver = format!("v{}{live}", i + 1);
    let local = local_display(&e.created_at);
    let day = local.get(..10).unwrap_or(&local);
    let note = match &e.note {
      Some(note) => sty.dim(&format!("  {note:?}")),
      None => String::new(),
    };
    println!(
      "{ver:<11}{}  {day}  {:<20}  {}{note}",
      sty.id(kumbarium_store::short_id(&e.id)),
      e.agent_id,
      e.content.len()
    );
  }
  if hidden > 0 {
    println!(
      "{}",
      sty.dim(&format!(
        "({hidden} noted small version(s) collapsed; --all \
         expands)"
      ))
    );
  }
  if with_diff {
    for (pair_i, pair) in versions.windows(2).enumerate() {
      if collapsed(pair_i + 1) {
        continue;
      }
      println!(
        "\n{}",
        sty.bold(&format!(
          "-- v{} -> v{} --",
          version_of(&versions, &pair[0].id),
          version_of(&versions, &pair[1].id)
        ))
      );
      print_diff(&pair[0].content, &pair[1].content, &sty);
    }
  }
  ExitCode::SUCCESS
}

pub(crate) fn revert_cmd(id: &str, apply: bool) -> ExitCode {
  let (_, mut state) = match open_stores() {
    Ok(v) => v,
    Err(e) => return fail(&e),
  };
  let sty = style::Style::detect();
  let versions = match resolve_history(&state, id) {
    Ok(v) => v,
    Err(e) => return fail(&e),
  };
  let target_full = match kumbarium_store::resolve_id(&state.library, id) {
    Ok(f) => f,
    Err(e) => return fail(&e.to_string()),
  };
  let head = versions.last().expect("history never empty").clone();
  let Some(target) = versions.iter().find(|e| e.id == target_full).cloned()
  else {
    return fail("target version not found in history");
  };
  if target.id == head.id {
    return fail(&format!(
      "{} is already the live version; pick an ancestor \
       (see: kumbarium history {})",
      kumbarium_store::short_id(&target.id),
      kumbarium_store::short_id(&target.id)
    ));
  }
  println!(
    "revert plan: supersede live {} with the content of {} \
     (v{} of {})",
    sty.id(kumbarium_store::short_id(&head.id)),
    sty.id(kumbarium_store::short_id(&target.id)),
    version_of(&versions, &target.id),
    versions.len()
  );
  print_diff(&head.content, &target.content, &sty);
  if !apply {
    println!(
      "\n{}",
      sty.yellow(
        "preview only: nothing written; re-run with --apply \
         to sign off"
      )
    );
    return ExitCode::SUCCESS;
  }
  let new = kumbarium_store::NewEntry {
    namespace: target.namespace.clone(),
    kind: target.kind,
    content: target.content.clone(),
    agent_id: "kumbarium-cli".into(),
    source: target.source.clone(),
    tags: target.tags.clone(),
    status: kumbarium_store::Status::Live,
  };
  let revert_note =
    format!("revert to {}", kumbarium_store::short_id(&target.id));
  let ids = match tools::store_split(
    &mut state,
    &new,
    Some(&head.id),
    Some(&revert_note),
  ) {
    Ok(ids) => ids,
    Err(e) => return fail(&e),
  };
  let event = kumbarium_audit::Event {
    agent_id: "kumbarium-cli".into(),
    kind: kumbarium_audit::EventKind::Supersede,
    scope: target.namespace.clone(),
    detail: serde_json::json!({
      "old_id": head.id,
      "new_id": ids[0],
      "revert_to": target.id,
      "parts": ids.len(),
      "note": revert_note,
    }),
  };
  if let Err(e) = kumbarium_audit::append(&state.audit, &event) {
    return fail(&format!("reverted, but audit append failed: {e}"));
  }
  println!(
    "\nreverted: {} superseded by {} ({} part(s))",
    sty.id(kumbarium_store::short_id(&head.id)),
    sty.id(kumbarium_store::short_id(&ids[0])),
    ids.len()
  );
  ExitCode::SUCCESS
}

/// Full entries for a fact's version chain, oldest first.
pub(crate) fn resolve_history(
  state: &tools::ServerState,
  id: &str,
) -> Result<Vec<kumbarium_store::Entry>, String> {
  let full = kumbarium_store::resolve_id(&state.library, id)
    .map_err(|e| e.to_string())?;
  let ids = kumbarium_store::version_history(&state.library, &full)
    .map_err(|e| e.to_string())?;
  ids
    .iter()
    .map(|v| kumbarium_store::get(&state.library, v).map_err(|e| e.to_string()))
    .collect()
}

pub(crate) fn version_of(
  versions: &[kumbarium_store::Entry],
  id: &str,
) -> usize {
  versions.iter().position(|e| e.id == id).unwrap_or(0) + 1
}

pub(crate) fn print_diff(old: &str, new: &str, sty: &style::Style) {
  for (mark, line) in diff::lines(old, new) {
    match mark {
      '-' => println!("{}", sty.red(&format!("- {line}"))),
      '+' => println!("{}", sty.green(&format!("+ {line}"))),
      _ => println!("  {}", sty.dim(&line)),
    }
  }
}

/// rg-flavored literal search: smart-case, exhaustive (--all
/// includes superseded/retired), grouped headings on a tty and
/// `id:line:text` when piped. Deliberately NOT recall: recall
/// ranks live memories for agents; grep finds every occurrence
/// for forensics.
pub(crate) fn grep_cmd(
  pattern: &str,
  namespace: Option<&str>,
  all: bool,
) -> ExitCode {
  let (_, state) = match open_stores() {
    Ok(v) => v,
    Err(e) => return fail(&e),
  };
  let sty = style::Style::detect();
  let entries =
    match kumbarium_store::entries_in(&state.library, namespace, all) {
      Ok(entries) => entries,
      Err(e) => return fail(&e.to_string()),
    };
  // Smart-case, rg-style: all-lowercase pattern matches
  // case-insensitively; any uppercase makes it exact.
  let sensitive = pattern.chars().any(|c| c.is_uppercase());
  let needle = if sensitive {
    pattern.to_string()
  } else {
    pattern.to_lowercase()
  };
  let mut hits = 0usize;
  for e in &entries {
    let mut first = true;
    for (lineno, line) in e.content.lines().enumerate() {
      let hay = if sensitive {
        line.to_string()
      } else {
        line.to_lowercase()
      };
      if !hay.contains(&needle) {
        continue;
      }
      hits += 1;
      if sty.on {
        if first {
          first = false;
          let mark = if e.superseded_by.is_some() {
            " [superseded]"
          } else if e.retired_at.is_some() {
            " [retired]"
          } else {
            ""
          };
          println!(
            "{}  {}{}",
            sty.id(kumbarium_store::short_id(&e.id)),
            e.namespace,
            sty.yellow(mark)
          );
        }
        println!(
          "{}: {}",
          sty.dim(&format!("{:>4}", lineno + 1)),
          highlight(line, &needle, sensitive, &sty)
        );
      } else {
        println!("{}:{}:{line}", kumbarium_store::short_id(&e.id), lineno + 1);
      }
    }
    if !first && sty.on {
      println!();
    }
  }
  if hits == 0 {
    eprintln!("no matches");
    return ExitCode::FAILURE;
  }
  ExitCode::SUCCESS
}

/// Paint every occurrence of the needle in a line, rg-style.
pub(crate) fn highlight(
  line: &str,
  needle: &str,
  sensitive: bool,
  sty: &style::Style,
) -> String {
  let hay = if sensitive {
    line.to_string()
  } else {
    line.to_lowercase()
  };
  let mut out = String::new();
  let mut pos = 0;
  while let Some(found) = hay[pos..].find(needle) {
    let start = pos + found;
    let end = start + needle.len();
    if !line.is_char_boundary(start) || !line.is_char_boundary(end) {
      break;
    }
    out.push_str(&line[pos..start]);
    out.push_str(&sty.bold(&sty.red(&line[start..end])));
    pos = end;
  }
  out.push_str(&line[pos..]);
  out
}

/// Move a memory to another namespace: a supersession into the
/// target with an auto-note, so history records the move rather
/// than anything mutating in place.
pub(crate) fn move_cmd(id: &str, namespace: &str) -> ExitCode {
  let namespace = &kumbarium_librarian::normalize_namespace(namespace);
  if let Err(e) = kumbarium_librarian::validate_namespace(namespace) {
    return fail(&format!("invalid namespace: {e}"));
  }
  let (_, mut state) = match open_stores() {
    Ok(v) => v,
    Err(e) => return fail(&e),
  };
  if kumbarium_store::resolve_id(&state.library, id).is_err() {
    // Ids are building-wide names: relocate a task the same
    // way, a supersession into the new shelf with the move
    // noted (D-034).
    return super::docket::move_task_cmd(&mut state, id, namespace);
  }
  let sty = style::Style::detect();
  let full = match kumbarium_store::resolve_id(&state.library, id) {
    Ok(f) => f,
    Err(e) => return fail(&e.to_string()),
  };
  let e = match kumbarium_store::get(&state.library, &full) {
    Ok(e) => e,
    Err(err) => return fail(&err.to_string()),
  };
  if &e.namespace == namespace {
    return fail("entry is already in that namespace");
  }
  let note = format!("moved from {}", e.namespace);
  let new = kumbarium_store::NewEntry {
    namespace: namespace.to_string(),
    kind: e.kind,
    content: e.content.clone(),
    agent_id: "kumbarium-cli".into(),
    source: e.source.clone(),
    tags: e.tags.clone(),
    status: kumbarium_store::Status::Live,
  };
  let ids = match tools::store_split(&mut state, &new, Some(&full), Some(&note))
  {
    Ok(ids) => ids,
    Err(err) => return fail(&err),
  };
  let event = kumbarium_audit::Event {
    agent_id: "kumbarium-cli".into(),
    kind: kumbarium_audit::EventKind::Supersede,
    scope: namespace.to_string(),
    detail: serde_json::json!({
      "old_id": full,
      "new_id": ids[0],
      "note": note,
    }),
  };
  if let Err(err) = kumbarium_audit::append(&state.audit, &event) {
    return fail(&format!("moved, but audit append failed: {err}"));
  }
  println!(
    "moved {} -> {} as {}",
    sty.id(kumbarium_store::short_id(&full)),
    namespace,
    sty.id(kumbarium_store::short_id(&ids[0]))
  );
  ExitCode::SUCCESS
}

/// Record confirmation evidence from the CLI (same semantics
/// as the MCP tool: stamps last_confirmed_at, never touches the
/// confidence number; the janitor judges that later).
pub(crate) fn confirm_cmd(id: &str) -> ExitCode {
  let (_, state) = match open_stores() {
    Ok(v) => v,
    Err(e) => return fail(&e),
  };
  let sty = style::Style::detect();
  let full = match kumbarium_store::resolve_id(&state.library, id) {
    Ok(f) => f,
    Err(e) => return fail(&e.to_string()),
  };
  if let Err(e) = kumbarium_store::confirm(&state.library, &full) {
    return fail(&e.to_string());
  }
  let scope = kumbarium_store::get(&state.library, &full)
    .map(|e| e.namespace)
    .unwrap_or_default();
  let event = kumbarium_audit::Event {
    agent_id: "kumbarium-cli".into(),
    kind: kumbarium_audit::EventKind::Confirm,
    scope,
    detail: serde_json::json!({ "id": full }),
  };
  if let Err(e) = kumbarium_audit::append(&state.audit, &event) {
    return fail(&format!("confirmed, but audit append failed: {e}"));
  }
  println!("confirmed {}", sty.id(kumbarium_store::short_id(&full)));
  ExitCode::SUCCESS
}
