//! The brain: namespace resolution, dual scoring types, and the
//! LLM curation seam. Pure logic; no I/O, no storage dependency.
//! The store persists, the librarian decides.

#![forbid(unsafe_code)]

mod namespace;
mod score;
mod split;

pub use namespace::{
  MAX_DEPTH, NamespaceError, namespace_chain, validate_namespace,
};
pub use score::Scores;
pub use split::{SPLIT_TARGET, split_for_storage};

/// The LLM curation seam. The write path (dedup, merge,
/// contradiction checks) and the janitor call through this trait;
/// v0.1 ships NO implementation (an Ollama-backed one comes
/// later), and nothing in the hot read path may ever require one.
pub trait Curator {
  /// Judge whether `candidate` duplicates or contradicts any of
  /// `existing` (entry contents). Returns the indices of entries
  /// the candidate supersedes; empty means store as new.
  fn supersedes(&self, candidate: &str, existing: &[String]) -> Vec<usize>;
}
