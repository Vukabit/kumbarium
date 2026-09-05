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

pub(crate) fn secret_cmd(rest: &[&str]) -> ExitCode {
  match rest {
    ["set", ns, name, flags @ ..] => set_cmd(ns, name, flags),
    ["set", ..] => fail("secret set needs <ns> <name>"),
    // Retrieval verbs take a bare unique name (resolve-or-
    // refuse, like id fragments); write verbs never do (the
    // full address is deliberate friction on writes).
    ["read", ns, name] => read_cmd(ns, name),
    ["read", name] => match resolve_unique_name(name) {
      Ok((ns, name)) => read_cmd(&ns, &name),
      Err(e) => fail(&e),
    },
    ["read", ..] => fail("secret read needs <ns> <name> (or a unique name)"),
    ["copy", ns, name] => copy_cmd(ns, name),
    ["copy", name] => match resolve_unique_name(name) {
      Ok((ns, name)) => copy_cmd(&ns, &name),
      Err(e) => fail(&e),
    },
    ["copy", ..] => fail("secret copy needs <ns> <name> (or a unique name)"),
    ["grant", ns, name, agent, flags @ ..] => grant_cmd(ns, name, agent, flags),
    ["grant", ..] => {
      fail("secret grant needs <ns> <name> <agent> [--until DATE]")
    }
    ["revoke", ns, name, agent] => revoke_cmd(ns, name, agent),
    ["revoke", ..] => fail("secret revoke needs <ns> <name> <agent>"),
    ["shred", ns, name] => shred_cmd(ns, name, false),
    ["shred", ns, name, "--yes"] => shred_cmd(ns, name, true),
    ["shred", ..] => fail("secret shred needs <ns> <name> [--yes]"),
    // exec: one positional before the flags is a bare name,
    // two are ns + name.
    ["exec", name, rest @ ..]
      if rest.first().is_some_and(|w| *w == "--" || *w == "--as") =>
    {
      match resolve_unique_name(name) {
        Ok((ns, name)) => exec_cmd(&ns, &name, rest),
        Err(e) => fail(&e),
      }
    }
    ["exec", ns, name, rest @ ..] => exec_cmd(ns, name, rest),
    ["exec", ..] => {
      fail("secret exec needs <ns> <name> [--as VAR] -- cmd args...")
    }
    ["leakscan"] => leakscan_cmd(None),
    ["leakscan", ns] => leakscan_cmd(Some(ns)),
    // Bare singular browses like the plural; the verb map
    // lives in kum help secrets and the wrong-shape errors.
    [] => secrets_cmd(None, false),
    [verb, ..] => fail(&format!(
      "no secret verb {verb:?}; the verbs: set read copy grant \
       revoke shred exec leakscan (kum help secrets)"
    )),
  }
}

