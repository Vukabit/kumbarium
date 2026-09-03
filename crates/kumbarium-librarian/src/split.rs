//! Write-path content splitting: oversized memories are stored
//! as parts chained with `continues` edges, and the LIBRARIAN
//! does the splitting so no writer (agent or importer) has to.
//! Deterministic and boundary-aware: parts break only at
//! paragraph boundaries (blank lines), preferring markdown
//! heading starts, and a paragraph is never cut internally. A
//! single paragraph larger than the target passes through whole
//! rather than mangled; a smarter semantic splitter is a future
//! Curator job, never this path's.

/// Target maximum part size in bytes. Callers on every write
/// path use this same constant, so agent writes and imports
/// split identically.
pub const SPLIT_TARGET: usize = 1500;

/// Sanitize a supersession note for storage/display: one line,
/// control characters stripped, at most 80 chars; None when
/// nothing survives. The note is a LABEL only: history collapse
/// is gated on the measured diff, never on the note.
pub fn sanitize_note(raw: &str) -> Option<String> {
  let cleaned: String = raw
    .chars()
    .filter(|c| !c.is_control())
    .collect::<String>()
    .trim()
    .chars()
    .take(80)
    .collect();
  if cleaned.is_empty() {
    None
  } else {
    Some(cleaned)
  }
}

/// Split `content` into storage parts of at most `max` bytes
/// each (except an indivisible oversized paragraph). Paragraphs
/// are packed greedily in order; a markdown heading starts a
/// new part early once the current part is half full. Blank-line
/// runs normalize to one blank line; otherwise text is
/// preserved verbatim and in order.
pub fn split_for_storage(content: &str, max: usize) -> Vec<String> {
  let blocks: Vec<&str> = content
    .split("\n\n")
    .map(|b| b.trim_matches('\n'))
    .filter(|b| !b.trim().is_empty())
    .collect();
  if blocks.is_empty() {
    return vec![content.trim().to_string()];
  }
  let mut parts = Vec::new();
  let mut current = String::new();
  for block in blocks {
    let would_be = if current.is_empty() {
      block.len()
    } else {
      current.len() + 2 + block.len()
    };
    let heading_break = block.starts_with('#') && current.len() > max / 2;
    if !current.is_empty() && (would_be > max || heading_break) {
      parts.push(std::mem::take(&mut current));
    }
    if !current.is_empty() {
      current.push_str("\n\n");
    }
    current.push_str(block);
  }
  if !current.is_empty() {
    parts.push(current);
  }
  parts
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn notes_sanitize_to_one_bounded_line() {
    assert_eq!(
      sanitize_note("  typo fix\n\x07 "),
      Some("typo fix".to_string())
    );
    assert_eq!(sanitize_note("\n\t  "), None);
    let long = "x".repeat(200);
    assert_eq!(sanitize_note(&long).unwrap().len(), 80);
  }

  #[test]
  fn small_content_stays_whole() {
    let parts = split_for_storage("one small fact", SPLIT_TARGET);
    assert_eq!(parts, ["one small fact"]);
  }

  #[test]
  fn splits_at_paragraph_boundaries_only() {
    let a = "a".repeat(60);
    let b = "b".repeat(60);
    let c = "c".repeat(60);
    let content = format!("{a}\n\n{b}\n\n{c}");
    let parts = split_for_storage(&content, 130);
    assert_eq!(parts.len(), 2);
    assert_eq!(parts[0], format!("{a}\n\n{b}"));
    assert_eq!(parts[1], c);
  }

  #[test]
  fn heading_starts_a_new_part_when_half_full() {
    let intro = "i".repeat(80);
    let content = format!("{intro}\n\n## section\n\nbody text");
    let parts = split_for_storage(&content, 140);
    // 80 > 140/2, so the heading breaks early even though the
    // heading block itself would still fit.
    assert_eq!(parts.len(), 2);
    assert!(parts[1].starts_with("## section"));
  }

  #[test]
  fn indivisible_oversized_paragraph_passes_whole() {
    let wall = "w".repeat(400);
    let parts = split_for_storage(&wall, 100);
    assert_eq!(parts, [wall]);
  }

  #[test]
  fn blank_line_runs_normalize_to_one() {
    let parts = split_for_storage("first\n\n\n\nsecond", SPLIT_TARGET);
    assert_eq!(parts, ["first\n\nsecond"]);
  }
}

#[cfg(test)]
mod prop_tests {
  use super::*;
  use proptest::prelude::*;

  fn paragraphs() -> impl Strategy<Value = Vec<String>> {
    // Always at least one letter: a whitespace-only "paragraph"
    // is not a block and is filtered by design.
    proptest::collection::vec("[a-z][a-z ]{0,79}", 1..12)
  }

  proptest! {
    // Nothing is lost or reordered: rejoining the parts equals
    // the blank-line-normalized original.
    #[test]
    fn parts_rejoin_to_normalized_original(paras in paragraphs()) {
      let content = paras.join("\n\n");
      let parts = split_for_storage(&content, 120);
      prop_assert_eq!(parts.join("\n\n"), content);
    }

    // Every part respects the cap unless it is a single
    // indivisible paragraph.
    #[test]
    fn parts_respect_the_cap(paras in paragraphs()) {
      let content = paras.join("\n\n");
      for part in split_for_storage(&content, 120) {
        prop_assert!(
          part.len() <= 120 || !part.contains("\n\n")
        );
      }
    }

    // Total for arbitrary input, and never returns empty parts.
    #[test]
    fn never_panics_or_yields_empty_parts(s in ".{0,400}") {
      for part in split_for_storage(&s, 100) {
        prop_assert!(!part.trim().is_empty() || s.trim().is_empty());
      }
    }
  }
}
