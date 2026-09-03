//! The loading dock (D-031): every exporter and importer, on
//! one shared flag contract and delivery spine.

use std::process::ExitCode;

use super::super::{bundle, import, open_stores, style};
use super::term::*;

/// Flags every exporter speaks identically (the export spine).
#[derive(Default)]
pub(crate) struct ExportOpts {
  out: Option<String>,
  stdout: bool,
  show: bool,
  open: bool,
  raw: bool,
}

pub(crate) fn parse_export_opts(rest: &[&str]) -> Result<ExportOpts, String> {
  let mut opts = ExportOpts::default();
  let mut it = rest.iter();
  while let Some(arg) = it.next() {
    match *arg {
      "--stdout" => opts.stdout = true,
      "--show" => opts.show = true,
      "--open" => opts.open = true,
      "--raw" => opts.raw = true,
      "--out" => match it.next() {
        Some(dir) => opts.out = Some((*dir).to_string()),
        None => return Err("--out needs a directory".into()),
      },
      other => {
        return Err(format!("unknown export flag {other:?}"));
      }
    }
  }
  if opts.stdout && (opts.show || opts.open) {
    return Err(
      "--stdout persists nothing; --show and --open need a file".into(),
    );
  }
  Ok(opts)
}

/// Persist one export artifact and finish the shared flags:
/// resolve the directory (--out or the artifact's shelf), write
/// atomically under the sortable stamped name, print the path,
/// then reveal and/or open on request.
pub(crate) fn deliver_export(
  shelf: std::path::PathBuf,
  name: String,
  content: &str,
  opts: &ExportOpts,
) -> ExitCode {
  let dir = match &opts.out {
    Some(raw) => expand_home(raw),
    None => shelf,
  };
  if let Err(e) = std::fs::create_dir_all(&dir) {
    return fail(&format!("creating {}: {e}", dir.display()));
  }
  let target = dir.join(name);
  if let Err(e) = kumbarium_util::write_atomically(&target, content.as_bytes())
  {
    return fail(&format!("writing export: {e}"));
  }
  println!("{}", shell_quote(&target.display().to_string()));
  if opts.show
    && let Err(e) = reveal(&target)
  {
    return fail(&e);
  }
  if opts.open
    && let Err(e) = open_in_editor(&target)
  {
    return fail(&e);
  }
  ExitCode::SUCCESS
}

pub(crate) fn export_minutes_cmd(rest: &[&str]) -> ExitCode {
  let opts = match parse_export_opts(rest) {
    Ok(o) => o,
    Err(e) => return fail(&e),
  };
  let (p, state) = match open_stores() {
    Ok(v) => v,
    Err(e) => return fail(&e),
  };
  let events = match kumbarium_audit::events_asc(&state.audit) {
    Ok(events) => events,
    Err(e) => return fail(&e.to_string()),
  };
  // --raw keeps the STORED form: UTC, machine-comparable
  // across exporting machines. Default is local time.
  let utc_display = |at: &str| -> String {
    let day = at.get(..10).unwrap_or(at);
    let time = at.get(11..19).unwrap_or("");
    format!("{day} {time}")
  };
  let minutes = if opts.raw {
    kumbarium_audit::render_minutes(
      &events,
      &utc_display,
      "All times UTC (as stored).",
    )
  } else {
    kumbarium_audit::render_minutes(
      &events,
      &local_display,
      "Times are local to the exporting machine.",
    )
  };
  if opts.stdout {
    // On a TTY, hanging-wrap table rows at the detail column
    // (8+2 + 9+1 + 20+1 + 20+1 = 62) so overflow stays
    // readable; piped/redirected output is byte-identical to
    // the file artifact.
    const EXPORT_DETAIL_COL: usize = 66;
    let width = term_width().filter(|w| *w > EXPORT_DETAIL_COL + 16);
    let Some(width) = width else {
      print!("{minutes}");
      return ExitCode::SUCCESS;
    };
    let mut in_fence = false;
    for line in minutes.lines() {
      if line.starts_with("```") {
        in_fence = !in_fence;
        println!("{line}");
        continue;
      }
      let wrappable = in_fence
        && line.len() > width
        && line.is_char_boundary(EXPORT_DETAIL_COL);
      if !wrappable {
        println!("{line}");
        continue;
      }
      let (prefix, rest) = line.split_at(EXPORT_DETAIL_COL);
      let chunks = wrap_words(rest, width - EXPORT_DETAIL_COL);
      println!(
        "{prefix}{}",
        chunks.first().map(String::as_str).unwrap_or("")
      );
      for chunk in chunks.iter().skip(1) {
        println!("{:EXPORT_DETAIL_COL$}{chunk}", "");
      }
    }
    return ExitCode::SUCCESS;
  }
  deliver_export(
    p.exports_dir.join("audit"),
    format!("minutes-{}Z.md", export_stamp()),
    &minutes,
    &opts,
  )
}

