//! Disciplined file I/O: a crash-safe atomic byte writer and a
//! size-capped, symlink-refusing reader.

use std::io::Write;
use std::path::Path;

#[derive(Debug, thiserror::Error)]
pub enum WriteError {
  #[error("temp-file creation failed")]
  Temp,
  #[error("write or fsync failed")]
  Io,
  #[error("atomic rename failed")]
  Rename,
}

#[derive(Debug, thiserror::Error)]
pub enum ReadError {
  #[error("file not found")]
  NotFound,
  #[error("path is a symlink (refused)")]
  Symlink,
  #[error("file exceeds the size cap")]
  TooLarge,
  #[error("read failed")]
  Io,
}

/// Serialize-agnostic crash-safe write: `bytes` land whole at
/// `path`, or the prior file is left intact. tmp-in-same-dir ->
/// fsync -> rename -> (unix) fsync parent dir. The caller owns
/// serialization; util never sees a format.
///
/// Durability caveats: the parent-directory fsync (making the rename
/// itself survive power loss) is unix-only; on Windows durability
/// relies on the rename's own semantics. And if that post-rename
/// fsync fails, the error is returned AFTER the new bytes are already
/// in place: on `Rename` the prior file is intact, but on a post-
/// rename `Io` the new file has landed and only its durability is
/// unconfirmed.
pub fn write_atomically(path: &Path, bytes: &[u8]) -> Result<(), WriteError> {
  // Temp in the target's OWN dir => same filesystem => the rename
  // is atomic and never a silent cross-device copy.
  let dir = path
    .parent()
    .filter(|p| !p.as_os_str().is_empty())
    .unwrap_or_else(|| Path::new("."));

  let mut tmp =
    tempfile::NamedTempFile::new_in(dir).map_err(|_| WriteError::Temp)?;
  tmp.write_all(bytes).map_err(|_| WriteError::Io)?;
  tmp.as_file().sync_all().map_err(|_| WriteError::Io)?;
  tmp.persist(path).map_err(|_| WriteError::Rename)?;

  // fsync the parent dir so the rename itself is durable.
  #[cfg(unix)]
  {
    use rustix::fs::{Mode, OFlags};
    let fd = rustix::fs::open(dir, OFlags::RDONLY, Mode::empty())
      .map_err(|_| WriteError::Io)?;
    rustix::fs::fsync(fd).map_err(|_| WriteError::Io)?;
  }
  Ok(())
}

/// Read up to `max_bytes` of `path`, refusing anything that is not a
/// regular file. A final-component symlink is refused (on unix
/// atomically via a NOFOLLOW open, no check-then-open race; on Windows
/// via a symlink pre-check then open, a small TOCTOU window,
/// mitigated by Windows requiring privilege to create symlinks); a
/// FIFO, device, or directory is refused after open (`O_NONBLOCK` on
/// unix so a writer-less FIFO cannot hang the open). A file over the
/// cap returns `TooLarge` without reading it in. An empty file is
/// `Ok(vec![])`; callers distinguish empty via `.len()`.
pub fn read_bytes(path: &Path, max_bytes: usize) -> Result<Vec<u8>, ReadError> {
  use std::io::Read;

  // Open, refusing a final-component symlink. NONBLOCK so opening a
  // writer-less FIFO returns immediately instead of blocking forever
  // (the is_file check below then refuses it).
  #[cfg(unix)]
  let file = {
    use rustix::fs::{Mode, OFlags};
    let fd = rustix::fs::open(
      path,
      OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::NONBLOCK,
      Mode::empty(),
    )
    .map_err(|e| match e {
      rustix::io::Errno::LOOP => ReadError::Symlink,
      rustix::io::Errno::NOENT => ReadError::NotFound,
      _ => ReadError::Io,
    })?;
    std::fs::File::from(fd)
  };

  #[cfg(not(unix))]
  let file = {
    // No atomic O_NOFOLLOW on Windows: pre-check then open.
    match std::fs::symlink_metadata(path) {
      Ok(m) if m.file_type().is_symlink() => {
        return Err(ReadError::Symlink);
      }
      Ok(_) => {}
      Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
        return Err(ReadError::NotFound);
      }
      Err(_) => {
        return Err(ReadError::Io);
      }
    }
    std::fs::File::open(path).map_err(|e| match e.kind() {
      std::io::ErrorKind::NotFound => ReadError::NotFound,
      _ => ReadError::Io,
    })?
  };

  // Refuse anything that is not a regular file (FIFO, device,
  // directory); the metadata is on the already-open fd, so no race.
  let meta = file.metadata().map_err(|_| ReadError::Io)?;
  if !meta.is_file() {
    return Err(ReadError::Io);
  }
  // Fast-reject oversize without reading it in.
  if meta.len() > max_bytes as u64 {
    return Err(ReadError::TooLarge);
  }

  // Read with a +1 cap so a file that GREW after the stat trips
  // TooLarge rather than silently truncating. saturating so a
  // usize::MAX cap can't overflow.
  let mut buf = Vec::new();
  (&file)
    .take((max_bytes as u64).saturating_add(1))
    .read_to_end(&mut buf)
    .map_err(|_| ReadError::Io)?;
  if buf.len() > max_bytes {
    return Err(ReadError::TooLarge);
  }
  Ok(buf)
}

