//! Time-ordered unique id (UUIDv7) generation + validation.

/// Generate a UUIDv7 (RFC 9562): a 48-bit Unix-ms timestamp in the
/// high bits, then a monotonic counter and random bits, rendered
/// as the canonical 36-char lowercase-hex form (8-4-4-4-12). The
/// layout is big-endian and time-leading, so lexicographic string
/// order equals chronological order (relied on for filename
/// ordering). The monotonic counter guarantees ordering even for
/// ids minted within the same millisecond.
pub fn generate_id() -> String {
  uuid::Uuid::now_v7().hyphenated().to_string()
}

/// True iff `candidate` is a canonical UUIDv7: 36 chars, lowercase
/// hex with dashes at 8-4-4-4-12, version nibble 7, RFC 9562
/// variant. TRUST BOUNDARY; call first on any id from an untrusted
/// source before it reaches path assembly or a store lookup.
pub fn is_valid_id(candidate: &str) -> bool {
  match uuid::Uuid::try_parse(candidate) {
    Ok(u) => {
      u.get_version() == Some(uuid::Version::SortRand)
        && u.get_variant() == uuid::Variant::RFC4122
        && u.hyphenated().to_string() == candidate
    }
    Err(_) => false,
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  // A known-canonical UUIDv7 literal (version nibble 7, RFC 9562
  // variant), used as the accept case + the base for reject cases.
  const CANON_V7: &str = "01912d68-783e-7cde-8f1a-4b2c9e0f3a71";

  #[test]
  fn generated_id_is_a_canonical_v7() {
    let id = generate_id();
    assert_eq!(id.len(), 36);
    assert!(is_valid_id(&id));
    // Version nibble is the first char of the third dash-group.
    assert_eq!(id.as_bytes()[14], b'7');
  }

  #[test]
  fn sequential_ids_are_monotonic() {
    let ids: Vec<String> = (0..1000).map(|_| generate_id()).collect();
    let mut sorted = ids.clone();
    sorted.sort();
    // Lexical order == mint order: the intra-ms monotonic guarantee.
    assert_eq!(ids, sorted);
    // And they are unique.
    sorted.dedup();
    assert_eq!(sorted.len(), ids.len());
  }

  #[test]
  fn is_valid_id_accepts_canonical() {
    assert!(is_valid_id(CANON_V7));
  }

  #[test]
  fn is_valid_id_rejects_non_canonical_and_non_v7() {
    let uppercase = CANON_V7.to_uppercase();
    let braced = format!("{{{CANON_V7}}}");
    let unhyphenated = CANON_V7.replace('-', "");
    let cases = [
      uppercase.as_str(),                     // uppercase
      "00000000-0000-0000-0000-000000000000", // nil, non-v7
      "01912d68-783e-4cde-8f1a-4b2c9e0f3a71", // version 4, not 7
      "01912d68-783e-7cde-0f1a-4b2c9e0f3a71", // v7 but NCS variant (0)
      braced.as_str(),                        // {..}
      unhyphenated.as_str(),                  // no dashes
      "01912d68-783e-7cde-8f1a-4b2c9e0f3a7",  // 35 chars
      "not-a-uuid",
      "",
    ];
    for bad in cases {
      assert!(!is_valid_id(bad), "should reject {bad:?}");
    }
  }
}

#[cfg(test)]
mod prop_tests {
  use super::*;
  use proptest::prelude::*;

  proptest! {
    #[test]
    fn every_generated_id_validates(_ in 0..64u8) {
      prop_assert!(is_valid_id(&generate_id()));
    }

    // Panic-freedom on arbitrary input (the always-on layer; the
    // coverage-guided libfuzzer target lives in fuzz/).
    #[test]
    fn is_valid_id_never_panics(s in ".*") {
      let _ = is_valid_id(&s);
    }

    // Denser near the UUID grammar (hex + dashes, near-canonical len).
    #[test]
    fn is_valid_id_never_panics_near_grammar(s in "[0-9a-fA-F-]{0,40}") {
      let _ = is_valid_id(&s);
    }
  }
}
