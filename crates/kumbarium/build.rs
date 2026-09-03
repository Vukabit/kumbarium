//! Build-time provenance capture. Emits ONLY deterministic-
//! from-source values as `cargo::rustc-env` vars (no wall-clock
//! timestamp, no build-host sysinfo) so builds stay bit-for-bit
//! reproducible. Never fails the build: a git-less (tarball /
//! air-gapped) build falls back to an injected env var, then
//! "unknown".

use std::process::Command;

fn main() {
  let sha = git(&["rev-parse", "HEAD"])
    .or_else(|| std::env::var("KUMBARIUM_GIT_SHA").ok())
    .unwrap_or_else(|| "unknown".to_string());
  let branch = git(&["rev-parse", "--abbrev-ref", "HEAD"])
    .or_else(|| std::env::var("KUMBARIUM_GIT_BRANCH").ok())
    .unwrap_or_else(|| "unknown".to_string());
  let dirty = match git(&["status", "--porcelain"]) {
    Some(s) => (!s.trim().is_empty()).to_string(),
    None => "unknown".to_string(),
  };
  let profile =
    std::env::var("PROFILE").unwrap_or_else(|_| "unknown".to_string());
  let target =
    std::env::var("TARGET").unwrap_or_else(|_| "unknown".to_string());

  emit("KUMBARIUM_GIT_SHA", &sha);
  emit("KUMBARIUM_GIT_BRANCH", &branch);
  emit("KUMBARIUM_GIT_DIRTY", &dirty);
  emit("KUMBARIUM_BUILD_PROFILE", &profile);
  emit("KUMBARIUM_TARGET", &target);

  // Refresh git values when HEAD / index move, or the injected
  // vars change.
  if let Some(git_dir) = git(&["rev-parse", "--git-dir"]) {
    println!("cargo::rerun-if-changed={git_dir}/HEAD");
    println!("cargo::rerun-if-changed={git_dir}/index");
  }
  println!("cargo::rerun-if-env-changed=KUMBARIUM_GIT_SHA");
  println!("cargo::rerun-if-env-changed=KUMBARIUM_GIT_BRANCH");
}

/// Run `git <args>` and return trimmed stdout, or None on any
/// failure (git absent, not a repo, non-zero exit, empty output).
fn git(args: &[&str]) -> Option<String> {
  let out = Command::new("git").args(args).output().ok()?;
  if !out.status.success() {
    return None;
  }
  let s = String::from_utf8(out.stdout).ok()?.trim().to_string();
  if s.is_empty() { None } else { Some(s) }
}

fn emit(key: &str, val: &str) {
  // Single-line guard: a value with an embedded newline would be
  // parsed by cargo as a second `cargo::` directive (injection).
  // Untrusted injected-fallback values flow through here, so
  // keep every emitted value to one line.
  let val = val.split(['\n', '\r']).next().unwrap_or("");
  println!("cargo::rustc-env={key}={val}");
}
