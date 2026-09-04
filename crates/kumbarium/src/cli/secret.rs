//! The restricted stacks' human surfaces (D-038): stock, read,
//! copy, grant, revoke, shred, and the listing. Values travel
//! through stdin and the clipboard, never argv (shell history
//! is a ledger too) and never the audit ledger (value-free by
//! shape). Every verb is witnessed before the value moves.

use std::io::IsTerminal as _;
use std::io::Write as _;
use std::process::ExitCode;

use zeroize::Zeroizing;

use super::super::{keystore, open_stores, style, tools};
use super::term::*;
use super::usage::paint_cli_page;

pub(crate) const SECRETS_USAGE: &str = "\
the restricted stacks: witnessed credential custody

  kumbarium secret set <ns> <name>     stock or rotate; value
       [--i-accept-plaintext]          from stdin or an echo-off
       [--expires DATE]                prompt, never argv
                                       (--expires: upstream
                                       expiry, surfaced never
                                       enforced)
  kumbarium secret read <ns> <name>    print the value
  kumbarium secret copy <ns> <name>    concealed clipboard copy,
                                       auto-clear in 90s
  kumbarium secret grant <ns> <name> <agent> [--until DATE]
                                       allow secret_read; the
                                       lease expires at read
                                       time, through DATE (UTC)
  kumbarium secret revoke <ns> <name> <agent>  withdraw it
  kumbarium secret shred <ns> <name>   destroy the value, keep
                                       the record
  kumbarium secrets [ns]               names, grants, sealing;
                                       never values";

pub(crate) fn secret_cmd(rest: &[&str]) -> ExitCode {
  match rest {
    ["set", ns, name, flags @ ..] => set_cmd(ns, name, flags),
    ["read", ns, name] => read_cmd(ns, name),
    ["copy", ns, name] => copy_cmd(ns, name),
    ["grant", ns, name, agent, flags @ ..] => grant_cmd(ns, name, agent, flags),
    ["revoke", ns, name, agent] => revoke_cmd(ns, name, agent),
    ["shred", ns, name] => shred_cmd(ns, name),
    [] => {
      let sty = style::Style::detect();
      println!("{}", paint_cli_page(SECRETS_USAGE, &sty));
      ExitCode::SUCCESS
    }
    _ => fail("unrecognized secret command; the map: kum secret"),
  }
}

/// Namespace gate shared by every verb: normalized, valid, and
/// registered (secrets live on real shelves only).
fn checked_namespace(
  state: &tools::ServerState,
  ns: &str,
) -> Result<String, String> {
  let ns = kumbarium_librarian::normalize_namespace(ns);
  kumbarium_librarian::validate_namespace(&ns)
    .map_err(|e| format!("invalid namespace: {e}"))?;
  match kumbarium_store::namespace_id(&state.library, &ns) {
    Ok(Some(_)) => Ok(ns),
    Ok(None) => Err(format!(
      "namespace {ns:?} is not registered; kumbarium namespace \
       add {ns}"
    )),
    Err(e) => Err(e.to_string()),
  }
}

/// The write-side key decision. Sticky per shelf: once sealed,
/// always sealed (a missing keystore then REFUSES rather than
/// downgrading); once plaintext, it stays plaintext and says
/// so. First use prefers the keystore and falls back to
/// plaintext only on the explicit flag.
fn key_for_set(
  state: &mut tools::ServerState,
  accept_plaintext: bool,
) -> Result<Option<[u8; kumbarium_secrets::KEY_LEN]>, String> {
  let mode = kumbarium_secrets::sealing_mode(state.secrets()?)
    .map_err(|e| e.to_string())?;
  if mode == Some(kumbarium_secrets::Sealing::Plaintext) {
    eprintln!(
      "kumbarium: this shelf is plaintext-mode (chosen at first \
       use); values rest unsealed"
    );
    return Ok(None);
  }
  match keystore::master_key() {
    keystore::Keystore::Present(key) => Ok(Some(key)),
    keystore::Keystore::Blocked(why) => Err(format!(
      "keystore blocked ({why}); refusing rather than silently \
       downgrading"
    )),
    keystore::Keystore::Absent => match mode {
      Some(_) => Err(
        "this shelf is keystore-sealed but no keystore substrate \
         is reachable; refusing rather than downgrading"
          .into(),
      ),
      None if accept_plaintext => Ok(None),
      None => Err(
        "no platform keystore substrate here. To store secrets \
         UNSEALED anyway, repeat with --i-accept-plaintext (a \
         deliberate, permanent choice for this shelf)"
          .into(),
      ),
    },
  }
}

