//! Persisted-data locations, resolved per platform via
//! `directories`. Everything Kumbarium writes lives under two
//! roots (data + config); a full backup is `cp -r` of one
//! directory. See docs/design/kumbarium-design.md for the map.

use std::fmt;
use std::path::PathBuf;

use directories::ProjectDirs;

#[derive(Debug, thiserror::Error)]
pub enum PathsError {
  #[error("no home directory; cannot resolve data locations")]
  NoHome,
}

/// The resolved location map. The data dir is the building:
/// library/ holds one database per section shelf (D-033), the
/// witness stays at the root watching all of them.
pub struct Paths {
  pub memory_db: PathBuf,
  pub docket_db: PathBuf,
  pub handoff_db: PathBuf,
  pub audit_db: PathBuf,
  pub lock_file: PathBuf,
  pub backups_dir: PathBuf,
  pub exports_dir: PathBuf,
  pub logs_dir: PathBuf,
  pub config_file: PathBuf,
}

/// Resolve the map. Creates nothing; startup code decides what
/// to create and when.
pub fn resolve() -> Result<Paths, PathsError> {
  // KUMBARIUM_HOME overrides everything: data AND config land
  // under one directory. For test harnesses, portable installs,
  // and throwaway libraries; unset means platform dirs.
  if let Some(home) = std::env::var_os("KUMBARIUM_HOME") {
    let home = std::path::PathBuf::from(home);
    return Ok(Paths {
      memory_db: home.join("library").join("memory.db"),
      docket_db: home.join("library").join("docket.db"),
      handoff_db: home.join("library").join("handoff.db"),
      audit_db: home.join("audit.db"),
      lock_file: home.join("kumbarium.lock"),
      backups_dir: home.join("backups"),
      exports_dir: home.join("exports"),
      logs_dir: home.join("logs"),
      config_file: home.join("config.toml"),
    });
  }
  let dirs =
    ProjectDirs::from("", "", "kumbarium").ok_or(PathsError::NoHome)?;
  let data = dirs.data_dir();
  // config_dir differs from data_dir on Linux; never assume they
  // are the same directory.
  let config = dirs.config_dir();
  Ok(Paths {
    memory_db: data.join("library").join("memory.db"),
    docket_db: data.join("library").join("docket.db"),
    handoff_db: data.join("library").join("handoff.db"),
    audit_db: data.join("audit.db"),
    lock_file: data.join("kumbarium.lock"),
    backups_dir: data.join("backups"),
    exports_dir: data.join("exports"),
    logs_dir: data.join("logs"),
    config_file: config.join("config.toml"),
  })
}

impl fmt::Display for Paths {
  fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
    writeln!(f, "memory:   {}", self.memory_db.display())?;
    writeln!(f, "docket:   {}", self.docket_db.display())?;
    writeln!(f, "handoff:  {}", self.handoff_db.display())?;
    writeln!(f, "audit:    {}", self.audit_db.display())?;
    writeln!(f, "lock:     {}", self.lock_file.display())?;
    writeln!(f, "backups:  {}", self.backups_dir.display())?;
    writeln!(f, "exports:  {}", self.exports_dir.display())?;
    writeln!(f, "logs:     {}", self.logs_dir.display())?;
    write!(f, "config:   {}", self.config_file.display())
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn everything_lives_under_the_two_roots() {
    let p = resolve().unwrap();
    let data_root = p.memory_db.parent().unwrap().parent().unwrap();
    for path in [
      &p.docket_db,
      &p.handoff_db,
      &p.audit_db,
      &p.lock_file,
      &p.backups_dir,
      &p.exports_dir,
      &p.logs_dir,
    ] {
      assert!(
        path.starts_with(data_root),
        "{} escapes the data root",
        path.display()
      );
    }
    assert!(p.config_file.ends_with("config.toml"));
  }
}
