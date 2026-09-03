//! The restricted stacks (docs/design/secrets.md, D-038):
//! sealed credential custody on the vetted floor (D-039).
//! Witnessed access is the product; sealing is XChaCha20-
//! Poly1305 under a caller-supplied master key (the keystore
//! lives in the binary, reached by shelling the OS tool), with
//! a versioned envelope and AAD binding namespace+name so a
//! ciphertext can never be re-shelved. Rotation supersedes and
//! SHREDS the ancestor's value; grants are deny-by-default with
//! read-time lease checks. Nothing on this shelf is ever
//! served (the standing exception to D-037).

#![forbid(unsafe_code)]

use std::path::Path;

use chacha20poly1305::aead::{Aead, KeyInit, Payload};
use chacha20poly1305::{XChaCha20Poly1305, XNonce};
use rusqlite::params;
use zeroize::Zeroizing;

pub use rusqlite::Connection;

const MIGRATIONS: &[(i64, &str, &str)] =
  &[(1, "0001_init", include_str!("../migrations/0001_init.sql"))];

/// Envelope version byte; an unknown version fails closed
/// (D-039). v1 = XChaCha20-Poly1305, 24-byte nonce prefix.
const ENVELOPE_V1: u8 = 1;
/// Envelope version for a shelf the human explicitly chose to
/// run unsealed (absent keystore substrate): the value is
/// stored as-is behind the same interface, honestly tagged.
const ENVELOPE_PLAIN: u8 = 0;

pub const KEY_LEN: usize = 32;
const NONCE_LEN: usize = 24;

#[derive(Debug, thiserror::Error)]
pub enum SecretsError {
  #[error("sqlite error: {0}")]
  Sqlite(#[from] rusqlite::Error),
  #[error("migration {0} failed: {1}")]
  Migration(i64, rusqlite::Error),
  #[error("no secret named {0:?} on that shelf")]
  SecretNotFound(String),
  #[error("no secret with id {0:?}")]
  IdNotFound(String),
  #[error("id fragment {0:?} matches more than one secret")]
  AmbiguousId(String),
  #[error("secret {0:?} has been shredded; the material is gone")]
  Shredded(String),
  #[error("secret value is empty")]
  EmptyValue,
  #[error("sealing failed (RNG or cipher); nothing was stored")]
  SealFailure,
  #[error(
    "unseal failed: wrong key, tampered envelope, or a \
     ciphertext moved between shelves"
  )]
  OpenFailure,
  #[error("unknown envelope version {0}; refusing a best-effort parse")]
  UnknownVersion(u8),
  #[error(
    "this shelf seals with {0:?} but the operation assumed {1:?}; \
     sealing mode is chosen at shelf creation and never silently \
     changes"
  )]
  SealingMismatch(String, String),
}

/// How a shelf seals, decided once at creation (D-039: absent
/// substrate is a loud, explicit human choice, never a silent
/// downgrade).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Sealing {
  Keystore,
  Plaintext,
}

impl Sealing {
  pub fn as_str(self) -> &'static str {
    match self {
      Sealing::Keystore => "keystore",
      Sealing::Plaintext => "plaintext",
    }
  }

  pub fn parse(s: &str) -> Option<Sealing> {
    match s {
      "keystore" => Some(Sealing::Keystore),
      "plaintext" => Some(Sealing::Plaintext),
      _ => None,
    }
  }
}

/// A secret's metadata: everything EXCEPT the value. Listing,
/// history, and show surfaces speak this type only, so a value
/// cannot reach them by shape.
#[derive(Debug, Clone)]
pub struct SecretMeta {
  pub id: String,
  pub namespace: String,
  pub name: String,
  pub agent_id: String,
  pub superseded_by: Option<String>,
  pub note: Option<String>,
  pub shredded_at: Option<String>,
  pub created_at: String,
  pub updated_at: String,
}

/// A grant row.
#[derive(Debug, Clone)]
pub struct Grant {
  pub namespace: String,
  pub name: String,
  pub agent_id: String,
  pub mode: String,
  pub expires_at: Option<String>,
  pub created_at: String,
}

