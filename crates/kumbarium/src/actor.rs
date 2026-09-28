//! Actor resolution (D-056): which minted actor a session is.
//! A serve process is the claimed client working in a workspace
//! (or an actor pinned with --agent); a CLI invocation is the
//! human, identified by git config or the OS login. Resolution
//! mints on first sight; binding (the hashed actor_bind event)
//! is ServerState's job, done lazily before the session's first
//! witnessed event.

use std::path::{Path, PathBuf};

use kumbarium_store::{Actor, ActorKind, NewActor};

/// What a session binds as.
#[derive(Debug, Clone)]
pub enum ActorSource {
  /// Binds nothing: test states (ServerState::in_memory) and
  /// tooling that predates actors.
  #[cfg_attr(not(test), allow(dead_code))]
  Unbound,
  /// A serve process: the claimed client in `workspace`, or the
  /// registered actor named by `requested` (--agent or
  /// KUMBARIUM_AGENT).
  Agent {
    workspace: PathBuf,
    requested: Option<String>,
  },
  /// A CLI invocation: the human, identity per `identity.human`.
  Human { mode: String, cwd: PathBuf },
}

/// One resolution: the actor to bind and how it was reached.
#[derive(Debug, Clone)]
pub struct Resolved {
  pub actor: Actor,
  /// auto | flag | git | os | fixed
  pub via: &'static str,
  pub minted: bool,
  /// A requested actor name that did not resolve (the server
  /// still starts, bound automatically; the bind records it).
  pub refused: Option<String>,
}

/// The unit separator keeps key fields from forging boundaries.
const SEP: char = '\u{1f}';

/// The git toplevel containing `dir`, else `dir` canonicalized.
pub fn workspace_of(dir: &Path) -> PathBuf {
  let top = std::process::Command::new("git")
    .arg("-C")
    .arg(dir)
    .args(["rev-parse", "--show-toplevel"])
    .stdin(std::process::Stdio::null())
    .stderr(std::process::Stdio::null())
    .output()
    .ok()
    .filter(|o| o.status.success())
    .and_then(|o| String::from_utf8(o.stdout).ok())
    .map(|s| s.trim().to_string())
    .filter(|s| !s.is_empty());
  match top {
    Some(t) => PathBuf::from(t),
    None => dir.canonicalize().unwrap_or_else(|_| dir.to_path_buf()),
  }
}

/// A path for display: the home directory as `~`.
pub fn display_path(p: &str) -> String {
  if let Ok(home) = std::env::var("HOME")
    && !home.is_empty()
    && let Some(rest) = p.strip_prefix(&home)
    && (rest.is_empty() || rest.starts_with('/'))
  {
    return format!("~{rest}");
  }
  p.to_string()
}

fn git_config(cwd: &Path, key: &str) -> Option<String> {
  std::process::Command::new("git")
    .arg("-C")
    .arg(cwd)
    .args(["config", "--get", key])
    .stdin(std::process::Stdio::null())
    .stderr(std::process::Stdio::null())
    .output()
    .ok()
    .filter(|o| o.status.success())
    .and_then(|o| String::from_utf8(o.stdout).ok())
    .map(|s| s.trim().to_string())
    .filter(|s| !s.is_empty())
}

fn login_name() -> Option<String> {
  ["USER", "LOGNAME", "USERNAME"]
    .iter()
    .find_map(|v| std::env::var(v).ok())
    .map(|s| s.trim().to_string())
    .filter(|s| !s.is_empty())
}

/// A valid actor name built from `raw`, or `fallback`. Reserved
/// agent words gain a suffix so a human named "list" cannot
/// shadow a lifecycle verb.
fn name_from(raw: &str, fallback: &str) -> String {
  let slug = kumbarium_store::actor_slug(raw);
  let slug = slug.trim_start_matches(|c: char| !c.is_ascii_alphanumeric());
  let mut name: String =
    slug.chars().take(kumbarium_store::MAX_ACTOR_NAME).collect();
  if name.is_empty() {
    name = fallback.to_string();
  }
  if super::tools::reserved_agent_word(&name) {
    name.push_str("-actor");
  }
  name
}

