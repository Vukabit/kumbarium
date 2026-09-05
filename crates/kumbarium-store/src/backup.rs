//! Snapshot + tiered-retention backups: VACUUM INTO a temp
//! file, integrity-check the copy, atomic-rename into place.
//! Flat timestamp-named files; the tiering lives entirely in
//! the pruner, computed from filenames, so the directory holds
//! no state and a human can read it at a glance. Generic over
//! the connection: the binary points this at the audit db too.

use std::path::{Path, PathBuf};

use rusqlite::Connection;

use crate::StoreError;

/// How many snapshots each tier keeps. Tiers overlap (a file
/// can satisfy recent AND daily); the kept set is the union.
#[derive(Debug, Clone, Copy)]
pub struct Retention {
  pub recent: usize,
  pub dailies: usize,
  pub weeklies: usize,
}

const MS_PER_DAY: i64 = 86_400_000;

/// Integrity-check an existing database file WITHOUT opening it
/// for writes or running any migration (the doctor's read-only
/// probe). `deep` runs the full `integrity_check` (O(N log N),
/// index cross-checks and UNIQUE verification); otherwise the
/// cheaper `quick_check`. Returns Ok(None) when the file is
/// sound, Ok(Some(problems)) when it is not.
pub fn integrity(
  path: &Path,
  deep: bool,
) -> Result<Option<Vec<String>>, StoreError> {
  let conn = Connection::open_with_flags(
    path,
    rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY
      | rusqlite::OpenFlags::SQLITE_OPEN_NO_MUTEX,
  )?;
  let pragma = if deep {
    "PRAGMA integrity_check"
  } else {
    "PRAGMA quick_check"
  };
  let mut stmt = conn.prepare(pragma)?;
  let rows = stmt
    .query_map([], |row| row.get::<_, String>(0))?
    .collect::<Result<Vec<_>, _>>()?;
  if rows.len() == 1 && rows[0] == "ok" {
    Ok(None)
  } else {
    Ok(Some(rows))
  }
}

/// Snapshot `conn`'s database into `dir`, returning the new
/// file's path. Crash-safe: the copy lands under a temp name,
/// is integrity-checked, then renamed; a failure at any point
/// leaves no half-written snapshot behind under a final name.
pub fn backup(conn: &Connection, dir: &Path) -> Result<PathBuf, StoreError> {
  std::fs::create_dir_all(dir)?;
  let tmp = dir.join(format!(".tmp-{}.db", kumbarium_util::generate_id()));
  let tmp_str = tmp.to_string_lossy().into_owned();
  let result = (|| {
    conn.execute("VACUUM INTO ?1", [tmp_str.as_str()])?;
    let copy = Connection::open(&tmp)?;
    let verdict: String =
      copy.query_row("PRAGMA integrity_check", [], |row| row.get(0))?;
    drop(copy);
    if verdict != "ok" {
      return Err(StoreError::BackupIntegrity(verdict));
    }
    let name = file_name(kumbarium_util::now_ms());
    let target = dir.join(name);
    std::fs::rename(&tmp, &target)?;
    Ok(target)
  })();
  if result.is_err() {
    std::fs::remove_file(&tmp).ok();
  }
  result
}

/// The newest snapshot's timestamp in `dir` (epoch ms), from
/// filenames alone: the state-free answer to "when did the last
/// backup run". None when the dir is empty or absent.
pub fn latest_backup_ms(dir: &Path) -> Option<i64> {
  snapshots(dir).ok()?.first().map(|(ms, _)| *ms)
}

/// Delete snapshots outside the retention union: the newest
/// `recent` files, the newest file of each of the most recent
/// `dailies` distinct days, and of the most recent `weeklies`
/// distinct weeks. Files whose names don't parse are never
/// touched. Returns what was removed.
pub fn prune(
  dir: &Path,
  retention: Retention,
) -> Result<Vec<PathBuf>, StoreError> {
  let files = snapshots(dir)?;
  let mut keep: Vec<bool> = vec![false; files.len()];
  for k in keep.iter_mut().take(retention.recent) {
    *k = true;
  }
  mark_bucket_leaders(&files, &mut keep, retention.dailies, |ms| {
    ms.div_euclid(MS_PER_DAY)
  });
  mark_bucket_leaders(&files, &mut keep, retention.weeklies, |ms| {
    ms.div_euclid(MS_PER_DAY * 7)
  });
  let mut removed = Vec::new();
  for ((_, path), kept) in files.into_iter().zip(keep) {
    if !kept {
      std::fs::remove_file(&path)?;
      removed.push(path);
    }
  }
  Ok(removed)
}

/// Keep the newest file in each of the most recent `buckets`
/// distinct buckets. `files` is sorted newest-first, so the
/// first file seen per bucket is its leader.
fn mark_bucket_leaders(
  files: &[(i64, PathBuf)],
  keep: &mut [bool],
  buckets: usize,
  bucket_of: impl Fn(i64) -> i64,
) {
  let mut seen: Vec<i64> = Vec::new();
  for (i, (ms, _)) in files.iter().enumerate() {
    let bucket = bucket_of(*ms);
    if seen.contains(&bucket) {
      continue;
    }
    if seen.len() >= buckets {
      break;
    }
    seen.push(bucket);
    keep[i] = true;
  }
}

/// Parseable snapshots in `dir`, newest first.
pub fn snapshots(dir: &Path) -> Result<Vec<(i64, PathBuf)>, StoreError> {
  let mut out = Vec::new();
  let read = match std::fs::read_dir(dir) {
    Ok(read) => read,
    Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
      return Ok(out);
    }
    Err(e) => return Err(e.into()),
  };
  for entry in read {
    let path = entry?.path();
    if let Some(ms) = path
      .file_name()
      .and_then(|n| n.to_str())
      .and_then(parse_file_name)
    {
      out.push((ms, path));
    }
  }
  out.sort_by_key(|a| std::cmp::Reverse(a.0));
  Ok(out)
}

