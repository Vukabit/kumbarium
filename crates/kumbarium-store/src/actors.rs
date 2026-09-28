//! The actor registry (D-056): the catalog of minted identities.
//! The ledger stays the authority on who acted (a hashed
//! actor_bind per session); this table only names, merges, and
//! retires. Nothing here rewrites history: a rename changes a
//! display name, a merge records `merged_into`, a retire stamps
//! a time.

use rusqlite::{Connection, OptionalExtension, params};

use super::StoreError;

/// Longest actor name accepted (names are display handles, and
/// they share a column with auto-minted `claimed@workspace`).
pub const MAX_ACTOR_NAME: usize = 64;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ActorKind {
  Agent,
  Human,
}

impl ActorKind {
  pub fn as_str(self) -> &'static str {
    match self {
      ActorKind::Agent => "agent",
      ActorKind::Human => "human",
    }
  }

  pub fn parse(s: &str) -> Option<ActorKind> {
    match s {
      "agent" => Some(ActorKind::Agent),
      "human" => Some(ActorKind::Human),
      _ => None,
    }
  }
}

#[derive(Debug, Clone)]
pub struct Actor {
  pub id: String,
  pub name: String,
  pub kind: ActorKind,
  /// The claimed client name it was minted for ('' for humans
  /// and for actors registered by hand).
  pub claimed: String,
  /// The workspace it was minted for ('' when none).
  pub workspace: String,
  /// The auto-mint key; None for actors registered by name.
  pub key: Option<String>,
  pub created_at: String,
  pub merged_into: Option<String>,
  pub retired_at: Option<String>,
  pub note: Option<String>,
}

/// What minting needs. `name` is the wanted display name; an
/// auto-mint takes a numbered suffix on collision, a registration
/// by hand refuses instead.
#[derive(Debug, Clone)]
pub struct NewActor {
  pub name: String,
  pub kind: ActorKind,
  pub claimed: String,
  pub workspace: String,
  pub key: Option<String>,
}

const COLUMNS: &str = "id, name, kind, claimed, workspace, key, created_at, \
                       merged_into, retired_at, note";

fn row_to_actor(row: &rusqlite::Row<'_>) -> Result<Actor, rusqlite::Error> {
  let kind_raw: String = row.get(2)?;
  let kind = ActorKind::parse(&kind_raw).ok_or_else(|| {
    rusqlite::Error::FromSqlConversionFailure(
      2,
      rusqlite::types::Type::Text,
      format!("unknown actor kind {kind_raw:?}").into(),
    )
  })?;
  Ok(Actor {
    id: row.get(0)?,
    name: row.get(1)?,
    kind,
    claimed: row.get(3)?,
    workspace: row.get(4)?,
    key: row.get(5)?,
    created_at: row.get(6)?,
    merged_into: row.get(7)?,
    retired_at: row.get(8)?,
    note: row.get(9)?,
  })
}

/// A display-safe slug: lowercase, `[a-z0-9._@-]`, everything
/// else folded to `-`, runs collapsed, ends trimmed.
pub fn actor_slug(raw: &str) -> String {
  let mut out = String::new();
  for c in raw.chars().flat_map(char::to_lowercase) {
    let keep = c.is_ascii_lowercase()
      || c.is_ascii_digit()
      || matches!(c, '.' | '_' | '@' | '-');
    let c = if keep { c } else { '-' };
    if c == '-' && out.ends_with('-') {
      continue;
    }
    out.push(c);
  }
  out.trim_matches(['-', '.']).to_string()
}

/// Actor names: 1..=64 chars of `[a-z0-9._@-]`, starting with a
/// letter or digit (so a name can never read as a flag).
pub fn validate_actor_name(name: &str) -> Result<(), StoreError> {
  let bad = |why: &'static str| {
    Err(StoreError::InvalidActorName(name.to_string(), why))
  };
  if name.is_empty() {
    return bad("empty");
  }
  if name.len() > MAX_ACTOR_NAME {
    return bad("longer than 64 characters");
  }
  if !name
    .bytes()
    .next()
    .is_some_and(|b| b.is_ascii_alphanumeric())
  {
    return bad("must start with a letter or digit");
  }
  let ok = name.bytes().all(|b| {
    b.is_ascii_lowercase()
      || b.is_ascii_digit()
      || matches!(b, b'.' | b'_' | b'@' | b'-')
  });
  if !ok {
    return bad("only lowercase letters, digits, and . _ @ - are allowed");
  }
  Ok(())
}

/// Every actor, by name.
pub fn actors(conn: &Connection) -> Result<Vec<Actor>, StoreError> {
  let mut stmt =
    conn.prepare(&format!("SELECT {COLUMNS} FROM actors ORDER BY name"))?;
  let rows = stmt
    .query_map([], row_to_actor)?
    .collect::<Result<Vec<_>, _>>()?;
  Ok(rows)
}