#[cfg(test)]
mod tests {
  use super::*;

  // A unique per-process scratch dir; cleaned at the end of each test.
  fn scratch(tag: &str) -> std::path::PathBuf {
    let d = std::env::temp_dir().join(format!(
      "kumbarium-util-{}-{}",
      tag,
      std::process::id()
    ));
    std::fs::create_dir_all(&d).unwrap();
    d
  }

  #[test]
  fn write_lands_whole_and_reads_back() {
    let dir = scratch("write");
    let target = dir.join("state.bin");
    write_atomically(&target, b"hello world").unwrap();
    assert_eq!(std::fs::read(&target).unwrap(), b"hello world");
    std::fs::remove_dir_all(&dir).ok();
  }

  #[test]
  fn overwrite_replaces_atomically() {
    let dir = scratch("overwrite");
    let target = dir.join("state.bin");
    write_atomically(&target, b"first").unwrap();
    write_atomically(&target, b"second-and-longer").unwrap();
    assert_eq!(std::fs::read(&target).unwrap(), b"second-and-longer");
    std::fs::remove_dir_all(&dir).ok();
  }

  #[test]
  fn empty_bytes_yields_empty_file() {
    let dir = scratch("empty");
    let target = dir.join("state.bin");
    write_atomically(&target, b"").unwrap();
    assert!(std::fs::read(&target).unwrap().is_empty());
    std::fs::remove_dir_all(&dir).ok();
  }

  #[test]
  fn leaves_no_stray_temp_files() {
    let dir = scratch("stray");
    let target = dir.join("state.bin");
    write_atomically(&target, b"a").unwrap();
    write_atomically(&target, b"bb").unwrap();
    // Only the target remains; persist consumed each temp.
    let entries = std::fs::read_dir(&dir).unwrap().count();
    assert_eq!(entries, 1);
    std::fs::remove_dir_all(&dir).ok();
  }

  #[test]
  fn reads_a_file_whole() {
    let dir = scratch("read");
    let f = dir.join("data");
    std::fs::write(&f, b"hello").unwrap();
    assert_eq!(read_bytes(&f, 1024).unwrap(), b"hello");
    std::fs::remove_dir_all(&dir).ok();
  }

  #[test]
  fn empty_file_reads_ok_empty() {
    let dir = scratch("read-empty");
    let f = dir.join("data");
    std::fs::write(&f, b"").unwrap();
    assert!(read_bytes(&f, 1024).unwrap().is_empty());
    std::fs::remove_dir_all(&dir).ok();
  }

  #[test]
  fn at_cap_reads_over_cap_rejects() {
    let dir = scratch("read-cap");
    let f = dir.join("data");
    std::fs::write(&f, b"12345").unwrap();
    // Exactly at the cap reads whole; one below trips TooLarge.
    assert_eq!(read_bytes(&f, 5).unwrap(), b"12345");
    assert!(matches!(read_bytes(&f, 4), Err(ReadError::TooLarge)));
    std::fs::remove_dir_all(&dir).ok();
  }

  #[test]
  fn missing_path_is_not_found() {
    let dir = scratch("read-missing");
    let missing = dir.join("nope");
    assert!(matches!(
      read_bytes(&missing, 1024),
      Err(ReadError::NotFound)
    ));
    std::fs::remove_dir_all(&dir).ok();
  }

  #[test]
  fn non_regular_file_is_refused() {
    // A directory is not a regular file; the same is_file guard that
    // refuses a FIFO/device catches it (portable, no mkfifo needed).
    let dir = scratch("read-nonreg");
    assert!(matches!(read_bytes(&dir, 1024), Err(ReadError::Io)));
    std::fs::remove_dir_all(&dir).ok();
  }

  #[cfg(unix)]
  #[test]
  fn symlink_is_refused() {
    let dir = scratch("read-symlink");
    let real = dir.join("real");
    std::fs::write(&real, b"secret").unwrap();
    let link = dir.join("link");
    std::os::unix::fs::symlink(&real, &link).unwrap();
    // The refusal is atomic (NOFOLLOW); the link is never followed.
    assert!(matches!(read_bytes(&link, 1024), Err(ReadError::Symlink)));
    std::fs::remove_dir_all(&dir).ok();
  }
}
