//! Namespace paths and chain resolution.
//!
//! Namespaces are registered slash-paths, max three segments,
//! e.g. `global`, `project/web.app`,
//! `agent/gemini/quarantine`. A query scoped to a namespace
//! searches its CHAIN: the namespace itself, each ancestor, and
//! `global`; never a sibling. That chain rule is the
//! cross-contamination firewall between projects.

/// Maximum path depth. Deep taxonomies are where personal tools
/// go to die.
pub const MAX_DEPTH: usize = 3;

const MAX_SEGMENT_LEN: usize = 64;

#[derive(Debug, PartialEq, Eq, thiserror::Error)]
pub enum NamespaceError {
  #[error("namespace is empty")]
  Empty,
  #[error("empty segment (leading, trailing, or doubled '/')")]
  EmptySegment,
  #[error("more than {MAX_DEPTH} segments")]
  TooDeep,
  #[error("segment exceeds {MAX_SEGMENT_LEN} chars")]
  SegmentTooLong,
  #[error("invalid character in segment {0:?}")]
  BadCharacter(String),
}

/// Normalize a namespace before validation: trim whitespace and
/// lowercase. Lossless by construction: the grammar admits only
/// lowercase, so no two registered paths can differ by case and
/// folding can never merge distinct namespaces. Guards agents
/// (and humans) who send `Project/Foo` from a not-found that the
/// grammar itself created. Every surface that accepts a
/// namespace normalizes first, registration included.
pub fn normalize_namespace(path: &str) -> String {
  path.trim().to_ascii_lowercase()
}

/// Validate a namespace path: 1..=MAX_DEPTH non-empty segments of
/// `[a-z0-9._-]`, each at most 64 chars. TRUST BOUNDARY: call on
/// any scope arriving from an agent before it reaches SQL or the
/// registry.
pub fn validate_namespace(path: &str) -> Result<(), NamespaceError> {
  if path.is_empty() {
    return Err(NamespaceError::Empty);
  }
  let segments: Vec<&str> = path.split('/').collect();
  if segments.len() > MAX_DEPTH {
    return Err(NamespaceError::TooDeep);
  }
  for segment in segments {
    if segment.is_empty() {
      return Err(NamespaceError::EmptySegment);
    }
    if segment.len() > MAX_SEGMENT_LEN {
      return Err(NamespaceError::SegmentTooLong);
    }
    let ok = segment.bytes().all(|b| {
      b.is_ascii_lowercase()
        || b.is_ascii_digit()
        || b == b'.'
        || b == b'_'
        || b == b'-'
    });
    if !ok {
      return Err(NamespaceError::BadCharacter(segment.to_string()));
    }
  }
  Ok(())
}

/// The search chain for a scope: the scope itself, each ancestor
/// prefix, then `global` (once). `global` itself yields
/// `["global"]`. Errors propagate from validation; a caller never
/// builds a chain from an unvalidated scope.
pub fn namespace_chain(scope: &str) -> Result<Vec<String>, NamespaceError> {
  validate_namespace(scope)?;
  let mut chain = Vec::new();
  let segments: Vec<&str> = scope.split('/').collect();
  for depth in (1..=segments.len()).rev() {
    chain.push(segments[..depth].join("/"));
  }
  if chain.last().map(String::as_str) != Some("global") {
    chain.push("global".to_string());
  }
  Ok(chain)
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn global_chain_is_just_global() {
    assert_eq!(namespace_chain("global").unwrap(), ["global"]);
  }

  #[test]
  fn project_chain_walks_up_then_global() {
    assert_eq!(
      namespace_chain("project/web.app").unwrap(),
      ["project/web.app", "project", "global"]
    );
  }

  #[test]
  fn three_deep_chain_includes_every_ancestor() {
    assert_eq!(
      namespace_chain("agent/gemini/quarantine").unwrap(),
      ["agent/gemini/quarantine", "agent/gemini", "agent", "global"]
    );
  }

  #[test]
  fn validation_rejection_table() {
    let cases: &[(&str, NamespaceError)] = &[
      ("", NamespaceError::Empty),
      ("/project", NamespaceError::EmptySegment),
      ("project/", NamespaceError::EmptySegment),
      ("a//b", NamespaceError::EmptySegment),
      ("a/b/c/d", NamespaceError::TooDeep),
      ("Project", NamespaceError::BadCharacter("Project".into())),
      ("pro ject", NamespaceError::BadCharacter("pro ject".into())),
      (
        "proj\u{00e9}ct",
        NamespaceError::BadCharacter("proj\u{00e9}ct".into()),
      ),
    ];
    for (path, want) in cases {
      assert_eq!(
        validate_namespace(path).unwrap_err(),
        *want,
        "path {path:?}"
      );
    }
    let long = "a".repeat(65);
    assert_eq!(
      validate_namespace(&long).unwrap_err(),
      NamespaceError::SegmentTooLong
    );
  }

  #[test]
  fn normalization_folds_case_and_trims() {
    assert_eq!(normalize_namespace(" Project/Foo "), "project/foo");
    assert_eq!(normalize_namespace("GLOBAL"), "global");
    // Post-normalization, the grammar accepts what it rejected.
    assert!(validate_namespace(&normalize_namespace("Project/Foo")).is_ok());
  }

  #[test]
  fn valid_paths_accept() {
    for path in ["global", "project/web.app", "agent/my_bot-2/quarantine"] {
      assert!(validate_namespace(path).is_ok(), "path {path:?}");
    }
  }
}

#[cfg(test)]
mod prop_tests {
  use super::*;
  use proptest::prelude::*;

  proptest! {
    // Panic-freedom on arbitrary input.
    #[test]
    fn validate_never_panics(s in ".*") {
      let _ = validate_namespace(&s);
    }

    // Every valid scope's chain ends at global, contains the
    // scope itself first, and never exceeds MAX_DEPTH + 1 links.
    #[test]
    fn chains_are_well_formed(
      segs in proptest::collection::vec("[a-z0-9._-]{1,8}", 1..=3)
    ) {
      let scope = segs.join("/");
      let chain = namespace_chain(&scope).unwrap();
      prop_assert_eq!(chain.first().unwrap(), &scope);
      prop_assert_eq!(chain.last().unwrap(), "global");
      prop_assert!(chain.len() <= MAX_DEPTH + 1);
    }
  }
}