/// Resolve a bare name to its shelf: exactly one live secret
/// bears it, or the answer is a refusal that lists the
/// candidates (never a guess, same stance as ambiguous ids).
fn resolve_unique_name(name: &str) -> Result<(String, String), String> {
  let (_, mut state) = open_stores()?;
  let conn = state.secrets()?;
  let rows = kumbarium_secrets::list(conn, None).map_err(|e| e.to_string())?;
  let matches: Vec<_> = rows.iter().filter(|m| m.name == name).collect();
  match matches.as_slice() {
    [] => Err(format!("no live secret named {name:?} on any shelf")),
    [one] => Ok((one.namespace.clone(), one.name.clone())),
    many => Err(format!(
      "{name:?} lives on {} shelves ({}); name the namespace",
      many.len(),
      many
        .iter()
        .map(|m| m.namespace.as_str())
        .collect::<Vec<_>>()
        .join(", ")
    )),
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

/// The saved terminal state for the SIGINT window below: a
/// Ctrl-C during the echo-off prompt must restore echo before
/// the process dies, or the user's terminal is left silently
/// broken (the audit's finding; a security tool cannot leave
/// terminals damaged).
#[cfg(unix)]
static SAVED_TERMIOS: std::sync::Mutex<Option<libc::termios>> =
  std::sync::Mutex::new(None);

#[cfg(unix)]
extern "C" fn restore_tty_on_interrupt(sig: libc::c_int) {
  // Async-signal-safe enough for the narrow window: tcsetattr
  // and _exit are on the safe list; the mutex is only ever
  // written before the handler is installed.
  if let Ok(guard) = SAVED_TERMIOS.try_lock()
    && let Some(saved) = guard.as_ref()
  {
    unsafe {
      libc::tcsetattr(0, libc::TCSANOW, saved);
    }
  }
  unsafe {
    libc::signal(sig, libc::SIG_DFL);
    libc::raise(sig);
  }
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
  *SAVED_TERMIOS.lock().unwrap() = Some(saved);
  unsafe {
    let handler: extern "C" fn(libc::c_int) = restore_tty_on_interrupt;
    libc::signal(libc::SIGINT, handler as usize as libc::sighandler_t);
  }
  term.c_lflag &= !libc::ECHO;
  unsafe { libc::tcsetattr(0, libc::TCSANOW, &term) };
  let mut line = Zeroizing::new(String::new());
  let res = std::io::stdin().read_line(&mut line);
  unsafe {
    libc::tcsetattr(0, libc::TCSANOW, &saved);
    libc::signal(libc::SIGINT, libc::SIG_DFL);
  }
  *SAVED_TERMIOS.lock().unwrap() = None;
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

/// The shelf's answer for a name, for the pre-move check.
fn check_stock(
  state: &mut tools::ServerState,
  ns: &str,
  name: &str,
) -> Result<kumbarium_secrets::StockStatus, String> {
  let conn = state.secrets()?;
  kumbarium_secrets::stock_status(conn, ns, name).map_err(|e| e.to_string())
}

/// A miss becomes the teaching error: shredded and
/// never-stocked are different facts and read differently.
fn stock_error(
  ns: &str,
  name: &str,
  status: &kumbarium_secrets::StockStatus,
) -> Result<(), String> {
  match status {
    kumbarium_secrets::StockStatus::Live => Ok(()),
    kumbarium_secrets::StockStatus::Shredded(at) => Err(format!(
      "{ns}/{name} was shredded on {at}; the value is gone \
       (kum history finds the chain)"
    )),
    kumbarium_secrets::StockStatus::Missing => Err(format!(
      "no secret named {ns}/{name}; kum secrets {ns} lists what \
       is stocked"
    )),
  }
}

fn witness(
  state: &tools::ServerState,
  kind: kumbarium_audit::EventKind,
  scope: &str,
  detail: serde_json::Value,
) -> Result<(), String> {
  let event = kumbarium_audit::Event {
    agent_id: "kumbarium-cli".into(),
    session_id: state.session_id.clone(),
    kind,
    scope: scope.into(),
    detail,
  };
  kumbarium_audit::append(&state.audit, &event)
    .map(|_| ())
    .map_err(|e| format!("audit append failed: {e}"))
}

/// The expiry composition (D-038): the broker knows the date,
/// the docket does the reminding. Exactly one open rotation
/// matter per secret, keyed by the mechanical source
/// `secret:<ns>/<name>`: filed on first expiry, its goal
/// re-graded when the expiry moves. Never closed here;
/// completion stays a human judgment.
fn sync_rotation_matter(
  state: &mut tools::ServerState,
  ns: &str,
  name: &str,
  date: &str,
) -> Result<String, String> {
  let source = format!("secret:{ns}/{name}");
  let existing = {
    let conn = state.docket()?;
    kumbarium_docket::tasks_in(conn, Some(&[ns.to_string()]), false)
      .map_err(|e| e.to_string())?
      .into_iter()
      .find(|t| t.source == source)
  };
  match existing {
    Some(t) if t.goal.as_deref() == Some(date) => Ok(format!(
      "rotation matter {} already watches {date}",
      kumbarium_docket::short_id(&t.id)
    )),
    Some(t) => {
      let edit = kumbarium_docket::TaskEdit {
        content: None,
        severity: None,
        goal: Some(Some(date.to_string())),
        namespace: None,
        note: Some("expiry moved at rotation".into()),
      };
      let task = {
        let conn = state.docket()?;
        kumbarium_docket::supersede_task(conn, &t.id, &edit, "kumbarium-cli")
          .map_err(|e| e.to_string())?
      };
      witness(
        state,
        kumbarium_audit::EventKind::TaskUpdate,
        ns,
        serde_json::json!({
          "old_id": t.id,
          "new_id": task.id,
          "severity": task.severity.as_str(),
          "goal": task.goal,
          "note": edit.note,
        }),
      )?;
      Ok(format!(
        "rotation matter re-graded to {date} ({})",
        kumbarium_docket::short_id(&task.id)
      ))
    }
    None => {
      let new = kumbarium_docket::NewTask {
        namespace: ns.to_string(),
        content: format!("rotate the {name} credential; it expires upstream"),
        agent_id: "kumbarium-cli".into(),
        source,
        severity: kumbarium_docket::Severity::Normal,
        goal: Some(date.to_string()),
        status: kumbarium_docket::Status::Live,
      };
      let task = {
        let conn = state.docket()?;
        kumbarium_docket::file_task(conn, &new).map_err(|e| e.to_string())?
      };
      witness(
        state,
        kumbarium_audit::EventKind::TaskFile,
        ns,
        serde_json::json!({
          "id": task.id,
          "severity": task.severity.as_str(),
          "goal": task.goal,
        }),
      )?;
      Ok(format!(
        "rotation matter filed on the docket ({}, goal {date}); \
         creep does the reminding",
        kumbarium_docket::short_id(&task.id)
      ))
    }
  }
}

/// The open rotation matter for a secret, if one stands.
fn open_rotation_matter(
  state: &mut tools::ServerState,
  ns: &str,
  name: &str,
) -> Option<kumbarium_docket::Task> {
  let source = format!("secret:{ns}/{name}");
  let conn = state.docket().ok()?;
  kumbarium_docket::tasks_in(conn, Some(&[ns.to_string()]), false)
    .ok()?
    .into_iter()
    .find(|t| t.source == source)
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
  match &expires {
    Some(date) => {
      println!(
        "expiry {date} recorded (metadata; the broker never \
         enforces it)"
      );
      // The secret is stocked either way: a docket hiccup
      // warns, never unwinds the set.
      match sync_rotation_matter(&mut state, &ns, name, date) {
        Ok(msg) => println!("{msg}"),
        Err(e) => eprintln!(
          "kumbarium: expiry recorded, but the docket sync \
           failed: {e}"
        ),
      }
    }
    None => {
      if rotating && let Some(t) = open_rotation_matter(&mut state, &ns, name) {
        println!(
          "open rotation matter {} remains; kum task done {} if \
           this rotation settles it",
          kumbarium_docket::short_id(&t.id),
          kumbarium_docket::short_id(&t.id)
        );
      }
    }
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
  // Witness BEFORE the value moves, with the TRUE outcome
  // (fail-closed, D-038): a miss must never read as a
  // disclosure on the ledger. The CLI is the human's own
  // hands: no grant gate, but every attempt is on the record.
  let status = match check_stock(&mut state, &ns, name) {
    Ok(s) => s,
    Err(e) => return fail(&e),
  };
  let found = status == kumbarium_secrets::StockStatus::Live;
  if let Err(e) = witness(
    &state,
    kumbarium_audit::EventKind::SecretRead,
    &ns,
    serde_json::json!({ "name": name, "granted": true, "found": found }),
  ) {
    return fail(&e);
  }
  if let Err(e) = stock_error(&ns, name, &status) {
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
  let status = match check_stock(&mut state, &ns, name) {
    Ok(s) => s,
    Err(e) => return fail(&e),
  };
  let found = status == kumbarium_secrets::StockStatus::Live;
  if let Err(e) = witness(
    &state,
    kumbarium_audit::EventKind::SecretCopy,
    &ns,
    serde_json::json!({ "name": name, "found": found }),
  ) {
    return fail(&e);
  }
  if let Err(e) = stock_error(&ns, name, &status) {
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
  // A reserved word can never initialize as an agent, so a
  // grant to one would be a grant to nobody; refuse it here
  // where the typo is cheapest to see.
  if tools::reserved_agent_word(agent) {
    return fail(&format!(
      "agent name {agent:?} is reserved for the agent \
       lifecycle; no agent may bear it (kum agents lists the \
       identities the ledger has seen)"
    ));
  }
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

fn shred_cmd(ns: &str, name: &str, yes: bool) -> ExitCode {
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
  if let Err(e) = confirm_destruction(
    &format!("shredding {ns}/{name} destroys the value and"),
    yes,
  ) {
    return fail(&e);
  }
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
  if let Some(t) = open_rotation_matter(&mut state, &ns, name) {
    println!(
      "open rotation matter {} remains; kum task done (rotated) \
       or kum task drop (credential retired) is your call",
      kumbarium_docket::short_id(&t.id)
    );
  }
  ExitCode::SUCCESS
}

/// `kum secrets [ns]`: the stacks at a glance. Names, grants,
/// sealing mode; structurally never values.
pub(crate) fn secrets_cmd(ns: Option<&str>, json: bool) -> ExitCode {
  let (p, mut state) = match open_stores() {
    Ok(v) => v,
    Err(e) => return fail(&e),
  };
  let sty = style::Style::detect();
  if !p.secrets_db.exists() {
    if json {
      return print_json(&serde_json::json!({
        "sealing": null, "secrets": [], "grants": []
      }));
    }
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
  let rows = match kumbarium_secrets::list(conn, ns.as_deref()) {
    Ok(r) => r,
    Err(e) => return fail(&e.to_string()),
  };
  if json {
    // Metadata only, values structurally absent (SecretMeta
    // carries none to leak).
    let grants = match kumbarium_secrets::grants(conn, ns.as_deref()) {
      Ok(g) => g,
      Err(e) => return fail(&e.to_string()),
    };
    let sealing = match mode {
      Some(kumbarium_secrets::Sealing::Keystore) => Some("keystore"),
      Some(kumbarium_secrets::Sealing::Plaintext) => Some("plaintext"),
      None => None,
    };
    return print_json(&serde_json::json!({
      "sealing": sealing,
      "secrets": rows.iter().map(|m| serde_json::json!({
        "id": m.id,
        "namespace": m.namespace,
        "name": m.name,
        "expires_at": m.expires_at,
        "shredded_at": m.shredded_at,
        "stocked_at": m.updated_at,
      })).collect::<Vec<_>>(),
      "grants": grants.iter().map(|g| serde_json::json!({
        "namespace": g.namespace,
        "name": g.name,
        "agent_id": g.agent_id,
        "mode": g.mode,
        "granted_at": g.created_at,
        "until": g.expires_at,
      })).collect::<Vec<_>>(),
    }));
  }
  let sealing = match mode {
    Some(kumbarium_secrets::Sealing::Keystore) => "keystore-sealed".into(),
    Some(kumbarium_secrets::Sealing::Plaintext) => {
      sty.yellow("PLAINTEXT (chosen at first use)")
    }
    None => "undecided (first set decides)".into(),
  };
  println!("{} ({sealing})", sty.bold("the restricted stacks"));
  if rows.is_empty() {
    println!("no secrets stocked");
  } else {
    const COLS: &[Col] = &[
      Col {
        title: "id",
        width: 8,
      },
      Col {
        title: "namespace",
        width: 20,
      },
      Col {
        title: "name",
        width: 20,
      },
      Col {
        title: "stocked (local)",
        width: 0,
      },
    ];
    println!("\n{}", sty.dim(&table_header(COLS)));
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
        "{} {} {} {}{expiry}",
        sty.id(&cell(COLS, 0, kumbarium_secrets::short_id(&m.id))),
        cell(COLS, 1, &m.namespace),
        cell(COLS, 2, &m.name),
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
      // Humanized: the day access was granted and the day the
      // lease ends, as the human typed them, never the internal
      // end-of-day timestamp.
      let granted_day = g.created_at.get(..10).unwrap_or(&g.created_at);
      let lease = match &g.expires_at {
        Some(until) => {
          format!(", until {}", until.get(..10).unwrap_or(until))
        }
        None => String::new(),
      };
      println!(
        "  {}/{} {} {} ({}, granted {granted_day}{lease})",
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
      return Err(format!(
        "no entry, task, handoff, or secret with id {id:?} \
         (ids: the 8-char short form, the full id, or any \
         unique fragment of 4+ hex chars)"
      ));
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
  {
    let holders = kumbarium_secrets::grants(conn, Some(&m.namespace))
      .map(|rows| {
        rows
          .into_iter()
          .filter(|g| g.name == m.name)
          .map(|g| {
            let until = g
              .expires_at
              .map(|u| {
                format!(" until {}", u.get(..10).unwrap_or("?").to_owned())
              })
              .unwrap_or_default();
            format!("{}{until}", g.agent_id)
          })
          .collect::<Vec<_>>()
      })
      .unwrap_or_default();
    if holders.is_empty() {
      println!("grants:     none (agents are refused)");
    } else {
      println!("grants:     {}", holders.join(", "));
    }
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

/// The env-var name a secret injects as by default:
/// `crates-io-token` becomes `CRATES_IO_TOKEN`.
fn env_name(name: &str) -> String {
  let mut var: String = name
    .chars()
    .map(|c| {
      if c.is_ascii_alphanumeric() {
        c.to_ascii_uppercase()
      } else {
        '_'
      }
    })
    .collect();
  if var.starts_with(|c: char| c.is_ascii_digit()) {
    var.insert(0, '_');
  }
  var
}

/// Copy `reader` to `writer`, replacing every occurrence of
/// `needle` with `mask`. Holds back needle.len()-1 bytes across
/// chunk boundaries so a value split between two reads still
/// dies; the tail flushes at EOF.
fn redact_stream(
  mut reader: impl std::io::Read,
  mut writer: impl std::io::Write,
  needle: &[u8],
  mask: &[u8],
) -> std::io::Result<()> {
  let find = |hay: &[u8]| hay.windows(needle.len()).position(|w| w == needle);
  let keep = needle.len().saturating_sub(1);
  let mut held: Vec<u8> = Vec::new();
  let mut chunk = [0u8; 8192];
  loop {
    let n = reader.read(&mut chunk)?;
    let eof = n == 0;
    held.extend_from_slice(&chunk[..n]);
    while let Some(pos) = find(&held) {
      writer.write_all(&held[..pos])?;
      writer.write_all(mask)?;
      held.drain(..pos + needle.len());
    }
    if eof {
      writer.write_all(&held)?;
      return writer.flush();
    }
    if held.len() > keep {
      let flush = held.len() - keep;
      writer.write_all(&held[..flush])?;
      held.drain(..flush);
    }
    writer.flush()?;
  }
}

/// `kum secret exec <ns> <name> [--as VAR] -- cmd args...`:
/// the value goes into the child's ENVIRONMENT (never argv,
/// never this process's stdout), and the child's stdout and
/// stderr stream back through the redactor, so a failing
/// command that echoes its credential (a curl URL, a 401
/// header dump, a stack trace) prints a mask instead. The
/// agent-facing use-not-see tension stays deferred: this is
/// the human-invoked half.
fn exec_cmd(ns: &str, name: &str, rest: &[&str]) -> ExitCode {
  let mut var: Option<String> = None;
  let mut i = 0;
  while i < rest.len() {
    match rest[i] {
      "--as" => match rest.get(i + 1) {
        Some(v) => {
          var = Some((*v).to_string());
          i += 2;
        }
        None => return fail("--as needs a variable name"),
      },
      "--" => {
        i += 1;
        break;
      }
      other => {
        return fail(&format!(
          "unexpected {other:?}; secret exec needs `--` before \
           the command"
        ));
      }
    }
  }
  let cmd = &rest[i..];
  if cmd.is_empty() {
    return fail("no command after `--`");
  }
  let var = var.unwrap_or_else(|| env_name(name));
  if var.is_empty()
    || var.starts_with(|c: char| c.is_ascii_digit())
    || !var.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
  {
    return fail(&format!("invalid env var name {var:?}"));
  }
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
  // command word is metadata worth keeping; the argv tail is
  // not (it can carry paths and fragments better left off the
  // ledger).
  let status = match check_stock(&mut state, &ns, name) {
    Ok(s) => s,
    Err(e) => return fail(&e),
  };
  let found = status == kumbarium_secrets::StockStatus::Live;
  if let Err(e) = witness(
    &state,
    kumbarium_audit::EventKind::SecretExec,
    &ns,
    serde_json::json!({ "name": name, "command": cmd[0], "found": found }),
  ) {
    return fail(&e);
  }
  if let Err(e) = stock_error(&ns, name, &status) {
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
  let value_text = String::from_utf8_lossy(&value).to_string();
  let mask = format!("[kumbarium:redacted {ns}/{name}]");
  let child = std::process::Command::new(cmd[0])
    .args(&cmd[1..])
    .env(&var, &value_text)
    .stdout(std::process::Stdio::piped())
    .stderr(std::process::Stdio::piped())
    .spawn();
  let mut child = match child {
    Ok(c) => c,
    Err(e) => return fail(&format!("spawning {}: {e}", cmd[0])),
  };
  let out = child.stdout.take().expect("piped");
  let err = child.stderr.take().expect("piped");
  let needle = value_text.clone().into_bytes();
  let mask_out = mask.clone().into_bytes();
  let t_out = std::thread::spawn(move || {
    let _ = redact_stream(out, std::io::stdout(), &needle, &mask_out);
  });
  let needle = value_text.into_bytes();
  let mask_err = mask.into_bytes();
  let t_err = std::thread::spawn(move || {
    let _ = redact_stream(err, std::io::stderr(), &needle, &mask_err);
  });
  let status = child.wait();
  let _ = t_out.join();
  let _ = t_err.join();
  match status {
    Ok(st) => {
      let code = st.code().unwrap_or(1).clamp(0, 255) as u8;
      ExitCode::from(code)
    }
    Err(e) => fail(&format!("waiting on {}: {e}", cmd[0])),
  }
}

/// One shelf sweep: every row whose text contains the value.
fn scan_shelf(
  conn: &kumbarium_secrets::Connection,
  sql: &str,
  value: &str,
) -> Vec<String> {
  let Ok(mut stmt) = conn.prepare(sql) else {
    return Vec::new();
  };
  stmt
    .query_map([value], |row| row.get::<_, String>(0))
    .map(|rows| rows.flatten().collect())
    .unwrap_or_default()
}

/// Sweep the shelves and the ledger for one secret's bytes.
/// Returns (shelf label, row id) per exposure; the content
/// itself never surfaces.
fn scan_for_value(
  state: &mut tools::ServerState,
  value: &str,
) -> Vec<(&'static str, String)> {
  let mut hits: Vec<(&'static str, String)> = Vec::new();
  for id in scan_shelf(
    &state.library,
    "SELECT id FROM entries WHERE instr(content, ?1) > 0",
    value,
  ) {
    hits.push(("memory", id));
  }
  if let Ok(conn) = state.docket() {
    for id in scan_shelf(
      conn,
      "SELECT id FROM tasks WHERE instr(content, ?1) > 0",
      value,
    ) {
      hits.push(("docket", id));
    }
  }
  if let Ok(conn) = state.handoff() {
    for id in scan_shelf(
      conn,
      "SELECT id FROM handoffs WHERE instr(content, ?1) > 0",
      value,
    ) {
      hits.push(("handoff", id));
    }
  }
  for id in scan_shelf(
    &state.audit,
    "SELECT id FROM events WHERE instr(detail, ?1) > 0",
    value,
  ) {
    hits.push(("ledger", id));
  }
  hits
}

/// Values shorter than this sweep everything and mean nothing.
const LEAKSCAN_MIN: usize = 8;

/// `kum secret leakscan [ns]`: unseal every live secret in
/// process and sweep memories, tasks, briefings, and ledger
/// details for its bytes. Detection for the custody terminus:
/// what left the broker's hands and landed where it must never
/// rest. Exit 1 on any exposure, so the scan can gate.
fn leakscan_cmd(ns: Option<&str>) -> ExitCode {
  let (p, mut state) = match open_stores() {
    Ok(v) => v,
    Err(e) => return fail(&e),
  };
  let sty = style::Style::detect();
  if !p.secrets_db.exists() {
    println!("nothing stocked, nothing to sweep");
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
  let key = match tools::secrets_key(&mut state) {
    Ok(k) => k,
    Err(e) => return fail(&e),
  };
  let rows = {
    let conn = match state.secrets() {
      Ok(c) => c,
      Err(e) => return fail(&e),
    };
    match kumbarium_secrets::list(conn, ns.as_deref()) {
      Ok(r) => r,
      Err(e) => return fail(&e.to_string()),
    }
  };
  let mut scanned = 0i64;
  let mut exposures = 0i64;
  for m in &rows {
    let value = {
      let conn = match state.secrets() {
        Ok(c) => c,
        Err(e) => return fail(&e),
      };
      match kumbarium_secrets::read_secret(
        conn,
        &m.namespace,
        &m.name,
        key.as_ref(),
      ) {
        Ok(v) => v,
        Err(e) => return fail(&e.to_string()),
      }
    };
    if value.len() < LEAKSCAN_MIN {
      println!(
        "{}/{}: skipped (value shorter than {LEAKSCAN_MIN} \
         bytes sweeps everything and means nothing)",
        m.namespace, m.name
      );
      continue;
    }
    scanned += 1;
    let text = String::from_utf8_lossy(&value).to_string();
    let hits = scan_for_value(&mut state, &text);
    if hits.is_empty() {
      println!("{}/{}: clean", m.namespace, m.name);
    } else {
      exposures += hits.len() as i64;
      for (shelf, id) in &hits {
        println!(
          "{}/{}: {} on the {shelf} shelf ({}); scrub it \
           (forget / shred the row), then rotate the credential",
          m.namespace,
          m.name,
          sty.red("EXPOSED"),
          &id[id.len().saturating_sub(8)..]
        );
      }
    }
  }
  println!(
    "{}",
    sty.dim(
      "swept memories, tasks, briefings, ledger details; \
       exported files on disk are not swept"
    )
  );
  if let Err(e) = witness(
    &state,
    kumbarium_audit::EventKind::SecretLeakscan,
    ns.as_deref().unwrap_or(""),
    serde_json::json!({ "scanned": scanned, "hits": exposures }),
  ) {
    return fail(&format!("swept, but {e}"));
  }
  if exposures > 0 {
    eprintln!("kumbarium: {exposures} exposure(s) found");
    return ExitCode::FAILURE;
  }
  ExitCode::SUCCESS
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn redactor_kills_values_split_across_chunks() {
    // A reader that yields one byte at a time forces the value
    // across every chunk boundary there is.
    struct OneByte<'a>(&'a [u8], usize);
    impl std::io::Read for OneByte<'_> {
      fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        if self.1 >= self.0.len() {
          return Ok(0);
        }
        buf[0] = self.0[self.1];
        self.1 += 1;
        Ok(1)
      }
    }
    let input = b"error: auth failed for token=tok-swordfish-9 (twice: \
tok-swordfish-9), retrying";
    let mut out = Vec::new();
    redact_stream(
      OneByte(input, 0),
      &mut out,
      b"tok-swordfish-9",
      b"[redacted]",
    )
    .unwrap();
    let text = String::from_utf8(out).unwrap();
    assert!(!text.contains("swordfish"), "{text}");
    assert_eq!(text.matches("[redacted]").count(), 2);
    assert!(text.ends_with("retrying"));
  }

  #[test]
  fn env_names_derive_mechanically() {
    assert_eq!(env_name("crates-io-token"), "CRATES_IO_TOKEN");
    assert_eq!(env_name("2fa.seed"), "_2FA_SEED");
  }

  #[test]
  fn leakscan_finds_planted_bytes_and_never_flags_clean_shelves() {
    let mut state = tools::ServerState::in_memory();
    let planted = "vk-vantrike-0441-leak";
    {
      let conn = state.docket().unwrap();
      kumbarium_docket::file_task(
        conn,
        &kumbarium_docket::NewTask {
          namespace: "project/x".into(),
          content: format!("use {planted} for the deploy"),
          agent_id: "unit-test".into(),
          source: "unit-test".into(),
          severity: kumbarium_docket::Severity::Normal,
          goal: None,
          status: kumbarium_docket::Status::Live,
        },
      )
      .unwrap();
    }
    let hits = scan_for_value(&mut state, planted);
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].0, "docket");
    assert!(scan_for_value(&mut state, "never-written-anywhere").is_empty());
  }

  #[test]
  fn rotation_matter_files_once_then_regrades() {
    let mut state = tools::ServerState::in_memory();
    let first =
      sync_rotation_matter(&mut state, "project/x", "api-key", "2026-12-01")
        .unwrap();
    assert!(first.contains("filed"), "{first}");
    let again =
      sync_rotation_matter(&mut state, "project/x", "api-key", "2026-12-01")
        .unwrap();
    assert!(again.contains("already watches"), "{again}");
    let moved =
      sync_rotation_matter(&mut state, "project/x", "api-key", "2027-06-01")
        .unwrap();
    assert!(moved.contains("re-graded"), "{moved}");
    // One open matter survives, goal moved, source mechanical.
    let open = {
      let conn = state.docket().unwrap();
      kumbarium_docket::tasks_in(conn, None, false).unwrap()
    };
    assert_eq!(open.len(), 1);
    assert_eq!(open[0].goal.as_deref(), Some("2027-06-01"));
    assert_eq!(open[0].source, "secret:project/x/api-key");
    // Both writes witnessed: one filing, one regrade.
    let (files, updates): (i64, i64) = state
      .audit
      .query_row(
        "SELECT
           (SELECT count(*) FROM events WHERE kind = 'task_file'),
           (SELECT count(*) FROM events WHERE kind = 'task_update')",
        [],
        |row| Ok((row.get(0)?, row.get(1)?)),
      )
      .unwrap();
    assert_eq!((files, updates), (1, 1));
  }
}
