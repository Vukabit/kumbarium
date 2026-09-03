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

/// The resolved location map.
pub struct Paths {
  pub library_db: PathBuf,
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
  let dirs =
    ProjectDirs::from("", "", "kumbarium").ok_or(PathsError::NoHome)?;
  let data = dirs.data_dir();
  // config_dir differs from data_dir on Linux; never assume they
  // are the same directory.
  let config = dirs.config_dir();
  Ok(Paths {
    library_db: data.join("library.db"),
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
    writeln!(f, "library:  {}", self.library_db.display())?;
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
    let data_root = p.library_db.parent().unwrap();
    for path in [
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