/// Open (creating if absent) the shelf at `path`.
pub fn open(path: &Path) -> Result<Connection, SecretsError> {
  let conn = Connection::open(path)?;
  configure(&conn)?;
  migrate(&conn)?;
  Ok(conn)
}

/// In-memory shelf with the schema applied (tests).
pub fn open_in_memory() -> Result<Connection, SecretsError> {
  let conn = Connection::open_in_memory()?;
  configure(&conn)?;
  migrate(&conn)?;
  Ok(conn)
}

fn configure(conn: &Connection) -> Result<(), SecretsError> {
  conn.pragma_update(None, "journal_mode", "wal")?;
  conn.pragma_update(None, "busy_timeout", 5000)?;
  conn.pragma_update(None, "foreign_keys", "on")?;
  conn.pragma_update(None, "synchronous", "normal")?;
  Ok(())
}

fn migrate(conn: &Connection) -> Result<(), SecretsError> {
  conn.execute(
    "CREATE TABLE IF NOT EXISTS schema_version (
       version INTEGER PRIMARY KEY,
       name TEXT NOT NULL,
       applied_at TEXT NOT NULL
     )",
    [],
  )?;
  let current: i64 = conn.query_row(
    "SELECT COALESCE(MAX(version), 0) FROM schema_version",
    [],
    |row| row.get(0),
  )?;
  for (version, name, sql) in MIGRATIONS {
    if *version <= current {
      continue;
    }
    conn
      .execute_batch(&format!("BEGIN;\n{sql}\nCOMMIT;"))
      .map_err(|e| SecretsError::Migration(*version, e))?;
    conn.execute(
      "INSERT INTO schema_version (version, name, applied_at)
       VALUES (?1, ?2, ?3)",
      params![version, name, kumbarium_util::now_iso8601()],
    )?;
  }
  Ok(())
}

/// The short display form of an id.
pub fn short_id(id: &str) -> &str {
  id.get(id.len().saturating_sub(8)..).unwrap_or(id)
}

/// The shelf's sealing mode, set on first use.
pub fn sealing_mode(
  conn: &Connection,
) -> Result<Option<Sealing>, SecretsError> {
  let raw: Option<String> = conn
    .query_row("SELECT mode FROM sealing WHERE id = 1", [], |row| {
      row.get(0)
    })
    .map(Some)
    .or_else(|e| match e {
      rusqlite::Error::QueryReturnedNoRows => Ok(None),
      other => Err(other),
    })?;
  Ok(raw.and_then(|r| Sealing::parse(&r)))
}

/// Record the shelf's sealing mode (first use only; a second
/// call with a different mode errors).
pub fn set_sealing_mode(
  conn: &Connection,
  mode: Sealing,
) -> Result<(), SecretsError> {
  match sealing_mode(conn)? {
    None => {
      conn.execute(
        "INSERT INTO sealing (id, mode, created_at)
         VALUES (1, ?1, ?2)",
        params![mode.as_str(), kumbarium_util::now_iso8601()],
      )?;
      Ok(())
    }
    Some(existing) if existing == mode => Ok(()),
    Some(existing) => Err(SecretsError::SealingMismatch(
      existing.as_str().into(),
      mode.as_str().into(),
    )),
  }
}

/// Length-prefixed canonical AAD: a ciphertext is bound to its
/// shelf and name and can never be re-shelved.
fn aad_for(namespace: &str, name: &str) -> Vec<u8> {
  let mut aad = Vec::new();
  for field in [namespace, name] {
    aad.extend_from_slice(&(field.len() as u32).to_le_bytes());
    aad.extend_from_slice(field.as_bytes());
  }
  aad
}

/// Seal a value into a versioned envelope. The nonce is fresh
/// random from the OS; an RNG failure fails closed, never a
/// zero or reused nonce (D-039).
fn seal(
  key: &[u8; KEY_LEN],
  namespace: &str,
  name: &str,
  value: &[u8],
) -> Result<Vec<u8>, SecretsError> {
  let cipher = XChaCha20Poly1305::new(key.into());
  let mut nonce_bytes = [0u8; NONCE_LEN];
  getrandom_fill(&mut nonce_bytes)?;
  let nonce = XNonce::from_slice(&nonce_bytes);
  let aad = aad_for(namespace, name);
  let ciphertext = cipher
    .encrypt(
      nonce,
      Payload {
        msg: value,
        aad: &aad,
      },
    )
    .map_err(|_| SecretsError::SealFailure)?;
  let mut envelope = Vec::with_capacity(1 + NONCE_LEN + ciphertext.len());
  envelope.push(ENVELOPE_V1);
  envelope.extend_from_slice(&nonce_bytes);
  envelope.extend_from_slice(&ciphertext);
  Ok(envelope)
}