/// Read the value: piped stdin whole, or an echo-off prompt on
/// a terminal (unix termios; elsewhere, pipe it).
fn read_value() -> Result<Zeroizing<Vec<u8>>, String> {
  if !std::io::stdin().is_terminal() {
    use std::io::Read as _;
    let mut buf = Zeroizing::new(Vec::new());
    std::io::stdin()
      .read_to_end(&mut buf)
      .map_err(|e| format!("reading stdin: {e}"))?;
    while buf.last().is_some_and(|b| *b == b'\n' || *b == b'\r') {
      buf.pop();
    }
    return Ok(buf);
  }
  prompt_echo_off()
}

#[cfg(unix)]
fn prompt_echo_off() -> Result<Zeroizing<Vec<u8>>, String> {
  eprint!("value (echo off): ");
  let _ = std::io::stderr().flush();
  let mut term = unsafe { std::mem::zeroed::<libc::termios>() };
  if unsafe { libc::tcgetattr(0, &mut term) } != 0 {
    return Err("terminal attributes unavailable; pipe the value".into());
  }
  let saved = term;
  term.c_lflag &= !libc::ECHO;
  unsafe { libc::tcsetattr(0, libc::TCSANOW, &term) };
  let mut line = Zeroizing::new(String::new());
  let res = std::io::stdin().read_line(&mut line);
  unsafe { libc::tcsetattr(0, libc::TCSANOW, &saved) };
  eprintln!();
  res.map_err(|e| format!("reading value: {e}"))?;
  let trimmed = line.trim_end_matches(['\n', '\r']);
  Ok(Zeroizing::new(trimmed.as_bytes().to_vec()))
}

#[cfg(not(unix))]
fn prompt_echo_off() -> Result<Zeroizing<Vec<u8>>, String> {
  // No termios here; a visible prompt would echo the value
  // into the console. Piping is the honest path.
  Err(
    "echo-off prompt is wired for unix today; pipe the value \
       on stdin instead"
      .into(),
  )
}

fn witness(
  state: &tools::ServerState,
  kind: kumbarium_audit::EventKind,
  scope: &str,
  detail: serde_json::Value,
) -> Result<(), String> {
  let event = kumbarium_audit::Event {
    agent_id: "kumbarium-cli".into(),
    kind,
    scope: scope.into(),
    detail,
  };
  kumbarium_audit::append(&state.audit, &event)
    .map(|_| ())
    .map_err(|e| format!("audit append failed: {e}"))
}

/// A calendar day, the same grammar as docket goals.
fn valid_date(date: &str) -> Result<(), String> {
  let ok = date.len() == 10
    && kumbarium_util::parse_iso8601_ms(&format!("{date}T00:00:00.000Z"))
      .is_some();
  match ok {
    true => Ok(()),
    false => Err(format!("invalid date {date:?}; use YYYY-MM-DD")),
  }
}

