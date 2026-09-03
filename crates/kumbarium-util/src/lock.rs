//! Cross-platform process lock (RAII): advisory on unix (flock),
//! mandatory on Windows (LockFileEx).

use std::path::Path;

#[derive(Debug, thiserror::Error)]
pub enum LockError {
  #[error("lock I/O failed")]
  Io,
}

/// RAII cross-platform exclusive lock on the sibling file
/// `<resource>.lock`: advisory on unix (flock), mandatory on Windows
/// (LockFileEx). Non-blocking: contention returns immediately. Held
/// until drop; the OS releases it when the fd closes (drop or process
/// death).
pub struct ProcessLock {
  // Holds the .lock fd open; the exclusive lock lives as long as
  // this fd is open, and the OS releases it when the fd closes
  // (on drop or process exit).
  _lock: fd_lock::RwLock<std::fs::File>,
}

impl ProcessLock {
  /// None on contention (held by another process); Err on I/O.
  pub fn try_acquire(
    resource: &Path,
  ) -> Result<Option<ProcessLock>, LockError> {
    // Sibling `<resource>.lock` (append, don't replace ext).
    let mut p = resource.as_os_str().to_owned();
    p.push(".lock");
    // write(true) to take an exclusive lock; truncate(false) since we
    // never write content and must not destroy an existing lock file.
    let file = std::fs::OpenOptions::new()
      .create(true)
      .write(true)
      .truncate(false)
      .open(std::path::PathBuf::from(p))
      .map_err(|_| LockError::Io)?;

    let mut rwlock = fd_lock::RwLock::new(file);

    // Apply the lock + forget the guard so its Drop can't unlock;
    // this ENDS the borrow of `rwlock` before we move it below.
    let acquired = match rwlock.try_write() {
      Ok(guard) => {
        std::mem::forget(guard);
        true
      }
      Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => false,
      Err(_) => return Err(LockError::Io),
    };
    if acquired {
      Ok(Some(ProcessLock { _lock: rwlock }))
    } else {
      Ok(None)
    }
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  fn scratch(tag: &str) -> std::path::PathBuf {
    let d = std::env::temp_dir().join(format!(
      "kumbarium-util-lock-{}-{}",
      tag,
      std::process::id()
    ));
    std::fs::create_dir_all(&d).unwrap();
    d
  }

  #[test]
  fn acquires_a_free_resource_and_creates_lockfile() {
    let dir = scratch("free");
    let res = dir.join("resource");
    let held = ProcessLock::try_acquire(&res).unwrap();
    assert!(held.is_some());
    // The sibling `<resource>.lock` file was created.
    assert!(dir.join("resource.lock").exists());
    std::fs::remove_dir_all(&dir).ok();
  }

  #[test]
  fn second_acquire_contends_then_release_reacquires() {
    let dir = scratch("contend");
    let res = dir.join("resource");

    let first = ProcessLock::try_acquire(&res).unwrap();
    assert!(first.is_some(), "first acquire");

    // A second lock on the same resource must not be granted.
    let second = ProcessLock::try_acquire(&res).unwrap();
    assert!(second.is_none(), "contention returns None");

    // Dropping the first closes its fd, releasing the lock.
    drop(first);
    let third = ProcessLock::try_acquire(&res).unwrap();
    assert!(third.is_some(), "re-acquire after release");

    std::fs::remove_dir_all(&dir).ok();
  }
}