/// OS entropy via the vetted primitive (getrandom is on the
/// D-039 floor, [redacted] precedent); a failure fails closed,
/// never a zero or derived nonce.
fn getrandom_fill(out: &mut [u8; NONCE_LEN]) -> Result<(), SecretsError> {
  getrandom::getrandom(out).map_err(|_| SecretsError::SealFailure)
}

/// Open a sealed envelope. Unknown versions fail closed.
fn unseal(
  key: &[u8; KEY_LEN],
  namespace: &str,
  name: &str,
  envelope: &[u8],
) -> Result<Zeroizing<Vec<u8>>, SecretsError> {
  let (&version, rest) =
    envelope.split_first().ok_or(SecretsError::OpenFailure)?;
  match version {
    ENVELOPE_V1 => {
      if rest.len() < NONCE_LEN {
        return Err(SecretsError::OpenFailure);
      }
      let (nonce_bytes, ciphertext) = rest.split_at(NONCE_LEN);
      let cipher = XChaCha20Poly1305::new(key.into());
      let aad = aad_for(namespace, name);
      let value = cipher
        .decrypt(
          XNonce::from_slice(nonce_bytes),
          Payload {
            msg: ciphertext,
            aad: &aad,
          },
        )
        .map_err(|_| SecretsError::OpenFailure)?;
      Ok(Zeroizing::new(value))
    }
    ENVELOPE_PLAIN => Ok(Zeroizing::new(rest.to_vec())),
    other => Err(SecretsError::UnknownVersion(other)),
  }
}

/// Store or rotate a secret. A live head with the same name is
/// superseded and its value SHREDDED in the same transaction:
/// the rotation history keeps the skeleton, never the material.
/// `key` is None only on a plaintext-mode shelf.
pub fn set_secret(
  conn: &Connection,
  namespace: &str,
  name: &str,
  value: &[u8],
  key: Option<&[u8; KEY_LEN]>,
  note: Option<&str>,
) -> Result<SecretMeta, SecretsError> {
  if value.is_empty() {
    return Err(SecretsError::EmptyValue);
  }
  let mode = match key {
    Some(_) => Sealing::Keystore,
    None => Sealing::Plaintext,
  };
  set_sealing_mode(conn, mode)?;
  let sealed = match key {
    Some(k) => seal(k, namespace, name, value)?,
    None => {
      let mut envelope = Vec::with_capacity(1 + value.len());
      envelope.push(ENVELOPE_PLAIN);
      envelope.extend_from_slice(value);
      envelope
    }
  };
  conn.execute_batch("BEGIN IMMEDIATE")?;
  let result = set_locked(conn, namespace, name, &sealed, note);
  match &result {
    Ok(_) => conn.execute_batch("COMMIT")?,
    Err(_) => {
      let _ = conn.execute_batch("ROLLBACK");
    }
  }
  result
}

fn set_locked(
  conn: &Connection,
  namespace: &str,
  name: &str,
  sealed: &[u8],
  note: Option<&str>,
) -> Result<SecretMeta, SecretsError> {
  let prior: Option<String> = conn
    .query_row(
      "SELECT id FROM secrets
       WHERE namespace = ?1 AND name = ?2
         AND superseded_by IS NULL AND shredded_at IS NULL",
      params![namespace, name],
      |row| row.get(0),
    )
    .map(Some)
    .or_else(|e| match e {
      rusqlite::Error::QueryReturnedNoRows => Ok(None),
      other => Err(other),
    })?;
  let id = kumbarium_util::generate_id();
  let now = kumbarium_util::now_iso8601();
  conn.execute(
    "INSERT INTO secrets
       (id, namespace, name, sealed, agent_id, note,
        created_at, updated_at)
     VALUES (?1, ?2, ?3, ?4, 'kumbarium-cli', ?5, ?6, ?6)",
    params![id, namespace, name, sealed, note, now],
  )?;
  if let Some(prev) = prior {
    // Rotation: chain forward AND shred the retired value.
    conn.execute(
      "UPDATE secrets
       SET superseded_by = ?1, sealed = NULL, shredded_at = ?2,
           updated_at = ?2
       WHERE id = ?3",
      params![id, now, prev],
    )?;
  }
  meta(conn, &id)
}

