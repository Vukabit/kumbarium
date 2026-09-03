//! Typed edges between entries (migration 0002). See that
//! migration's header for the relation vocabulary and why
//! supersession is NOT an edge.

use rusqlite::{Connection, params};

use crate::StoreError;

/// Relation kinds. Mirrors the CHECK constraint.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Rel {
  Continues,
  RelatesTo,
  Duplicates,
  Contradicts,
}

impl Rel {
  pub fn as_str(self) -> &'static str {
    match self {
      Rel::Continues => "continues",
      Rel::RelatesTo => "relates_to",
      Rel::Duplicates => "duplicates",
      Rel::Contradicts => "contradicts",
    }
  }

  pub fn parse(s: &str) -> Option<Rel> {
    match s {
      "continues" => Some(Rel::Continues),
      "relates_to" => Some(Rel::RelatesTo),
      "duplicates" => Some(Rel::Duplicates),
      "contradicts" => Some(Rel::Contradicts),
      _ => None,
    }
  }
}

/// One edge, as stored.
#[derive(Debug, Clone)]
pub struct Link {
  pub from_id: String,
  pub to_id: String,
  pub rel: Rel,
}

/// Create an edge. Idempotent: linking twice is Ok. Both
/// endpoints must exist; self-links are refused.
pub fn link(
  conn: &Connection,
  from_id: &str,
  to_id: &str,
  rel: Rel,
) -> Result<(), StoreError> {
  if from_id == to_id {
    return Err(StoreError::SelfLink(from_id.to_string()));
  }
  for id in [from_id, to_id] {
    let exists: bool = conn.query_row(
      "SELECT EXISTS(SELECT 1 FROM entries WHERE id = ?1)",
      [id],
      |row| row.get(0),
    )?;
    if !exists {
      return Err(StoreError::EntryNotFound(id.to_string()));
    }
  }
  conn.execute(
    "INSERT OR IGNORE INTO entry_links
       (from_id, to_id, rel, created_at)
     VALUES (?1, ?2, ?3, ?4)",
    params![from_id, to_id, rel.as_str(), kumbarium_util::now_iso8601()],
  )?;
  Ok(())
}

/// Remove an edge; Ok even if it was not there.
pub fn unlink(
  conn: &Connection,
  from_id: &str,
  to_id: &str,
  rel: Rel,
) -> Result<(), StoreError> {
  conn.execute(
    "DELETE FROM entry_links
     WHERE from_id = ?1 AND to_id = ?2 AND rel = ?3",
    params![from_id, to_id, rel.as_str()],
  )?;
  Ok(())
}

/// The ordered `continues` chain containing `id` (a split
/// memory's set), earliest part first; `id` itself is always a
/// member. Cycle-proof via a visited set. Auto-split chains are
/// linear by construction, but hand-made `continues` edges may
/// branch: mint order wins (UUIDv7 ids sort chronologically)
/// and the returned flag is true so callers can SAY the chain
/// branched instead of guessing silently.
pub fn continues_chain(
  conn: &Connection,
  id: &str,
) -> Result<(Vec<String>, bool), StoreError> {
  let mut branched = false;
  let mut visited: std::collections::HashSet<String> = [id.to_string()].into();

  let mut walk = |start: &str, sql: &str| -> Result<Vec<String>, StoreError> {
    let mut out = Vec::new();
    let mut current = start.to_string();
    loop {
      let mut stmt = conn.prepare(sql)?;
      let nexts = stmt
        .query_map([&current], |row| row.get::<_, String>(0))?
        .collect::<Result<Vec<_>, _>>()?;
      let Some(next) = nexts.first().cloned() else {
        break;
      };
      if nexts.len() > 1 {
        branched = true;
      }
      if !visited.insert(next.clone()) {
        break; // cycle
      }
      out.push(next.clone());
      current = next;
    }
    Ok(out)
  };

  // Backward: my outgoing continues edges point at earlier
  // parts. Forward: edges pointing at me come from later parts.
  let mut earlier = walk(
    id,
    "SELECT to_id FROM entry_links
     WHERE from_id = ?1 AND rel = 'continues' ORDER BY to_id",
  )?;
  let later = walk(
    id,
    "SELECT from_id FROM entry_links
     WHERE to_id = ?1 AND rel = 'continues' ORDER BY from_id",
  )?;
  earlier.reverse();
  earlier.push(id.to_string());
  earlier.extend(later);
  Ok((earlier, branched))
}

/// Every edge touching `id`, in either direction.
pub fn links_of(conn: &Connection, id: &str) -> Result<Vec<Link>, StoreError> {
  let mut stmt = conn.prepare(
    "SELECT from_id, to_id, rel FROM entry_links
     WHERE from_id = ?1 OR to_id = ?1
     ORDER BY rel, from_id, to_id",
  )?;
  let rows = stmt
    .query_map([id], |row| {
      Ok((
        row.get::<_, String>(0)?,
        row.get::<_, String>(1)?,
        row.get::<_, String>(2)?,
      ))
    })?
    .collect::<Result<Vec<_>, _>>()?;
  let mut links = Vec::new();
  for (from_id, to_id, raw) in rows {
    let rel = Rel::parse(&raw).ok_or_else(|| {
      StoreError::Sqlite(rusqlite::Error::IntegralValueOutOfRange(2, 0))
    })?;
    links.push(Link {
      from_id,
      to_id,
      rel,
    });
  }
  Ok(links)
}

#[cfg(test)]
mod tests {
  use super::*;
  use crate::{Kind, NewEntry, remember};