fn set_cmd(ns: &str, name: &str, flags: &[&str]) -> ExitCode {
  let mut accept_plaintext = false;
  let mut expires: Option<String> = None;
  let mut it = flags.iter();
  while let Some(flag) = it.next() {
    match *flag {
      "--i-accept-plaintext" => accept_plaintext = true,
      "--expires" => match it.next() {
        Some(date) => {
          if let Err(e) = valid_date(date) {
            return fail(&e);
          }
          expires = Some((*date).to_string());
        }
        None => return fail("--expires needs YYYY-MM-DD"),
      },
      other => return fail(&format!("unknown flag {other:?}")),
    }
  }
  let (_, mut state) = match open_stores() {
    Ok(v) => v,
    Err(e) => return fail(&e),
  };
  let ns = match checked_namespace(&state, ns) {
    Ok(ns) => ns,
    Err(e) => return fail(&e),
  };
  let key = match key_for_set(&mut state, accept_plaintext) {
    Ok(k) => k,
    Err(e) => return fail(&e),
  };
  let value = match read_value() {
    Ok(v) if v.is_empty() => return fail("empty value; nothing stocked"),
    Ok(v) => v,
    Err(e) => return fail(&e),
  };
  let conn = match state.secrets() {
    Ok(c) => c,
    Err(e) => return fail(&e),
  };
  let rotating = kumbarium_secrets::list(conn, Some(&ns))
    .map(|rows| rows.iter().any(|m| m.name == name))
    .unwrap_or(false);
  let meta = match kumbarium_secrets::set_secret(
    conn,
    &ns,
    name,
    &value,
    key.as_ref(),
    None,
    expires.as_deref(),
  ) {
    Ok(m) => m,
    Err(e) => return fail(&e.to_string()),
  };
  if let Err(e) = witness(
    &state,
    kumbarium_audit::EventKind::SecretSet,
    &ns,
    serde_json::json!({ "name": name, "id": meta.id }),
  ) {
    return fail(&format!("stocked, but {e}"));
  }
  let sty = style::Style::detect();
  let sealed = if key.is_some() { "sealed" } else { "PLAINTEXT" };
  if rotating {
    println!(
      "rotated {ns}/{name} ({}, {sealed}); the retired value is \
       shredded, its record remains",
      sty.id(kumbarium_secrets::short_id(&meta.id))
    );
  } else {
    println!(
      "stocked {ns}/{name} ({}, {sealed}); agents need a grant \
       to read it",
      sty.id(kumbarium_secrets::short_id(&meta.id))
    );
  }
  if let Some(date) = &expires {
    println!(
      "expiry {date} recorded (metadata; the broker never \
       enforces it). The docket can do the reminding: kum task \
       {ns} \"rotate {name}\" --goal {date}"
    );
  }
  ExitCode::SUCCESS
}

fn read_cmd(ns: &str, name: &str) -> ExitCode {
  let (_, mut state) = match open_stores() {
    Ok(v) => v,
    Err(e) => return fail(&e),
  };
  let ns = kumbarium_librarian::normalize_namespace(ns);
  if let Err(e) = kumbarium_librarian::validate_namespace(&ns) {
    return fail(&format!("invalid namespace: {e}"));
  }
  let key = match tools::secrets_key(&mut state) {
    Ok(k) => k,
    Err(e) => return fail(&e),
  };
  // Witness BEFORE the value moves (fail-closed, D-038). The
  // CLI is the human's own hands: no grant gate, but the read
  // is on the ledger like anyone else's.
  if let Err(e) = witness(
    &state,
    kumbarium_audit::EventKind::SecretRead,
    &ns,
    serde_json::json!({ "name": name, "granted": true }),
  ) {
    return fail(&e);
  }
  let conn = match state.secrets() {
    Ok(c) => c,
    Err(e) => return fail(&e),
  };
  let value =
    match kumbarium_secrets::read_secret(conn, &ns, name, key.as_ref()) {
      Ok(v) => v,
      Err(e) => return fail(&e.to_string()),
    };
  if std::io::stdout().is_terminal() {
    eprintln!(
      "kumbarium: terminal scrollback is a ledger too; prefer \
       kum secret copy"
    );
  }
  let mut out = std::io::stdout();
  if out
    .write_all(&value)
    .and_then(|()| out.write_all(b"\n"))
    .is_err()
  {
    return fail("writing value to stdout");
  }
  ExitCode::SUCCESS
}

fn copy_cmd(ns: &str, name: &str) -> ExitCode {
  let (_, mut state) = match open_stores() {
    Ok(v) => v,
    Err(e) => return fail(&e),
  };
  let ns = kumbarium_librarian::normalize_namespace(ns);
  if let Err(e) = kumbarium_librarian::validate_namespace(&ns) {
    return fail(&format!("invalid namespace: {e}"));
  }
  let key = match tools::secrets_key(&mut state) {
    Ok(k) => k,
    Err(e) => return fail(&e),
  };
  if let Err(e) = witness(
    &state,
    kumbarium_audit::EventKind::SecretCopy,
    &ns,
    serde_json::json!({ "name": name }),
  ) {
    return fail(&e);
  }
  let conn = match state.secrets() {
    Ok(c) => c,
    Err(e) => return fail(&e),
  };
  let value =
    match kumbarium_secrets::read_secret(conn, &ns, name, key.as_ref()) {
      Ok(v) => v,
      Err(e) => return fail(&e.to_string()),
    };
  match clipboard_copy(&value) {
    Ok(tool) => {
      spawn_clipboard_clear(tool);
      println!(
        "copied {ns}/{name} to the clipboard (concealed); it \
         clears itself in 90 seconds"
      );
      ExitCode::SUCCESS
    }
    Err(e) => fail(&e),
  }
}