/// Read a secret's value (the live head for namespace+name).
/// The caller does the grant check and the witnessing; this is
/// the mechanical unseal.
pub fn read_secret(
  conn: &Connection,
  namespace: &str,
  name: &str,
  key: Option<&[u8; KEY_LEN]>,
) -> Result<Zeroizing<Vec<u8>>, SecretsError> {
  let sealed: Vec<u8> = conn
    .query_row(
      "SELECT sealed FROM secrets
       WHERE namespace = ?1 AND name = ?2
         AND superseded_by IS NULL AND shredded_at IS NULL",
      params![namespace, name],
      |row| row.get(0),
    )
    .map_err(|e| match e {
      rusqlite::Error::QueryReturnedNoRows => {
        SecretsError::SecretNotFound(format!("{namespace}/{name}"))
      }
      other => other.into(),
    })?;
  match key {
    Some(k) => unseal(k, namespace, name, &sealed),
    None => {
      let (&version, rest) =
        sealed.split_first().ok_or(SecretsError::OpenFailure)?;
      if version != ENVELOPE_PLAIN {
        return Err(SecretsError::SealingMismatch(
          "keystore".into(),
          "plaintext".into(),
        ));
      }
      Ok(Zeroizing::new(rest.to_vec()))
    }
  }
}

/// Destroy a secret's material; the row and its history stay.
pub fn shred(
  conn: &Connection,
  namespace: &str,
  name: &str,
) -> Result<SecretMeta, SecretsError> {
  let id: String = conn
    .query_row(
      "SELECT id FROM secrets
       WHERE namespace = ?1 AND name = ?2
         AND superseded_by IS NULL AND shredded_at IS NULL",
      params![namespace, name],
      |row| row.get(0),
    )
    .map_err(|e| match e {
      rusqlite::Error::QueryReturnedNoRows => {
        SecretsError::SecretNotFound(format!("{namespace}/{name}"))
      }
      other => other.into(),
    })?;
  let now = kumbarium_util::now_iso8601();
  conn.execute(
    "UPDATE secrets
     SET sealed = NULL, shredded_at = ?1, updated_at = ?1
     WHERE id = ?2",
    params![now, id],
  )?;
  meta(conn, &id)
}

/// Metadata for one secret by id (never the value, by shape).
pub fn meta(conn: &Connection, id: &str) -> Result<SecretMeta, SecretsError> {
  let mut stmt = conn.prepare(
    "SELECT id, namespace, name, agent_id, superseded_by, note,
            shredded_at, created_at, updated_at
     FROM secrets WHERE id = ?1",
  )?;
  stmt.query_row([id], row_to_meta).map_err(|e| match e {
    rusqlite::Error::QueryReturnedNoRows => {
      SecretsError::IdNotFound(id.to_string())
    }
    other => other.into(),
  })
}

/// Resolve an id fragment, same grammar as every shelf.
pub fn resolve_id(
  conn: &Connection,
  fragment: &str,
) -> Result<String, SecretsError> {
  if kumbarium_util::is_valid_id(fragment) {
    return Ok(fragment.to_string());
  }
  let hexish = fragment.bytes().all(|b| b.is_ascii_hexdigit() || b == b'-');
  if fragment.len() < 4 || !hexish {
    return Err(SecretsError::IdNotFound(fragment.to_string()));
  }
  let mut stmt =
    conn.prepare("SELECT id FROM secrets WHERE id LIKE ?1 LIMIT 2")?;
  let matches = stmt
    .query_map([format!("%{fragment}%")], |row| row.get::<_, String>(0))?
    .collect::<Result<Vec<_>, _>>()?;
  match matches.as_slice() {
    [] => Err(SecretsError::IdNotFound(fragment.to_string())),
    [id] => Ok(id.clone()),
    _ => Err(SecretsError::AmbiguousId(fragment.to_string())),
  }
}