/// The auto-minted agent name: `<claimed>@<workspace leaf>`.
fn agent_name(claimed: &str, workspace: &Path) -> String {
  let client = name_from(claimed, "agent");
  let leaf = workspace
    .file_name()
    .map(|l| kumbarium_store::actor_slug(&l.to_string_lossy()))
    .filter(|l| !l.is_empty());
  let joined = match leaf {
    Some(l) => format!("{client}@{l}"),
    None => client,
  };
  joined
    .chars()
    .take(kumbarium_store::MAX_ACTOR_NAME)
    .collect()
}

fn mint_or_find(
  conn: &kumbarium_store::Connection,
  new: NewActor,
) -> Result<(Actor, bool), String> {
  if let Some(key) = &new.key
    && let Some(found) =
      kumbarium_store::actor_by_key(conn, key).map_err(|e| e.to_string())?
  {
    return Ok((follow_merge(conn, found)?, false));
  }
  let (actor, minted) = kumbarium_store::mint_actor(conn, &new, false)
    .map_err(|e| e.to_string())?;
  Ok((follow_merge(conn, actor)?, minted))
}

/// A merged actor binds as the actor it was merged into.
fn follow_merge(
  conn: &kumbarium_store::Connection,
  actor: Actor,
) -> Result<Actor, String> {
  if actor.merged_into.is_none() {
    return Ok(actor);
  }
  let root =
    kumbarium_store::actor_root(conn, &actor.id).map_err(|e| e.to_string())?;
  kumbarium_store::actor_get(conn, &root).map_err(|e| e.to_string())
}

/// Resolve (minting on first sight) the actor a session binds
/// as. None for `Unbound`.
pub fn resolve(
  conn: &kumbarium_store::Connection,
  source: &ActorSource,
  claimed: &str,
) -> Result<Option<Resolved>, String> {
  match source {
    ActorSource::Unbound => Ok(None),
    ActorSource::Agent {
      workspace,
      requested,
    } => {
      let mut refused = None;
      if let Some(name) = requested {
        match kumbarium_store::resolve_actor(conn, name) {
          Ok(a) => {
            return Ok(Some(Resolved {
              actor: follow_merge(conn, a)?,
              via: "flag",
              minted: false,
              refused: None,
            }));
          }
          Err(_) => refused = Some(name.clone()),
        }
      }
      let ws = workspace.to_string_lossy().to_string();
      let key = format!("agent{SEP}{claimed}{SEP}{ws}");
      let (actor, minted) = mint_or_find(
        conn,
        NewActor {
          name: agent_name(claimed, workspace),
          kind: ActorKind::Agent,
          claimed: claimed.to_string(),
          workspace: ws,
          key: Some(key),
        },
      )?;
      Ok(Some(Resolved {
        actor,
        via: "auto",
        minted,
        refused,
      }))
    }
    ActorSource::Human { mode, cwd } => {
      let (via, key_value, display) = match mode.as_str() {
        "git" | "os" => {
          let git = (mode == "git")
            .then(|| git_config(cwd, "user.email"))
            .flatten();
          match git {
            Some(email) => {
              let name = git_config(cwd, "user.name").unwrap_or_else(|| {
                email.split('@').next().unwrap_or("").into()
              });
              ("git", email, name)
            }
            None => {
              let login = login_name().unwrap_or_else(|| "human".into());
              ("os", login.clone(), login)
            }
          }
        }
        fixed => ("fixed", fixed.to_string(), fixed.to_string()),
      };
      let key = format!("human{SEP}{via}{SEP}{key_value}");
      let (actor, minted) = mint_or_find(
        conn,
        NewActor {
          name: name_from(&display, "human"),
          kind: ActorKind::Human,
          claimed: String::new(),
          workspace: String::new(),
          key: Some(key),
        },
      )?;
      Ok(Some(Resolved {
        actor,
        via,
        minted,
        refused: None,
      }))
    }
  }
}