/// Pipe the value into the platform clipboard tool; the value
/// never touches argv or stdout. Returns the tool used, for
/// the matching auto-clear.
fn clipboard_copy(value: &[u8]) -> Result<&'static str, String> {
  let candidates: &[&[&str]] = if cfg!(target_os = "macos") {
    &[&["pbcopy"]]
  } else {
    &[&["wl-copy"], &["xclip", "-selection", "clipboard"]]
  };
  for argv in candidates {
    let child = std::process::Command::new(argv[0])
      .args(&argv[1..])
      .stdin(std::process::Stdio::piped())
      .spawn();
    let Ok(mut child) = child else { continue };
    if let Some(stdin) = child.stdin.as_mut()
      && stdin.write_all(value).is_err()
    {
      let _ = child.wait();
      continue;
    }
    drop(child.stdin.take());
    match child.wait() {
      Ok(status) if status.success() => return Ok(argv[0]),
      _ => continue,
    }
  }
  Err(
    "no clipboard tool reachable (pbcopy / wl-copy / xclip); \
     kum secret read prints instead"
      .into(),
  )
}

/// Detached best-effort clear: a credential should not sit on
/// the clipboard all afternoon. Clearing overwrites with empty
/// input via the same tool; if the spawn fails, the copy stays
/// (stated, not hidden: the printed message already promised
/// only what this attempts).
fn spawn_clipboard_clear(tool: &str) {
  let script = match tool {
    "xclip" => "sleep 90; printf '' | xclip -selection clipboard",
    "wl-copy" => "sleep 90; wl-copy --clear",
    _ => "sleep 90; printf '' | pbcopy",
  };
  let _ = std::process::Command::new("sh")
    .args(["-c", script])
    .stdin(std::process::Stdio::null())
    .stdout(std::process::Stdio::null())
    .stderr(std::process::Stdio::null())
    .spawn();
}

fn grant_cmd(ns: &str, name: &str, agent: &str, flags: &[&str]) -> ExitCode {
  // A lease: --until DATE grants through that day (UTC), and
  // read-time re-checks make expiry honest enforcement, not
  // metadata (D-038).
  let mut until: Option<String> = None;
  let mut it = flags.iter();
  while let Some(flag) = it.next() {
    match *flag {
      "--until" => match it.next() {
        Some(date) => {
          if let Err(e) = valid_date(date) {
            return fail(&e);
          }
          until = Some(format!("{date}T23:59:59.999Z"));
        }
        None => return fail("--until needs YYYY-MM-DD"),
      },
      other => return fail(&format!("unknown flag {other:?}")),
    }
  }
  let (_, mut state) = match open_stores() {
    Ok(v) => v,
    Err(e) => return fail(&e),
  };
  let ns = match checked_namespace(&state, ns) {
    Ok(ns) => ns,
    Err(e) => return fail(&e),
  };
  if agent.trim().is_empty() {
    return fail("grant needs an agent id");
  }
  let conn = match state.secrets() {
    Ok(c) => c,
    Err(e) => return fail(&e),
  };
  let exists = kumbarium_secrets::list(conn, Some(&ns))
    .map(|rows| rows.iter().any(|m| m.name == name))
    .unwrap_or(false);
  if !exists {
    return fail(&format!(
      "no live secret {ns}/{name}; grants name real stock only"
    ));
  }
  if let Err(e) =
    kumbarium_secrets::grant(conn, &ns, name, agent, until.as_deref())
  {
    return fail(&e.to_string());
  }
  if let Err(e) = witness(
    &state,
    kumbarium_audit::EventKind::SecretGrant,
    &ns,
    match &until {
      Some(ts) => serde_json::json!({
        "name": name, "grantee": agent, "until": ts,
      }),
      None => serde_json::json!({ "name": name, "grantee": agent }),
    },
  ) {
    return fail(&format!("granted, but {e}"));
  }
  match &until {
    Some(ts) => println!(
      "granted reveal on {ns}/{name} to {agent} through {} UTC \
       (revocable anytime; every read re-checks)",
      &ts[..10]
    ),
    None => {
      println!("granted reveal on {ns}/{name} to {agent} (revocable anytime)")
    }
  }
  ExitCode::SUCCESS
}