/// The sortable second-resolution stamp every export name uses.
pub(crate) fn export_stamp() -> String {
  kumbarium_util::now_iso8601()
    .get(..19)
    .unwrap_or_default()
    .replace(':', "-")
}

pub(crate) fn import_claude(rest: &[&str]) -> ExitCode {
  let mut opts = import::Options {
    dirs: Vec::new(),
    apply: false,
    map: Vec::new(),
  };
  let mut it = rest.iter();
  while let Some(arg) = it.next() {
    match *arg {
      "--apply" => opts.apply = true,
      "--dir" => match it.next() {
        Some(d) => opts.dirs.push(d.into()),
        None => return fail("--dir needs a path"),
      },
      "--map" => match it.next().and_then(|m| m.split_once('=')) {
        Some((name, ns)) => {
          opts.map.push((name.to_string(), ns.to_string()));
        }
        None => return fail("--map needs name=namespace"),
      },
      other => {
        return fail(&format!("unknown import flag {other:?}"));
      }
    }
  }
  if opts.dirs.is_empty() {
    opts.dirs = import::default_dirs();
  }
  if opts.dirs.is_empty() {
    return fail("no Claude memory dirs found; pass --dir <path>");
  }
  let (_, mut state) = match open_stores() {
    Ok(v) => v,
    Err(e) => return fail(&e),
  };
  match import::run(&mut state, &opts) {
    Ok(report) => {
      for line in report {
        println!("{line}");
      }
      ExitCode::SUCCESS
    }
    Err(e) => fail(&e),
  }
}

/// Export one shelf as a hashed bundle file (D-028), through
/// the shared export spine.
pub(crate) fn export_bundle_cmd(scope: &str, rest: &[&str]) -> ExitCode {
  let opts = match parse_export_opts(rest) {
    Ok(o) => o,
    Err(e) => return fail(&e),
  };
  if opts.raw {
    return fail("--raw applies to minutes only");
  }
  let scope = &kumbarium_librarian::normalize_namespace(scope);
  let (p, state) = match open_stores() {
    Ok(v) => v,
    Err(e) => return fail(&e),
  };
  let (text, count) = match bundle::export(&state, scope) {
    Ok(v) => v,
    Err(e) => return fail(&e),
  };
  if opts.stdout {
    print!("{text}");
    return ExitCode::SUCCESS;
  }
  eprintln!("bundled {count} entries from {scope}");
  deliver_export(
    p.exports_dir.join("bundles"),
    format!(
      "bundle-{}-{}Z.json",
      scope.replace('/', "-"),
      export_stamp()
    ),
    &text,
    &opts,
  )
}

/// Union-merge a bundle file (D-028); --pending routes every
/// imported chain head through the desk. Witnessed as an import
/// event carrying the bundle hash.
pub(crate) fn import_bundle_cmd(file: &str, as_pending: bool) -> ExitCode {
  let (_, mut state) = match open_stores() {
    Ok(v) => v,
    Err(e) => return fail(&e),
  };
  let sty = style::Style::detect();
  let text = match std::fs::read_to_string(file) {
    Ok(t) => t,
    Err(e) => return fail(&format!("reading {file}: {e}")),
  };
  let summary = match bundle::import(&mut state, &text, as_pending) {
    Ok(s) => s,
    Err(e) => return fail(&e),
  };
  let event = kumbarium_audit::Event {
    agent_id: "kumbarium-cli".into(),
    kind: kumbarium_audit::EventKind::Import,
    scope: summary.scope.clone(),
    detail: serde_json::json!({
      "planned": summary.planned,
      "imported": summary.imported,
      "edges": 0,
      "bundle_hash": summary.hash,
      "skipped": summary.skipped,
      "extended": summary.extended,
      "forks": summary.forks.len(),
      "pending": as_pending,
    }),
  };
  if let Err(e) = kumbarium_audit::append(&state.audit, &event) {
    return fail(&format!("imported, but audit append failed: {e}"));
  }
  println!(
    "bundle {}: {} imported, {} already present, {} chains \
     fast-forwarded",
    &summary.hash[..12],
    summary.imported,
    summary.skipped,
    summary.extended
  );
  for (rival, local) in &summary.forks {
    println!(
      "{} fork: rival {} sent to the desk (contradicts live {}); \
       judge via kum review {}",
      sty.yellow("!"),
      sty.id(kumbarium_store::short_id(rival)),
      sty.id(kumbarium_store::short_id(local)),
      kumbarium_store::short_id(rival)
    );
  }
  if as_pending && summary.imported > 0 {
    println!("imported heads are pending: kum inbox to review");
  }
  ExitCode::SUCCESS
}
