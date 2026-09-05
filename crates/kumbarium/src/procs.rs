//! The presence registry (D-048): which kumbarium processes
//! are alive on this library home, on which binary, spawned by
//! whom. One JSON record per process under `library/procs/`,
//! made trustworthy by an OS advisory lock (kumbarium-util's
//! ProcessLock): the fd lock dies with the process, so
//! liveness is "does the lock refuse me", never a pid check a
//! recycled pid could fool. Records are debris the moment
//! their lock releases; readers skip them and the doctor
//! sweeps them.
//!
//! Only long-running processes register (today: serve). A
//! CLI invocation lives milliseconds; registering it would be
//! churn without information.

use std::path::{Path, PathBuf};

use serde_json::{Value, json};

/// What a record says about its process. Everything here is
/// display truth for `kum processes`; the LOCK is the liveness
/// truth.
#[derive(Debug, Clone)]
pub struct PresenceInfo {
  pub pid: u32,
  pub version: String,
  pub agent: String,
  pub session: String,
  pub client: String,
  pub since: String,
}

/// A held registration: the record file plus the lock that
/// vouches for it. Dropping removes the record (clean exit);
/// process death releases the lock and leaves the file as
/// sweepable debris.
pub struct Presence {
  // Option so Drop can release the lock BEFORE unlinking: on
  // Windows a locked-open file refuses deletion.
  lock: Option<kumbarium_util::ProcessLock>,
  path: PathBuf,
}

impl Presence {
  /// Register this process. Returns None (never an error a
  /// caller must handle) when the registry cannot be written:
  /// presence is awareness, and awareness must never stop the
  /// library from serving.
  pub fn register(procs_dir: &Path, info: &PresenceInfo) -> Option<Presence> {
    std::fs::create_dir_all(procs_dir).ok()?;
    let path = procs_dir.join(format!("{}.json", info.pid));
    std::fs::write(&path, render(info)).ok()?;
    let lock = kumbarium_util::ProcessLock::try_acquire(&path).ok()??;
    Some(Presence {
      lock: Some(lock),
      path,
    })
  }

  /// Rewrite the record (the claimed agent arrives at
  /// initialize, after registration). Failures are ignored for
  /// the same reason register's are.
  pub fn update(&self, info: &PresenceInfo) {
    let _ = std::fs::write(&self.path, render(info));
  }
}

impl Drop for Presence {
  fn drop(&mut self) {
    // Release the lock first (Windows refuses to delete a
    // locked-open file), then remove record and sibling.
    drop(self.lock.take());
    let _ = std::fs::remove_file(&self.path);
    let mut lock = self.path.as_os_str().to_owned();
    lock.push(".lock");
    let _ = std::fs::remove_file(PathBuf::from(lock));
  }
}

fn render(info: &PresenceInfo) -> String {
  serde_json::to_string_pretty(&json!({
    "pid": info.pid,
    "version": info.version,
    "agent": info.agent,
    "session": info.session,
    "client": info.client,
    "since": info.since,
  }))
  .unwrap_or_default()
    + "\n"
}

fn parse(text: &str) -> Option<PresenceInfo> {
  let v: Value = serde_json::from_str(text).ok()?;
  let s = |k: &str| {
    v.get(k)
      .and_then(Value::as_str)
      .unwrap_or_default()
      .to_string()
  };
  Some(PresenceInfo {
    pid: v.get("pid").and_then(Value::as_u64)? as u32,
    version: s("version"),
    agent: s("agent"),
    session: s("session"),
    client: s("client"),
    since: s("since"),
  })
}

/// Every record whose lock refuses us: the live processes.
/// Unlocked records are debris and are skipped (the doctor's
/// to sweep, via `stale`).
pub fn live(procs_dir: &Path) -> Vec<PresenceInfo> {
  scan(procs_dir).into_iter().filter_map(|r| r.info).collect()
}

/// Record files whose lock we could take: dead processes'
/// leavings, safe to remove.
pub fn stale(procs_dir: &Path) -> Vec<PathBuf> {
  scan(procs_dir)
    .into_iter()
    .filter(|r| r.info.is_none())
    .map(|r| r.path)
    .collect()
}

struct Scanned {
  path: PathBuf,
  /// Some = alive (lock refused us), None = debris.
  info: Option<PresenceInfo>,
}

fn scan(procs_dir: &Path) -> Vec<Scanned> {
  let Ok(read) = std::fs::read_dir(procs_dir) else {
    return Vec::new();
  };
  let mut out = Vec::new();
  for entry in read.flatten() {
    let path = entry.path();
    if path.extension().and_then(|e| e.to_str()) != Some("json") {
      continue;
    }
    // Acquiring the lock proves the holder is DEAD (and we
    // release our probe immediately by dropping it); WouldBlock
    // proves it is alive.
    let alive = match kumbarium_util::ProcessLock::try_acquire(&path) {
      Ok(Some(probe)) => {
        drop(probe);
        false
      }
      Ok(None) => true,
      Err(_) => false,
    };
    let info = if alive {
      std::fs::read_to_string(&path).ok().and_then(|t| parse(&t))
    } else {
      None
    };
    out.push(Scanned { path, info });
  }
  out
}

/// The parent process's name, for the client column ("which of
/// my terminals is this"). Best-effort: one `ps` call at
/// registration time, empty on anything unexpected.
#[cfg(unix)]
pub fn parent_client_name() -> String {
  let ppid = unsafe { libc::getppid() };
  std::process::Command::new("ps")
    .args(["-o", "comm=", "-p", &ppid.to_string()])
    .output()
    .ok()
    .and_then(|o| String::from_utf8(o.stdout).ok())
    .map(|s| {
      let name = s.trim();
      // A path like /Applications/.../claude reads better as
      // its basename.
      name.rsplit('/').next().unwrap_or(name).to_string()
    })
    .unwrap_or_default()
}

#[cfg(not(unix))]
pub fn parent_client_name() -> String {
  String::new()
}

#[cfg(test)]
mod tests {
  use super::*;

  fn info(pid: u32) -> PresenceInfo {
    PresenceInfo {
      pid,
      version: "0.0.0-test".into(),
      agent: "test-agent".into(),
      session: "0123456789abcdef".into(),
      client: "cargo-test".into(),
      since: kumbarium_util::now_iso8601(),
    }
  }

  #[test]
  fn register_appears_live_and_drop_cleans_up() {
    let dir =
      std::env::temp_dir().join(format!("kum-procs-{}", std::process::id()));
    std::fs::remove_dir_all(&dir).ok();
    let held = Presence::register(&dir, &info(std::process::id())).unwrap();
    let rows = live(&dir);
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].agent, "test-agent");
    assert!(stale(&dir).is_empty());
    drop(held);
    assert!(live(&dir).is_empty());
    assert!(stale(&dir).is_empty(), "clean exit leaves no debris");
    std::fs::remove_dir_all(&dir).ok();
  }

  #[test]
  fn an_unlocked_record_is_debris_not_a_ghost() {
    let dir = std::env::temp_dir()
      .join(format!("kum-procs-ghost-{}", std::process::id()));
    std::fs::remove_dir_all(&dir).ok();
    std::fs::create_dir_all(&dir).unwrap();
    // A record written by a "process" that never held (or no
    // longer holds) the lock: the crashed shape.
    std::fs::write(dir.join("99999.json"), render(&info(99999))).unwrap();
    assert!(live(&dir).is_empty(), "no lock, no liveness");
    assert_eq!(stale(&dir).len(), 1);
    std::fs::remove_dir_all(&dir).ok();
  }
}
