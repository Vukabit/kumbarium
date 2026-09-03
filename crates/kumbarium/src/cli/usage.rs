//! The usage page and the loading-dock listing, plus the
//! painter that colors them on a terminal (piped output stays
//! byte-identical: Style no-ops).

use super::super::style;

pub(crate) const EXPORTS: &str = "\
the loading dock: everything leaving the library goes here
(imports enter through `kumbarium import`; minutes have no
import, the ledger admits events only by witnessing them)

  kumbarium export minutes [--raw]    audit minutes markdown
  kumbarium export bundle <ns>        a shelf, hashed JSON

shared flags:
  --out DIR    export into DIR (default: the exports/ shelf)
  --stdout     stream instead; nothing persisted
  --show       reveal the file in the OS file explorer
  --open       open the file in $VISUAL / $EDITOR";

/// Paint a usage-shaped page for a terminal: section headers
/// bold, subcommands cyan, <placeholders> yellow, [flags] dim,
/// prose dim, descriptions untouched. Zero-width when piped
/// (Style no-ops), so alignment and byte-identity both hold.
pub(crate) fn paint_cli_page(text: &str, sty: &style::Style) -> String {
  text
    .lines()
    .map(|line| paint_cli_line(line, sty))
    .collect::<Vec<_>>()
    .join("\n")
}

pub(crate) fn paint_cli_line(line: &str, sty: &style::Style) -> String {
  if line.is_empty() {
    return String::new();
  }
  if !line.starts_with(' ') {
    if let Some(rest) = line.strip_prefix("kumbarium:") {
      return format!("{}{}", sty.bold("kumbarium:"), sty.dim(rest));
    }
    return if line.ends_with(':') {
      sty.bold(line)
    } else {
      sty.dim(line)
    };
  }
  // Indented row: the invocation field ends at the first run of
  // two spaces after it starts; everything beyond is prose and
  // stays unpainted. Deeply indented lines that do not begin
  // with a flag or placeholder are description continuations,
  // prose through and through.
  let indent_len = line.len() - line.trim_start().len();
  let (indent, body) = line.split_at(indent_len);
  if indent_len > 6 && !body.starts_with(['[', '<']) {
    return line.to_string();
  }
  let gap = body
    .as_bytes()
    .windows(2)
    .position(|w| w == b"  ")
    .unwrap_or(body.len());
  let (inv, desc) = body.split_at(gap);
  format!("{indent}{}{desc}", paint_invocation(inv, sty))
}

pub(crate) fn paint_invocation(inv: &str, sty: &style::Style) -> String {
  let mut out = String::new();
  let mut rest = inv;
  while !rest.is_empty() {
    if let Some(r) = rest.strip_prefix(' ') {
      out.push(' ');
      rest = r;
      continue;
    }
    if rest.starts_with('(') {
      // Prose continuation; nothing command-like follows.
      out.push_str(rest);
      break;
    }
    if rest.starts_with('[') {
      let mut end = rest.find(']').map(|i| i + 1).unwrap_or(rest.len());
      while rest[end..].starts_with('.') {
        end += 1;
      }
      out.push_str(&sty.dim(&rest[..end]));
      rest = &rest[end..];
      continue;
    }
    if rest.starts_with('<') {
      let end = rest.find('>').map(|i| i + 1).unwrap_or(rest.len());
      out.push_str(&sty.yellow(&rest[..end]));
      rest = &rest[end..];
      continue;
    }
    let end = rest.find(' ').unwrap_or(rest.len());
    let word = &rest[..end];
    let painted = if word == "kumbarium" || word.ends_with(':') {
      sty.dim(word)
    } else if word.chars().all(|c| c.is_ascii_uppercase()) {
      sty.yellow(word)
    } else {
      sty.cyan(word)
    };
    out.push_str(&painted);
    rest = &rest[end..];
  }
  out
}

pub(crate) const USAGE: &str = "\
kumbarium: the place of remembering
kum is the short alias; every command below answers to both.

Usage:

wire agents up:
  kumbarium serve                     speak MCP over stdio
  kumbarium instructions [--snippet]  agent setup: MCP
                                      registration + root-file
                                      instruction block

the collection:
  kumbarium list [ns] [--all]         browse entries
  kumbarium show <id> [--full]        one entry (--full stitches
                                      a split set in order)
  kumbarium grep <pat> [ns] [--all]   literal search, rg-style
  kumbarium history <id> [--diff]     a fact's version chain
                     [--all]          (--all expands collapsed
                                      noted-small versions)
  kumbarium confirm <id>              record a fact proved true
  kumbarium move <id> <namespace>     relocate (as supersession)

lifecycle, human sign-off:
  kumbarium retire <id>               hide from suggestions
  kumbarium unretire <id>             restore to suggestions
  kumbarium revert <id> [--apply]     restore an old version
                                      (preview only until the
                                      --apply sign-off; CLI
                                      only, agents cannot)
  kumbarium janitor [--apply]         confidence pass over the
                                      ledger (preview until the
                                      --apply sign-off; CLI
                                      only, agents cannot)

the docket:
  kumbarium task <ns> <matter...>     file a matter
       [--severity S] [--goal DATE]   (severity low|normal|
                                      high|urgent; goal is a
                                      watched YYYY-MM-DD)
  kumbarium tasks [ns] [--all]        the timeline: open
                  [--severity S]      matters, creep marked
  kumbarium roadmap [ns]              the same matters pivoted
                                      by goal horizon
  kumbarium task done <id> [note]     record a matter complete
  kumbarium task drop <id> [note]     overtaken by events
  kumbarium task grade <id>           re-judge severity or goal
       [--severity S] [--goal DATE]   (the old version chains)
  kumbarium task history <id>         a matter's chain: every
                                      regrade and goal slip

the circulation desk:
  kumbarium inbox                     pending entries awaiting
                                      approval (the desk queue)
  kumbarium review <id>               a pending entry in full:
                                      content, provenance, and
                                      the collision surface
  kumbarium approve <id>              promote to circulation
  kumbarium reject <id> [reason]      decline, kept on record

the loading dock:
  kumbarium export                    list the loading dock
  kumbarium export minutes [--raw]    audit minutes markdown
  kumbarium export bundle <ns>        a shelf, hashed JSON
      shared: [--out DIR] [--stdout] [--show] [--open]
  kumbarium import bundle <FILE>      union-merge a bundle
                          [--pending] (forks go to the desk;
                                      --pending queues all)
  kumbarium import claude [--apply]   import Claude Code
      [--dir <path>]... [--map name=namespace]...  memories

the witness:
  kumbarium audit tail [n]            recent audit events
             [--scope <ns>]           (optionally one scope)
  kumbarium audit verify              recompute the ledger's
                                      hash chain; tampering
                                      names its first break

upkeep:
  kumbarium namespace add <path> [d]  register a namespace
  kumbarium namespace list            list namespaces
  kumbarium status                    library health at a glance
  kumbarium backup                    snapshot both dbs now
  kumbarium config [--init|--open]    effective tunables
                                      (--init writes template,
                                      --open edits it)
  kumbarium paths                     where persisted data lives

meta:
  kumbarium version                   print the version
  kumbarium help [topic]              manual pages with grammar
                                      and examples";