/// Who acted, for reading the ledger (D-056): a minted actor
/// (merges followed to the survivor), or a claimed name for
/// history from before actors, which is never guessed at.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Who {
  Actor(String),
  Claimed(String),
}

/// The ledger's attribution map: every actor, merge roots, and
/// which actor each session bound.
pub struct ActorIndex {
  pub actors: std::collections::BTreeMap<String, Actor>,
  roots: std::collections::HashMap<String, String>,
  sessions: std::collections::HashMap<String, String>,
}

impl ActorIndex {
  pub fn load(
    library: &kumbarium_store::Connection,
    events: &[kumbarium_audit::StoredEvent],
  ) -> Result<ActorIndex, String> {
    let list = kumbarium_store::actors(library).map_err(|e| e.to_string())?;
    let mut roots = std::collections::HashMap::new();
    for a in &list {
      let root = kumbarium_store::actor_root(library, &a.id)
        .map_err(|e| e.to_string())?;
      roots.insert(a.id.clone(), root);
    }
    let mut sessions = std::collections::HashMap::new();
    for ev in events.iter().filter(|e| e.kind == "actor_bind") {
      let detail: serde_json::Value =
        serde_json::from_str(&ev.detail).unwrap_or_default();
      if let Some(id) = detail.get("actor").and_then(|x| x.as_str()) {
        // The first bind is the session's; a replay never moves it.
        sessions
          .entry(ev.session_id.clone())
          .or_insert_with(|| id.to_string());
      }
    }
    Ok(ActorIndex {
      actors: list.into_iter().map(|a| (a.id.clone(), a)).collect(),
      roots,
      sessions,
    })
  }

  fn root(&self, id: &str) -> String {
    self
      .roots
      .get(id)
      .cloned()
      .unwrap_or_else(|| id.to_string())
  }

  /// The actor a session bound, merges followed.
  pub fn of_session(&self, session: &str) -> Option<String> {
    self.sessions.get(session).map(|id| self.root(id))
  }

  pub fn who_event(&self, ev: &kumbarium_audit::StoredEvent) -> Who {
    match self.of_session(&ev.session_id) {
      Some(id) => Who::Actor(id),
      None => Who::Claimed(ev.agent_id.clone()),
    }
  }

  pub fn who_entry(&self, e: &kumbarium_store::Entry) -> Who {
    match &e.actor_id {
      Some(id) => Who::Actor(self.root(id)),
      None => Who::Claimed(e.agent_id.clone()),
    }
  }

  /// The author a desk judgment names: its actor when the
  /// event recorded one (D-056), else the claimed name.
  pub fn who_submitter(&self, detail: &serde_json::Value) -> Option<Who> {
    if let Some(id) = detail.get("submitter_actor").and_then(|x| x.as_str()) {
      return Some(Who::Actor(self.root(id)));
    }
    detail
      .get("submitter")
      .and_then(|x| x.as_str())
      .map(|n| Who::Claimed(n.to_string()))
  }

  /// A lease's holder: its session's actor, else its claim.
  pub fn who_session(&self, session: &str, claimed: &str) -> Who {
    match self.of_session(session) {
      Some(id) => Who::Actor(id),
      None => Who::Claimed(claimed.to_string()),
    }
  }

  /// What a human typed: an actor (name, id fragment), else a
  /// claimed name taken as-is (pre-actor history).
  pub fn resolve(
    &self,
    library: &kumbarium_store::Connection,
    typed: &str,
  ) -> Who {
    match kumbarium_store::resolve_actor(library, typed) {
      Ok(a) => Who::Actor(self.root(&a.id)),
      Err(_) => Who::Claimed(typed.to_string()),
    }
  }

  pub fn actor(&self, who: &Who) -> Option<&Actor> {
    match who {
      Who::Actor(id) => self.actors.get(id),
      Who::Claimed(_) => None,
    }
  }