pub fn actor_by_key(
  conn: &Connection,
  key: &str,
) -> Result<Option<Actor>, StoreError> {
  Ok(
    conn
      .query_row(
        &format!("SELECT {COLUMNS} FROM actors WHERE key = ?1"),
        [key],
        row_to_actor,
      )
      .optional()?,
  )
}

pub fn actor_get(conn: &Connection, id: &str) -> Result<Actor, StoreError> {
  conn
    .query_row(
      &format!("SELECT {COLUMNS} FROM actors WHERE id = ?1"),
      [id],
      row_to_actor,
    )
    .optional()?
    .ok_or_else(|| StoreError::ActorNotFound(id.to_string()))
}

/// Resolve what a human typed: an exact name, a full id, or a
/// unique id fragment of at least 4 hex chars (the short form
/// is the last 8). Ambiguity is an error, never a guess.
pub fn resolve_actor(
  conn: &Connection,
  frag: &str,
) -> Result<Actor, StoreError> {
  let by_name = conn
    .query_row(
      &format!("SELECT {COLUMNS} FROM actors WHERE name = ?1"),
      [frag],
      row_to_actor,
    )
    .optional()?;
  if let Some(a) = by_name {
    return Ok(a);
  }
  let hexish = frag.bytes().all(|b| b.is_ascii_hexdigit() || b == b'-');
  if frag.len() < 4 || !hexish {
    return Err(StoreError::ActorNotFound(frag.to_string()));
  }
  let mut stmt = conn.prepare(&format!(
    "SELECT {COLUMNS} FROM actors WHERE id LIKE ?1 LIMIT 2"
  ))?;
  let hits = stmt
    .query_map([format!("%{frag}%")], row_to_actor)?
    .collect::<Result<Vec<_>, _>>()?;
  match hits.as_slice() {
    [] => Err(StoreError::ActorNotFound(frag.to_string())),
    [a] => Ok(a.clone()),
    _ => Err(StoreError::AmbiguousActor(frag.to_string())),
  }
}

/// Mint an actor: (the actor, whether this call created it).
/// With `exact`, a taken name is refused; without it (auto-mint),
/// the name takes `-2`, `-3`... A concurrent mint of the same
/// key (two sessions opening at once) returns the winner's row,
/// created = false, instead of failing.
pub fn mint_actor(
  conn: &Connection,
  new: &NewActor,
  exact: bool,
) -> Result<(Actor, bool), StoreError> {
  validate_actor_name(&new.name)?;
  if let Some(key) = &new.key
    && let Some(existing) = actor_by_key(conn, key)?
  {
    return Ok((existing, false));
  }
  let id = kumbarium_util::generate_id();
  let now = kumbarium_util::now_iso8601();
  for n in 1..1000 {
    let name = if n == 1 {
      new.name.clone()
    } else {
      let suffix = format!("-{n}");
      let room = MAX_ACTOR_NAME - suffix.len();
      let base: String = new.name.chars().take(room).collect();
      format!("{base}{suffix}")
    };
    let inserted = conn.execute(
      "INSERT INTO actors (id, name, kind, claimed, workspace, key, created_at)
       VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
      params![
        id,
        name,
        new.kind.as_str(),
        new.claimed,
        new.workspace,
        new.key,
        now
      ],
    );
    match inserted {
      Ok(_) => return Ok((actor_get(conn, &id)?, true)),
      Err(rusqlite::Error::SqliteFailure(e, Some(msg)))
        if e.code == rusqlite::ErrorCode::ConstraintViolation =>
      {
        if msg.contains("actors.key")
          && let Some(key) = &new.key
          && let Some(existing) = actor_by_key(conn, key)?
        {
          return Ok((existing, false));
        }
        if msg.contains("actors.name") {
          if exact {
            return Err(StoreError::ActorNameTaken(name));
          }
          continue;
        }
        return Err(rusqlite::Error::SqliteFailure(e, Some(msg)).into());
      }
      Err(other) => return Err(other.into()),
    }
  }
  Err(StoreError::ActorNameTaken(new.name.clone()))
}

pub fn rename_actor(
  conn: &Connection,
  id: &str,
  name: &str,
) -> Result<(), StoreError> {
  validate_actor_name(name)?;
  actor_get(conn, id)?;
  match conn.execute(
    "UPDATE actors SET name = ?1 WHERE id = ?2",
    params![name, id],
  ) {
    Ok(_) => Ok(()),
    Err(rusqlite::Error::SqliteFailure(e, _))
      if e.code == rusqlite::ErrorCode::ConstraintViolation =>
    {
      Err(StoreError::ActorNameTaken(name.to_string()))
    }
    Err(other) => Err(other.into()),
  }
}

/// The surviving actor an id reads as: follow `merged_into`
/// (bounded, so a corrupted cycle cannot hang a read).
pub fn actor_root(conn: &Connection, id: &str) -> Result<String, StoreError> {
  let mut current = id.to_string();
  for _ in 0..64 {
    let next: Option<String> = conn
      .query_row(
        "SELECT merged_into FROM actors WHERE id = ?1",
        [&current],
        |row| row.get(0),
      )
      .optional()?
      .flatten();
    match next {
      Some(n) => current = n,
      None => return Ok(current),
    }
  }
  Ok(current)
}