/// `2026-09-03T02-40-00Z.db` for the given epoch ms (second
/// precision; colons are not filesystem-safe everywhere).
fn file_name(now_ms: i64) -> String {
  let iso = kumbarium_util::format_iso8601_ms(now_ms);
  let secs = iso.get(..19).unwrap_or(&iso);
  format!("{}Z.db", secs.replace(':', "-"))
}

/// Inverse of `file_name`; None for anything else in the dir.
fn parse_file_name(name: &str) -> Option<i64> {
  let stem = name.strip_suffix("Z.db")?;
  if stem.len() != 19 {
    return None;
  }
  let (date, time) = stem.split_at(10);
  let time = time.strip_prefix('T')?;
  let iso = format!("{date}T{}Z", time.replace('-', ":"));
  kumbarium_util::parse_iso8601_ms(&iso)
}

#[cfg(test)]
mod tests {
  use super::*;

  fn ms(day: i64, hour: i64) -> i64 {
    day * MS_PER_DAY + hour * 3_600_000
  }

  #[test]
  fn file_name_round_trips() {
    let now = 1_756_857_600_000; // 2026-ish, second-aligned
    let name = file_name(now);
    assert!(name.ends_with("Z.db"), "{name}");
    assert_eq!(parse_file_name(&name), Some(now));
    assert_eq!(parse_file_name("library.db"), None);
    assert_eq!(parse_file_name(".tmp-x.db"), None);
  }

  #[test]
  fn integrity_passes_a_sound_db_and_reads_only() {
    let dir = tempfile::tempdir().unwrap();
    let src = dir.path().join("library.db");
    let _ = crate::open(&src).unwrap();
    // A sound database returns None at either tier.
    assert!(integrity(&src, false).unwrap().is_none());
    assert!(integrity(&src, true).unwrap().is_none());
    // Read-only: a truncated garbage file reports problems
    // rather than being opened for writes or migrated.
    let junk = dir.path().join("junk.db");
    std::fs::write(&junk, b"this is not a sqlite database").unwrap();
    assert!(
      integrity(&junk, false).is_err()
        || integrity(&junk, false).unwrap().is_some()
    );
  }

  #[test]
  fn backup_copies_verify_and_land_atomically() {
    let dir = tempfile::tempdir().unwrap();
    let src = dir.path().join("library.db");
    let mut conn = crate::open(&src).unwrap();
    crate::remember(
      &mut conn,
      &crate::NewEntry {
        namespace: "global".into(),
        kind: crate::Kind::Reference,
        content: "a fact worth keeping".into(),
        agent_id: "test".into(),
        source: "".into(),
        tags: vec![],
        status: crate::Status::Live,
      },
    )
    .unwrap();
    let bdir = dir.path().join("backups");
    let snap = backup(&conn, &bdir).unwrap();
    // The copy is a valid store holding the data.
    let copy = crate::open(&snap).unwrap();
    let n: i64 = copy
      .query_row("SELECT count(*) FROM entries", [], |r| r.get(0))
      .unwrap();
    assert_eq!(n, 1);
    // No temp litter.
    let stray = std::fs::read_dir(&bdir)
      .unwrap()
      .filter_map(|e| e.ok())
      .filter(|e| e.file_name().to_string_lossy().starts_with(".tmp-"))
      .count();
    assert_eq!(stray, 0);
    assert!(latest_backup_ms(&bdir).is_some());
  }

  #[test]
  fn prune_keeps_the_tier_union() {
    let dir = tempfile::tempdir().unwrap();
    // Two per day for 10 days: day D hours 0 and 12.
    for day in 0..10 {
      for hour in [0, 12] {
        let name = file_name(ms(20_700 + day, hour));
        std::fs::write(dir.path().join(name), b"x").unwrap();
      }
    }
    std::fs::write(dir.path().join("not-a-snapshot.db"), b"x").unwrap();
    let removed = prune(
      dir.path(),
      Retention {
        recent: 2,
        dailies: 7,
        weeklies: 4,
      },
    )
    .unwrap();
    let left = snapshots(dir.path()).unwrap();
    // Kept: newest 2, plus newest-per-day for 7 days (top file
    // of each day; day 9's is already in recent), plus
    // newest-per-week for the 2 distinct weeks present.
    assert!(left.len() < 20, "something was pruned");
    // The two newest snapshots survive verbatim (day 9, hours
    // 12 and 0); older days keep only their newest (hour 12).
    assert_eq!(left[0].0, ms(20_709, 12));
    assert_eq!(left[1].0, ms(20_709, 0));
    for (msv, _) in &left[2..] {
      assert_eq!(
        msv.rem_euclid(MS_PER_DAY) / 3_600_000,
        12,
        "older kept files are their day's newest"
      );
    }
    // 7 distinct days survive at their newest (hour 12) file.
    let days: std::collections::HashSet<i64> =
      left.iter().map(|(m, _)| m.div_euclid(MS_PER_DAY)).collect();
    assert!(days.len() >= 7, "days kept: {}", days.len());
    // The unparseable file was never touched.
    assert!(dir.path().join("not-a-snapshot.db").exists());
    assert!(!removed.is_empty());
  }

  #[test]
  fn latest_backup_ms_on_missing_dir_is_none() {
    let dir = tempfile::tempdir().unwrap();
    assert_eq!(latest_backup_ms(&dir.path().join("nope")), None);
  }
}