  /// The display label: the actor's name, or the claimed name
  /// marked unbound.
  pub fn label(&self, who: &Who) -> String {
    match who {
      Who::Actor(id) => self
        .actors
        .get(id)
        .map(|a| a.name.clone())
        .unwrap_or_else(|| kumbarium_store::short_id(id).to_string()),
      Who::Claimed(name) => format!("{name} (unbound)"),
    }
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  fn conn() -> kumbarium_store::Connection {
    kumbarium_store::open_in_memory().unwrap()
  }

  fn agent(ws: &str, requested: Option<&str>) -> ActorSource {
    ActorSource::Agent {
      workspace: PathBuf::from(ws),
      requested: requested.map(str::to_string),
    }
  }

  #[test]
  fn agents_mint_once_per_client_and_workspace() {
    let c = conn();
    let a = resolve(&c, &agent("/p/kumbarium", None), "claude-code")
      .unwrap()
      .unwrap();
    assert!(a.minted);
    assert_eq!(a.actor.name, "claude-code@kumbarium");
    let again = resolve(&c, &agent("/p/kumbarium", None), "claude-code")
      .unwrap()
      .unwrap();
    assert!(!again.minted);
    assert_eq!(again.actor.id, a.actor.id);
    let other = resolve(&c, &agent("/p/ambyte", None), "claude-code")
      .unwrap()
      .unwrap();
    assert_ne!(other.actor.id, a.actor.id);
    // Same leaf, different path: a second actor, suffixed.
    let clone =
      resolve(&c, &agent("/elsewhere/kumbarium", None), "claude-code")
        .unwrap()
        .unwrap();
    assert_eq!(clone.actor.name, "claude-code@kumbarium-2");
  }

  #[test]
  fn requested_actor_pins_and_unknown_falls_back() {
    let c = conn();
    let (pinned, _) = kumbarium_store::mint_actor(
      &c,
      &NewActor {
        name: "reviewer".into(),
        kind: ActorKind::Agent,
        claimed: String::new(),
        workspace: String::new(),
        key: None,
      },
      true,
    )
    .unwrap();
    let r = resolve(&c, &agent("/p/x", Some("reviewer")), "claude-code")
      .unwrap()
      .unwrap();
    assert_eq!((r.actor.id.as_str(), r.via), (pinned.id.as_str(), "flag"));
    let r = resolve(&c, &agent("/p/x", Some("nobody")), "claude-code")
      .unwrap()
      .unwrap();
    assert_eq!(r.via, "auto");
    assert_eq!(r.refused.as_deref(), Some("nobody"));
  }

  #[test]
  fn merged_actors_bind_as_their_survivor() {
    let c = conn();
    let a = resolve(&c, &agent("/p/a", None), "claude-code")
      .unwrap()
      .unwrap();
    let b = resolve(&c, &agent("/p/b", None), "claude-code")
      .unwrap()
      .unwrap();
    kumbarium_store::merge_actor(&c, &a.actor.id, &b.actor.id).unwrap();
    let again = resolve(&c, &agent("/p/a", None), "claude-code")
      .unwrap()
      .unwrap();
    assert_eq!(again.actor.id, b.actor.id);
  }

  #[test]
  fn fixed_human_identity_and_reserved_names() {
    let c = conn();
    let src = ActorSource::Human {
      mode: "Front Desk".into(),
      cwd: PathBuf::from("/"),
    };
    let h = resolve(&c, &src, "kumbarium-cli").unwrap().unwrap();
    assert_eq!((h.actor.name.as_str(), h.via), ("front-desk", "fixed"));
    assert_eq!(h.actor.kind, ActorKind::Human);
    assert_eq!(name_from("List", "human"), "list-actor");
    assert!(resolve(&c, &ActorSource::Unbound, "x").unwrap().is_none());
  }

  #[test]
  fn display_path_abbreviates_home() {
    let home = std::env::var("HOME").unwrap_or_default();
    if !home.is_empty() {
      assert_eq!(display_path(&format!("{home}/p")), "~/p");
      assert_eq!(display_path(&format!("{home}x/p")), format!("{home}x/p"));
    }
  }
}