/// Live secrets' metadata, optionally one shelf's.
pub fn list(
  conn: &Connection,
  namespace: Option<&str>,
) -> Result<Vec<SecretMeta>, SecretsError> {
  let mut sql = String::from(
    "SELECT id, namespace, name, agent_id, superseded_by, note,
            shredded_at, created_at, updated_at
     FROM secrets
     WHERE superseded_by IS NULL AND shredded_at IS NULL",
  );
  let mut args: Vec<String> = Vec::new();
  if let Some(ns) = namespace {
    sql.push_str(" AND namespace = ?1");
    args.push(ns.to_string());
  }
  sql.push_str(" ORDER BY namespace, name");
  let mut stmt = conn.prepare(&sql)?;
  let rows = stmt
    .query_map(rusqlite::params_from_iter(args.iter()), row_to_meta)?
    .collect::<Result<Vec<_>, _>>()?;
  Ok(rows)
}

/// A secret's chain, oldest first: the rotation history,
/// values structurally absent.
pub fn history(
  conn: &Connection,
  id: &str,
) -> Result<Vec<SecretMeta>, SecretsError> {
  let mut chain = vec![meta(conn, id)?];
  loop {
    let cur = &chain[0];
    let prev: Option<String> = conn
      .query_row(
        "SELECT id FROM secrets WHERE superseded_by = ?1",
        [&cur.id],
        |row| row.get(0),
      )
      .map(Some)
      .or_else(|e| match e {
        rusqlite::Error::QueryReturnedNoRows => Ok(None),
        other => Err(other),
      })?;
    match prev {
      Some(p) if chain.len() < 1000 => chain.insert(0, meta(conn, &p)?),
      _ => break,
    }
  }
  loop {
    let cur = chain.last().expect("nonempty");
    match &cur.superseded_by {
      Some(next) if chain.len() < 1000 => {
        let next = next.clone();
        chain.push(meta(conn, &next)?);
      }
      _ => break,
    }
  }
  Ok(chain)
}

/// Grant an agent access. Human-only caller; witnessed by them.
pub fn grant(
  conn: &Connection,
  namespace: &str,
  name: &str,
  agent_id: &str,
  expires_at: Option<&str>,
) -> Result<(), SecretsError> {
  conn.execute(
    "INSERT INTO grants
       (namespace, name, agent_id, mode, expires_at, created_at)
     VALUES (?1, ?2, ?3, 'reveal', ?4, ?5)
     ON CONFLICT (namespace, name, agent_id)
     DO UPDATE SET expires_at = ?4",
    params![
      namespace,
      name,
      agent_id,
      expires_at,
      kumbarium_util::now_iso8601()
    ],
  )?;
  Ok(())
}

/// Revoke an agent's access; instantaneous because every read
/// re-checks (no cached token outlives this).
pub fn revoke(
  conn: &Connection,
  namespace: &str,
  name: &str,
  agent_id: &str,
) -> Result<bool, SecretsError> {
  let n = conn.execute(
    "DELETE FROM grants
     WHERE namespace = ?1 AND name = ?2 AND agent_id = ?3",
    params![namespace, name, agent_id],
  )?;
  Ok(n > 0)
}

/// Deny-by-default read-time check: a reveal grant exists for
/// this agent and has not expired.
pub fn check_grant(
  conn: &Connection,
  namespace: &str,
  name: &str,
  agent_id: &str,
) -> Result<bool, SecretsError> {
  let expires: Option<Option<String>> = conn
    .query_row(
      "SELECT expires_at FROM grants
       WHERE namespace = ?1 AND name = ?2 AND agent_id = ?3
         AND mode = 'reveal'",
      params![namespace, name, agent_id],
      |row| row.get(0),
    )
    .map(Some)
    .or_else(|e| match e {
      rusqlite::Error::QueryReturnedNoRows => Ok(None),
      other => Err(other),
    })?;
  Ok(match expires {
    None => false,
    Some(None) => true,
    Some(Some(until)) => kumbarium_util::now_iso8601() < until,
  })
}