  fn seeded() -> (Connection, String, String) {
    let mut conn = crate::open_in_memory().unwrap();
    let mk = |conn: &mut Connection, content: &str| {
      remember(
        conn,
        &NewEntry {
          namespace: "global".into(),
          kind: Kind::Reference,
          content: content.into(),
          agent_id: "test".into(),
          source: "".into(),
          tags: vec![],
        },
      )
      .unwrap()
      .id
    };
    let a = mk(&mut conn, "part one of the design");
    let b = mk(&mut conn, "part two of the design");
    (conn, a, b)
  }

  #[test]
  fn link_round_trips_both_directions() {
    let (conn, a, b) = seeded();
    link(&conn, &a, &b, Rel::Continues).unwrap();
    let from_a = links_of(&conn, &a).unwrap();
    let from_b = links_of(&conn, &b).unwrap();
    assert_eq!(from_a.len(), 1);
    assert_eq!(from_b.len(), 1);
    assert_eq!(from_a[0].rel, Rel::Continues);
    assert_eq!(from_a[0].to_id, b);
    assert_eq!(from_b[0].from_id, a);
  }

  #[test]
  fn linking_twice_is_idempotent() {
    let (conn, a, b) = seeded();
    link(&conn, &a, &b, Rel::RelatesTo).unwrap();
    link(&conn, &a, &b, Rel::RelatesTo).unwrap();
    assert_eq!(links_of(&conn, &a).unwrap().len(), 1);
  }

  #[test]
  fn self_and_dangling_links_are_refused() {
    let (conn, a, _) = seeded();
    assert!(matches!(
      link(&conn, &a, &a, Rel::RelatesTo),
      Err(StoreError::SelfLink(_))
    ));
    assert!(matches!(
      link(&conn, &a, "missing-id", Rel::RelatesTo),
      Err(StoreError::EntryNotFound(_))
    ));
  }

  #[test]
  fn unlink_removes_only_the_named_edge() {
    let (conn, a, b) = seeded();
    link(&conn, &a, &b, Rel::Continues).unwrap();
    link(&conn, &a, &b, Rel::RelatesTo).unwrap();
    unlink(&conn, &a, &b, Rel::Continues).unwrap();
    let left = links_of(&conn, &a).unwrap();
    assert_eq!(left.len(), 1);
    assert_eq!(left[0].rel, Rel::RelatesTo);
  }

  #[test]
  fn chain_reconstructs_from_any_member() {
    let (mut conn, a, b) = seeded();
    let c = remember(
      &mut conn,
      &NewEntry {
        namespace: "global".into(),
        kind: Kind::Reference,
        content: "part three of the design".into(),
        agent_id: "test".into(),
        source: "".into(),
        tags: vec![],
      },
    )
    .unwrap()
    .id;
    // Auto-split direction: later part points at its predecessor.
    link(&conn, &b, &a, Rel::Continues).unwrap();
    link(&conn, &c, &b, Rel::Continues).unwrap();
    for member in [&a, &b, &c] {
      let (chain, branched) = continues_chain(&conn, member).unwrap();
      assert_eq!(chain, [a.clone(), b.clone(), c.clone()]);
      assert!(!branched);
    }
    // A lone entry is a chain of one.
    let (solo, _) = continues_chain(&conn, "not-linked").unwrap();
    assert_eq!(solo, ["not-linked"]);
  }

  #[test]
  fn branched_chains_flag_and_take_mint_order() {
    let (mut conn, a, b) = seeded();
    let c = remember(
      &mut conn,
      &NewEntry {
        namespace: "global".into(),
        kind: Kind::Reference,
        content: "a rival continuation".into(),
        agent_id: "test".into(),
        source: "".into(),
        tags: vec![],
      },
    )
    .unwrap()
    .id;
    // Both b and c claim to continue a: a branch.
    link(&conn, &b, &a, Rel::Continues).unwrap();
    link(&conn, &c, &a, Rel::Continues).unwrap();
    let (chain, branched) = continues_chain(&conn, &a).unwrap();
    assert!(branched);
    // Mint order: b was created before c, so b wins the walk.
    assert_eq!(chain, [a.clone(), b.clone()]);
  }

  #[test]
  fn cyclic_chains_terminate() {
    let (conn, a, b) = seeded();
    link(&conn, &b, &a, Rel::Continues).unwrap();
    link(&conn, &a, &b, Rel::Continues).unwrap();
    let (chain, _) = continues_chain(&conn, &a).unwrap();
    assert!(chain.len() <= 2, "cycle terminated: {chain:?}");
    assert!(chain.contains(&a));
  }

  #[test]
  fn supersession_rewires_set_membership() {
    let (mut conn, a, b) = seeded();
    // b continues a; then supersede b: the replacement must take
    // b's place in the set, and b must keep no edges.
    link(&conn, &b, &a, Rel::Continues).unwrap();
    let b2 = crate::supersede(
      &mut conn,
      &b,
      &NewEntry {
        namespace: "global".into(),
        kind: Kind::Reference,
        content: "part two, corrected".into(),
        agent_id: "test".into(),
        source: "".into(),
        tags: vec![],
      },
      Some("typo fix"),
    )
    .unwrap()
    .id;
    let (chain, branched) = continues_chain(&conn, &a).unwrap();
    assert_eq!(chain, [a.clone(), b2.clone()]);
    assert!(!branched);
    assert!(
      links_of(&conn, &b).unwrap().is_empty(),
      "superseded version keeps no edges"
    );
  }

  #[test]
  fn forget_cleans_up_edges() {
    let (mut conn, a, b) = seeded();
    link(&conn, &a, &b, Rel::Continues).unwrap();
    crate::forget(&mut conn, &b).unwrap();
    assert!(links_of(&conn, &a).unwrap().is_empty());
  }
}
