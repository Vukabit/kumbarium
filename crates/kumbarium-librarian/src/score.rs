//! Dual scoring: relevance and confidence are DIFFERENT numbers
//! and both travel to the agent. Relevance is computed at query
//! time (how well an entry matches this query); confidence is a
//! property of the entry (how trustworthy the fact is), fed by
//! provenance, confirmation recency, and contradiction history.

/// The score pair returned with every recall hit, plus the
/// human-readable basis line for the confidence number (a reason
/// travels better through an LLM than a bare float).
#[derive(Debug, Clone, PartialEq)]
pub struct Scores {
  /// Query-time match strength, 0.0..=1.0.
  pub relevance: f64,
  /// Entry trustworthiness, 0.0..=1.0.
  pub confidence: f64,
  /// Why the confidence is what it is, e.g.
  /// "unconfirmed for 90d" or "confirmed 2026-09-01, never
  /// contradicted".
  pub confidence_basis: String,
}

impl Scores {
  /// Clamp both scores into 0.0..=1.0 (NaN becomes 0.0), so a
  /// scoring bug can never leak an out-of-range number to agents.
  pub fn clamped(self) -> Scores {
    let clamp = |v: f64| {
      if v.is_nan() { 0.0 } else { v.clamp(0.0, 1.0) }
    };
    Scores {
      relevance: clamp(self.relevance),
      confidence: clamp(self.confidence),
      confidence_basis: self.confidence_basis,
    }
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn clamped_bounds_out_of_range_scores() {
    let s = Scores {
      relevance: 1.7,
      confidence: -0.3,
      confidence_basis: "test".into(),
    }
    .clamped();
    assert_eq!(s.relevance, 1.0);
    assert_eq!(s.confidence, 0.0);
  }

  #[test]
  fn clamped_maps_nan_to_zero() {
    let s = Scores {
      relevance: f64::NAN,
      confidence: 0.5,
      confidence_basis: "test".into(),
    }
    .clamped();
    assert_eq!(s.relevance, 0.0);
    assert_eq!(s.confidence, 0.5);
  }

  #[test]
  fn clamped_leaves_in_range_scores_alone() {
    let s = Scores {
      relevance: 0.91,
      confidence: 0.62,
      confidence_basis: "unconfirmed for 90d".into(),
    };
    assert_eq!(s.clone().clamped(), s);
  }
}