/// Fold `from` into `into`. Refused when they are the same, when
/// `from` is already merged, or when `into` already reads as
/// `from` (a cycle). The surviving id is `into`'s root.
pub fn merge_actor(
  conn: &Connection,
  from: &str,
  into: &str,
) -> Result<String, StoreError> {
  let source = actor_get(conn, from)?;
  actor_get(conn, into)?;
  if let Some(already) = &source.merged_into {
    return Err(StoreError::ActorMerge(format!(
      "{} is already merged into {already}",
      source.name
    )));
  }
  let target = actor_root(conn, into)?;
  if target == from {
    return Err(StoreError::ActorMerge(
      "an actor cannot be merged into itself".into(),
    ));
  }
  conn.execute(
    "UPDATE actors SET merged_into = ?1 WHERE id = ?2",
    params![target, from],
  )?;
  Ok(target)
}

/// Retire (Some(now)) or unretire (None). Idempotent.
pub fn set_actor_retired(
  conn: &Connection,
  id: &str,
  retired: bool,
) -> Result<(), StoreError> {
  actor_get(conn, id)?;
  let at = retired.then(kumbarium_util::now_iso8601);
  conn.execute(
    "UPDATE actors SET retired_at = ?1 WHERE id = ?2",
    params![at, id],
  )?;
  Ok(())
}

#[cfg(test)]
mod tests {
  use super::*;

  fn new(name: &str, key: Option<&str>) -> NewActor {
    NewActor {
      name: name.into(),
      kind: ActorKind::Agent,
      claimed: "claude-code".into(),
      workspace: "/w".into(),
      key: key.map(str::to_string),
    }
  }

  #[test]
  fn mint_is_stable_per_key_and_suffixes_names() {
    let conn = crate::open_in_memory().unwrap();
    let (a, created) =
      mint_actor(&conn, &new("claude-code@w", Some("k1")), false).unwrap();
    assert!(created);
    let (again, created) =
      mint_actor(&conn, &new("claude-code@w", Some("k1")), false).unwrap();
    assert_eq!((a.id.as_str(), created), (again.id.as_str(), false));
    let (b, _) =
      mint_actor(&conn, &new("claude-code@w", Some("k2")), false).unwrap();
    assert_eq!(b.name, "claude-code@w-2");
    let err = mint_actor(&conn, &new("claude-code@w", None), true).unwrap_err();
    assert!(matches!(err, StoreError::ActorNameTaken(_)));
  }

  #[test]
  fn resolve_by_name_and_fragment() {
    let conn = crate::open_in_memory().unwrap();
    let a = mint_actor(&conn, &new("alpha", None), true).unwrap().0;
    assert_eq!(resolve_actor(&conn, "alpha").unwrap().id, a.id);
    let short = &a.id[a.id.len() - 8..];
    assert_eq!(resolve_actor(&conn, short).unwrap().id, a.id);
    assert!(resolve_actor(&conn, "nobody").is_err());
  }

  #[test]
  fn merge_follows_and_refuses_cycles() {
    let conn = crate::open_in_memory().unwrap();
    let a = mint_actor(&conn, &new("a", None), true).unwrap().0;
    let b = mint_actor(&conn, &new("b", None), true).unwrap().0;
    let c = mint_actor(&conn, &new("c", None), true).unwrap().0;
    merge_actor(&conn, &a.id, &b.id).unwrap();
    assert_eq!(actor_root(&conn, &a.id).unwrap(), b.id);
    merge_actor(&conn, &b.id, &c.id).unwrap();
    assert_eq!(actor_root(&conn, &a.id).unwrap(), c.id);
    assert!(merge_actor(&conn, &c.id, &a.id).is_err(), "cycle");
    assert!(merge_actor(&conn, &a.id, &c.id).is_err(), "already merged");
  }

  #[test]
  fn names_are_validated_and_slugged() {
    assert!(validate_actor_name("claude-code@kumbarium").is_ok());
    assert!(validate_actor_name("-flag").is_err());
    assert!(validate_actor_name("Upper").is_err());
    assert_eq!(actor_slug("Shawn Bays"), "shawn-bays");
    assert_eq!(actor_slug("--Ngomia!!"), "ngomia");
  }

  #[test]
  fn rename_and_retire() {
    let conn = crate::open_in_memory().unwrap();
    let a = mint_actor(&conn, &new("a", None), true).unwrap().0;
    mint_actor(&conn, &new("b", None), true).unwrap();
    assert!(matches!(
      rename_actor(&conn, &a.id, "b"),
      Err(StoreError::ActorNameTaken(_))
    ));
    rename_actor(&conn, &a.id, "renamed").unwrap();
    set_actor_retired(&conn, &a.id, true).unwrap();
    let got = actor_get(&conn, &a.id).unwrap();
    assert_eq!(got.name, "renamed");
    assert!(got.retired_at.is_some());
  }
}