fn revoke_cmd(ns: &str, name: &str, agent: &str) -> ExitCode {
  let (_, mut state) = match open_stores() {
    Ok(v) => v,
    Err(e) => return fail(&e),
  };
  let ns = kumbarium_librarian::normalize_namespace(ns);
  if let Err(e) = kumbarium_librarian::validate_namespace(&ns) {
    return fail(&format!("invalid namespace: {e}"));
  }
  let conn = match state.secrets() {
    Ok(c) => c,
    Err(e) => return fail(&e),
  };
  let removed = match kumbarium_secrets::revoke(conn, &ns, name, agent) {
    Ok(r) => r,
    Err(e) => return fail(&e.to_string()),
  };
  if !removed {
    return fail(&format!("no grant on {ns}/{name} for {agent}"));
  }
  if let Err(e) = witness(
    &state,
    kumbarium_audit::EventKind::SecretRevoke,
    &ns,
    serde_json::json!({ "name": name, "grantee": agent }),
  ) {
    return fail(&format!("revoked, but {e}"));
  }
  println!(
    "revoked {agent} from {ns}/{name}; effective now (every \
     read re-checks)"
  );
  ExitCode::SUCCESS
}

fn shred_cmd(ns: &str, name: &str) -> ExitCode {
  let (_, mut state) = match open_stores() {
    Ok(v) => v,
    Err(e) => return fail(&e),
  };
  let ns = kumbarium_librarian::normalize_namespace(ns);
  if let Err(e) = kumbarium_librarian::validate_namespace(&ns) {
    return fail(&format!("invalid namespace: {e}"));
  }
  let conn = match state.secrets() {
    Ok(c) => c,
    Err(e) => return fail(&e),
  };
  let meta = match kumbarium_secrets::shred(conn, &ns, name) {
    Ok(m) => m,
    Err(e) => return fail(&e.to_string()),
  };
  if let Err(e) = witness(
    &state,
    kumbarium_audit::EventKind::SecretShred,
    &ns,
    serde_json::json!({ "name": name, "id": meta.id }),
  ) {
    return fail(&format!("shredded, but {e}"));
  }
  let sty = style::Style::detect();
  println!(
    "shredded {ns}/{name} ({}); the value is destroyed, the \
     record remains",
    sty.id(kumbarium_secrets::short_id(&meta.id))
  );
  ExitCode::SUCCESS
}

/// `kum secrets [ns]`: the stacks at a glance. Names, grants,
/// sealing mode; structurally never values.
pub(crate) fn secrets_cmd(ns: Option<&str>) -> ExitCode {
  let (p, mut state) = match open_stores() {
    Ok(v) => v,
    Err(e) => return fail(&e),
  };
  let sty = style::Style::detect();
  if !p.secrets_db.exists() {
    println!("the restricted stacks are empty (no secrets stocked)");
    return ExitCode::SUCCESS;
  }
  let ns = match ns {
    Some(raw) => {
      let n = kumbarium_librarian::normalize_namespace(raw);
      if let Err(e) = kumbarium_librarian::validate_namespace(&n) {
        return fail(&format!("invalid namespace: {e}"));
      }
      Some(n)
    }
    None => None,
  };
  let conn = match state.secrets() {
    Ok(c) => c,
    Err(e) => return fail(&e),
  };
  let mode = match kumbarium_secrets::sealing_mode(conn) {
    Ok(m) => m,
    Err(e) => return fail(&e.to_string()),
  };
  let sealing = match mode {
    Some(kumbarium_secrets::Sealing::Keystore) => "keystore-sealed".into(),
    Some(kumbarium_secrets::Sealing::Plaintext) => {
      sty.yellow("PLAINTEXT (chosen at first use)")
    }
    None => "undecided (first set decides)".into(),
  };
  println!("{} ({sealing})", sty.bold("the restricted stacks"));
  let rows = match kumbarium_secrets::list(conn, ns.as_deref()) {
    Ok(r) => r,
    Err(e) => return fail(&e.to_string()),
  };
  if rows.is_empty() {
    println!("no secrets stocked");
  } else {
    println!(
      "\n{}",
      sty.dim("id        namespace            name                 rotated")
    );
    let today = kumbarium_util::now_iso8601();
    for m in &rows {
      let expiry = match &m.expires_at {
        Some(d) if &today[..10] > d.as_str() => {
          sty.red(&format!("  EXPIRED {d}"))
        }
        Some(d) => sty.dim(&format!("  expires {d}")),
        None => String::new(),
      };
      println!(
        "{}  {:<20} {:<20} {}{expiry}",
        sty.id(&format!("{:<8}", kumbarium_secrets::short_id(&m.id))),
        m.namespace,
        m.name,
        sty.dim(&local_display(&m.updated_at))
      );
    }
  }
  let grants = match kumbarium_secrets::grants(conn, ns.as_deref()) {
    Ok(g) => g,
    Err(e) => return fail(&e.to_string()),
  };
  if !grants.is_empty() {
    println!("\n{}", sty.bold("grants (deny-by-default elsewhere)"));
    for g in &grants {
      let lease = match &g.expires_at {
        Some(until) => format!(" until {until}"),
        None => String::new(),
      };
      println!(
        "  {}/{} {} {} ({}{lease})",
        g.namespace,
        g.name,
        sty.dim("->"),
        g.agent_id,
        g.mode
      );
    }
  }
  ExitCode::SUCCESS
}