/// Every grant on a shelf (or all), for the listing.
pub fn grants(
  conn: &Connection,
  namespace: Option<&str>,
) -> Result<Vec<Grant>, SecretsError> {
  let mut sql = String::from(
    "SELECT namespace, name, agent_id, mode, expires_at,
            created_at
     FROM grants",
  );
  let mut args: Vec<String> = Vec::new();
  if let Some(ns) = namespace {
    sql.push_str(" WHERE namespace = ?1");
    args.push(ns.to_string());
  }
  sql.push_str(" ORDER BY namespace, name, agent_id");
  let mut stmt = conn.prepare(&sql)?;
  let rows = stmt
    .query_map(rusqlite::params_from_iter(args.iter()), |row| {
      Ok(Grant {
        namespace: row.get(0)?,
        name: row.get(1)?,
        agent_id: row.get(2)?,
        mode: row.get(3)?,
        expires_at: row.get(4)?,
        created_at: row.get(5)?,
      })
    })?
    .collect::<Result<Vec<_>, _>>()?;
  Ok(rows)
}

/// (live secrets, grants) counts for `kum status`.
pub fn counts(conn: &Connection) -> Result<(i64, i64), SecretsError> {
  let live: i64 = conn.query_row(
    "SELECT count(*) FROM secrets
     WHERE superseded_by IS NULL AND shredded_at IS NULL",
    [],
    |row| row.get(0),
  )?;
  let granted: i64 =
    conn.query_row("SELECT count(*) FROM grants", [], |row| row.get(0))?;
  Ok((live, granted))
}

fn row_to_meta(row: &rusqlite::Row<'_>) -> Result<SecretMeta, rusqlite::Error> {
  Ok(SecretMeta {
    id: row.get(0)?,
    namespace: row.get(1)?,
    name: row.get(2)?,
    agent_id: row.get(3)?,
    superseded_by: row.get(4)?,
    note: row.get(5)?,
    shredded_at: row.get(6)?,
    created_at: row.get(7)?,
    updated_at: row.get(8)?,
  })
}

#[cfg(test)]
mod tests {
  use super::*;

  const KEY: [u8; KEY_LEN] = [7u8; KEY_LEN];

  #[test]
  fn seal_unseal_round_trips_and_binds_the_shelf() {
    let conn = open_in_memory().unwrap();
    set_secret(
      &conn,
      "project/x",
      "deploy-key",
      b"hunter2-but-long",
      Some(&KEY),
      None,
    )
    .unwrap();
    let value =
      read_secret(&conn, "project/x", "deploy-key", Some(&KEY)).unwrap();
    assert_eq!(&value[..], b"hunter2-but-long");
    // AAD binding: the raw envelope moved to another name must
    // refuse to open.
    let sealed: Vec<u8> = conn
      .query_row(
        "SELECT sealed FROM secrets WHERE name = 'deploy-key'",
        [],
        |r| r.get(0),
      )
      .unwrap();
    conn
      .execute(
        "INSERT INTO secrets
           (id, namespace, name, sealed, agent_id, created_at,
            updated_at)
         VALUES ('stolen-id-0000', 'project/x', 'other-name', ?1,
                 'test', '2026', '2026')",
        [&sealed],
      )
      .unwrap();
    let err = read_secret(&conn, "project/x", "other-name", Some(&KEY));
    assert!(matches!(err, Err(SecretsError::OpenFailure)));
  }

  #[test]
  fn wrong_key_and_unknown_version_fail_closed() {
    let conn = open_in_memory().unwrap();
    set_secret(&conn, "global", "tok", b"value-one", Some(&KEY), None).unwrap();
    let wrong = [9u8; KEY_LEN];
    assert!(matches!(
      read_secret(&conn, "global", "tok", Some(&wrong)),
      Err(SecretsError::OpenFailure)
    ));
    conn
      .execute("UPDATE secrets SET sealed = x'FF00'", [])
      .unwrap();
    assert!(matches!(
      read_secret(&conn, "global", "tok", Some(&KEY)),
      Err(SecretsError::UnknownVersion(255))
    ));
  }

  #[test]
  fn rotation_chains_and_shreds_the_ancestor() {
    let conn = open_in_memory().unwrap();
    let v1 = set_secret(&conn, "global", "api", b"old-value", Some(&KEY), None)
      .unwrap();
    let v2 = set_secret(
      &conn,
      "global",
      "api",
      b"new-value",
      Some(&KEY),
      Some("quarterly rotation"),
    )
    .unwrap();
    let value = read_secret(&conn, "global", "api", Some(&KEY)).unwrap();
    assert_eq!(&value[..], b"new-value");
    // The ancestor's bytes are provably gone.
    let old_sealed: Option<Vec<u8>> = conn
      .query_row("SELECT sealed FROM secrets WHERE id = ?1", [&v1.id], |r| {
        r.get(0)
      })
      .unwrap();
    assert!(old_sealed.is_none(), "shredded at rotation");
    let old = meta(&conn, &v1.id).unwrap();
    assert!(old.shredded_at.is_some());
    assert_eq!(old.superseded_by.as_deref(), Some(v2.id.as_str()));
    let chain = history(&conn, &v1.id).unwrap();
    assert_eq!(chain.len(), 2, "the rotation history keeps its skeleton");
  }

  #[test]
  fn grants_deny_by_default_and_leases_expire() {
    let conn = open_in_memory().unwrap();
    set_secret(&conn, "global", "tok", b"value", Some(&KEY), None).unwrap();
    assert!(!check_grant(&conn, "global", "tok", "claude-code").unwrap());
    grant(&conn, "global", "tok", "claude-code", None).unwrap();
    assert!(check_grant(&conn, "global", "tok", "claude-code").unwrap());
    // A lease in the past denies.
    grant(
      &conn,
      "global",
      "tok",
      "claude-code",
      Some("2020-01-01T00:00:00.000Z"),
    )
    .unwrap();
    assert!(!check_grant(&conn, "global", "tok", "claude-code").unwrap());
    // Revocation is instantaneous by construction.
    grant(&conn, "global", "tok", "claude-code", None).unwrap();
    assert!(revoke(&conn, "global", "tok", "claude-code").unwrap());
    assert!(!check_grant(&conn, "global", "tok", "claude-code").unwrap());
  }

  #[test]
  fn shred_destroys_material_and_keeps_the_record() {
    let conn = open_in_memory().unwrap();
    set_secret(&conn, "global", "doomed", b"value", Some(&KEY), None).unwrap();
    let m = shred(&conn, "global", "doomed").unwrap();
    assert!(m.shredded_at.is_some());
    assert!(matches!(
      read_secret(&conn, "global", "doomed", Some(&KEY)),
      Err(SecretsError::SecretNotFound(_))
    ));
    assert!(list(&conn, None).unwrap().is_empty());
    assert!(meta(&conn, &m.id).is_ok(), "the record survives");
  }

  #[test]
  fn plaintext_mode_is_explicit_and_sticky() {
    let conn = open_in_memory().unwrap();
    set_secret(&conn, "global", "tok", b"value", None, None).unwrap();
    assert_eq!(sealing_mode(&conn).unwrap(), Some(Sealing::Plaintext));
    // The shelf never silently changes sealing mode.
    let err = set_secret(&conn, "global", "tok2", b"v", Some(&KEY), None);
    assert!(matches!(err, Err(SecretsError::SealingMismatch(_, _))));
    let value = read_secret(&conn, "global", "tok", None).unwrap();
    assert_eq!(&value[..], b"value");
  }

  #[test]
  fn nonces_are_fresh_per_seal() {
    let conn = open_in_memory().unwrap();
    set_secret(&conn, "global", "a", b"same-value", Some(&KEY), None).unwrap();
    set_secret(&conn, "global", "b", b"same-value", Some(&KEY), None).unwrap();
    let blobs: Vec<Vec<u8>> = conn
      .prepare("SELECT sealed FROM secrets ORDER BY name")
      .unwrap()
      .query_map([], |r| r.get(0))
      .unwrap()
      .collect::<Result<_, _>>()
      .unwrap();
    assert_ne!(
      blobs[0][1..25],
      blobs[1][1..25],
      "distinct nonces for identical plaintexts"
    );
  }
}