/// The metadata card (`kum show` fall-through, last shelf in
/// the chain): everything about a secret EXCEPT its value,
/// absent by shape (SecretMeta).
pub(crate) fn show_secret(
  state: &mut tools::ServerState,
  id: &str,
) -> Result<ExitCode, String> {
  let conn = state.secrets()?;
  let full = match kumbarium_secrets::resolve_id(conn, id) {
    Ok(f) => f,
    Err(kumbarium_secrets::SecretsError::IdNotFound(_)) => {
      return Err(format!("no entry, task, handoff, or secret with id {id:?}"));
    }
    Err(e) => return Err(e.to_string()),
  };
  let m = kumbarium_secrets::meta(conn, &full).map_err(|e| e.to_string())?;
  let sty = style::Style::detect();
  println!(
    "{}",
    sty.bold("secret (the restricted stacks; the value is never shown)")
  );
  println!(
    "id:         {} (short: {})",
    m.id,
    kumbarium_secrets::short_id(&m.id)
  );
  println!("namespace:  {}", m.namespace);
  println!("name:       {}", m.name);
  let status = if m.shredded_at.is_some() && m.superseded_by.is_none() {
    "shredded (value destroyed, record kept)"
  } else if m.superseded_by.is_some() {
    "superseded (rotated; value shredded)"
  } else {
    "live"
  };
  println!("status:     {status}");
  if let Some(d) = &m.expires_at {
    let today = kumbarium_util::now_iso8601();
    if &today[..10] > d.as_str() {
      println!("expires:    {} (upstream; never enforced)", sty.red(d));
    } else {
      println!("expires:    {d} (upstream; never enforced)");
    }
  }
  if let Some(note) = &m.note {
    println!("note:       {note}");
  }
  println!(
    "stocked:    {} by {}",
    local_display(&m.created_at),
    m.agent_id
  );
  if let Some(next) = &m.superseded_by {
    println!(
      "superseded: by {} (kum history {} reads the chain)",
      kumbarium_secrets::short_id(next),
      kumbarium_secrets::short_id(&m.id)
    );
  }
  println!(
    "\nkum secret read {} {} prints the value (witnessed)",
    m.namespace, m.name
  );
  Ok(ExitCode::SUCCESS)
}

/// The rotation chain (`kum history` fall-through): who rotated
/// and when, values structurally absent.
pub(crate) fn secret_history_cmd(
  state: &mut tools::ServerState,
  id: &str,
) -> ExitCode {
  let sty = style::Style::detect();
  let conn = match state.secrets() {
    Ok(c) => c,
    Err(e) => return fail(&e),
  };
  let full = match kumbarium_secrets::resolve_id(conn, id) {
    Ok(f) => f,
    Err(e) => return fail(&e.to_string()),
  };
  let chain = match kumbarium_secrets::history(conn, &full) {
    Ok(c) => c,
    Err(e) => return fail(&e.to_string()),
  };
  let head = chain.last().expect("nonempty chain");
  println!(
    "{}",
    sty.bold(&format!(
      "rotation chain for {}/{} ({} versions, values absent)",
      head.namespace,
      head.name,
      chain.len()
    ))
  );
  for (i, m) in chain.iter().enumerate() {
    let live = m.superseded_by.is_none() && m.shredded_at.is_none();
    let marker = if live { "live    " } else { "shredded" };
    println!(
      "v{} {} {} {}",
      i + 1,
      sty.dim(marker),
      sty.id(kumbarium_secrets::short_id(&m.id)),
      sty.dim(&local_display(&m.created_at))
    );
  }
  ExitCode::SUCCESS
}
