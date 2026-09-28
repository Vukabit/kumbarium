//! The dossier (D-042): `kum dossier <agent>` renders one
//! agent's whole witnessed story as a deterministic postmortem:
//! what it was served, what it wrote and how those writes
//! fared, what the desk judged, what credentials it touched or
//! was refused, and the chronological record itself, with the
//! hash chain verified up front so the page states its own
//! trustworthiness. The binder's sibling on the other axis: the
//! binder reads a SCOPE, the dossier reads an AGENT. Pure
//! rendering, nothing written, not witnessed (browsing is not
//! circulation).

use std::collections::BTreeSet;
use std::process::ExitCode;

use super::super::actor::{ActorIndex, Who};
use super::super::{open_stores, style};
use super::term::*;

/// One roster row, tallied from every shelf.
#[derive(Default)]
struct RosterRow {
  sessions: BTreeSet<String>,
  events: usize,
  first_at: String,
  last_at: String,
  live: usize,
  corrected: usize,
  grants: usize,
  leases: usize,
}

/// `kum agents`: the roster. Every identity the witness has
/// ever seen, what it holds, and when it was last here. Counts,
/// never scores (the metric-theater trap stays sprung): the
/// numbers are for YOUR judgment, and `kum dossier <agent>` is
/// the deep story behind any row.
pub(crate) fn agents_cmd(all: bool, json: bool) -> ExitCode {
  let (p, mut state) = match open_stores() {
    Ok(v) => v,
    Err(e) => return fail(&e),
  };
  let sty = style::Style::detect();
  let events = match kumbarium_audit::events_asc(&state.audit) {
    Ok(v) => v,
    Err(e) => return fail(&e.to_string()),
  };
  // Rows are ACTORS (D-056): sessions attributed through their
  // bind, merges followed. History from before actors keeps its
  // claimed name, marked unbound, never guessed into an actor.
  let idx = match ActorIndex::load(&state.library, &events) {
    Ok(i) => i,
    Err(e) => return fail(&e),
  };
  let mut roster: std::collections::BTreeMap<Who, RosterRow> =
    std::collections::BTreeMap::new();
  for ev in &events {
    let row = roster.entry(idx.who_event(ev)).or_default();
    row.events += 1;
    if !ev.session_id.is_empty() {
      row.sessions.insert(ev.session_id.clone());
    }
    if row.first_at.is_empty() {
      row.first_at = ev.at.clone();
    }
    row.last_at = ev.at.clone();
  }
  // Registered actors appear even before their first session.
  for id in idx.actors.keys() {
    if idx.actors[id].merged_into.is_none() {
      roster.entry(Who::Actor(id.clone())).or_default();
    }
  }
  // The estate, per writer (writers may predate the ledger:
  // imports carry identities too, so entries seed rows).
  let entries = match kumbarium_store::entries_in(&state.library, None, true) {
    Ok(v) => v,
    Err(e) => return fail(&e.to_string()),
  };
  for e in &entries {
    let who = idx.who_entry(e);
    if e.status == kumbarium_store::Status::Live {
      let corrected = match &e.superseded_by {
        None => {
          roster.entry(who.clone()).or_default().live += 1;
          false
        }
        Some(next) => kumbarium_store::get(&state.library, next)
          .map(|s| idx.who_entry(&s) != who)
          .unwrap_or(false),
      };
      if corrected {
        roster.entry(who).or_default().corrected += 1;
      }
    }
  }
  if p.secrets_db.exists()
    && let Ok(conn) = state.secrets()
    && let Ok(grants) = kumbarium_secrets::grants(conn, None)
  {
    for g in grants {
      let who = if g.grantee_kind == "actor" {
        idx.resolve(&state.library, &g.agent_id)
      } else {
        Who::Claimed(g.agent_id)
      };
      roster.entry(who).or_default().grants += 1;
    }
  }
  let ttl = state.cfg.leases_ttl_minutes;
  if p.leases_db.exists()
    && let Ok(conn) = state.leases()
    && let Ok(active) =
      kumbarium_leases::active_in(conn, None, kumbarium_util::now_ms(), ttl)
  {
    for l in active {
      let who = idx.who_session(&l.session_id, &l.agent_id);
      roster.entry(who).or_default().leases += 1;
    }
  }
  // Retired: `kum agent retire` on an actor, or (legacy) a
  // claimed name listed under [agents] retired in config.
  let cfg_retired: std::collections::HashSet<&str> = state
    .cfg
    .agents_retired
    .iter()
    .map(String::as_str)
    .collect();
  let is_retired = |who: &Who| match who {
    Who::Actor(_) => idx.actor(who).is_some_and(|a| a.retired_at.is_some()),
    Who::Claimed(name) => cfg_retired.contains(name.as_str()),
  };
  let hidden = roster.keys().filter(|w| is_retired(w)).count();
  if !all {
    roster.retain(|who, _| !is_retired(who));
  }
  let mut rows: Vec<(&Who, &RosterRow)> = roster.iter().collect();
  rows.sort_by(|a, b| b.1.last_at.cmp(&a.1.last_at));
  if json {
    let out: Vec<serde_json::Value> = rows
      .iter()
      .map(|(who, r)| {
        let a = idx.actor(who);
        serde_json::json!({
          "agent": match who {
            Who::Actor(_) => a.map(|a| a.name.clone()).unwrap_or_default(),
            Who::Claimed(n) => n.clone(),
          },
          "actor_id": a.map(|a| a.id.clone()),
          "kind": a.map(|a| a.kind.as_str()),
          "claimed": a.map(|a| a.claimed.clone()),
          "workspace": a
            .filter(|a| !a.workspace.is_empty())
            .map(|a| super::super::actor::display_path(&a.workspace)),
          "bound": matches!(who, Who::Actor(_)),
          "first_at": (!r.first_at.is_empty()).then_some(&r.first_at),
          "last_at": (!r.last_at.is_empty()).then_some(&r.last_at),
          "sessions": r.sessions.len(),
          "events": r.events,
          "live": r.live,
          "corrected_by_others": r.corrected,
          "grants": r.grants,
          "active_leases": r.leases,
          "retired": is_retired(who),
        })
      })
      .collect();
    return print_json(&serde_json::json!(out));
  }
  if roster.is_empty() {
    println!("no identities witnessed yet");
    return ExitCode::SUCCESS;
  }
  println!(
    "{} {}",
    sty.bold("the roster"),
    sty.dim(&format!(
      "({} identities; kum agent <name> for any deep story)",
      roster.len()
    ))
  );
  const COLS: &[Col] = &[
    Col {
      title: "actor",
      width: 30,
    },
    Col {
      title: "id",
      width: 8,
    },
    Col {
      title: "last seen (local)",
      width: 19,
    },
    Col {
      title: "sess",
      width: 4,
    },
    Col {
      title: "events",
      width: 6,
    },
    Col {
      title: "live",
      width: 4,
    },
    Col {
      title: "corr",
      width: 4,
    },
    Col {
      title: "grants",
      width: 6,
    },
    Col {
      title: "leases",
      width: 0,
    },
  ];
  println!("{}", sty.dim(&table_header(COLS)));
  for (who, r) in rows {
    let mut mark = String::new();
    if let Some(a) = idx.actor(who)
      && a.kind == kumbarium_store::ActorKind::Human
    {
      mark.push_str(&sty.dim(" [human]"));
    }
    if is_retired(who) {
      mark.push_str(&sty.yellow(" [retired]"));
    }
    let last = if r.last_at.is_empty() {
      match who {
        Who::Actor(_) => "(no sessions yet)".to_string(),
        Who::Claimed(_) => "(pre-ledger)".to_string(),
      }
    } else {
      local_display(&r.last_at)
    };
    let corr_cell = format!("{:>4}", r.corrected);
    let corr = if r.corrected > 0 {
      sty.yellow(&corr_cell)
    } else {
      corr_cell
    };
    let (label, id) = match who {
      Who::Actor(id) => {
        (idx.label(who), kumbarium_store::short_id(id).to_string())
      }
      Who::Claimed(_) => (idx.label(who), String::new()),
    };
    let label_cell = cell(COLS, 0, &label);
    let label_cell = if matches!(who, Who::Claimed(_)) {
      sty.dim(&label_cell)
    } else {
      label_cell
    };
    println!(
      "{} {} {} {:>4} {:>6} {:>4} {} {:>6} {}{mark}",
      label_cell,
      sty.id(&cell(COLS, 1, &id)),
      sty.dim(&cell(COLS, 2, &last)),
      r.sessions.len(),
      r.events,
      r.live,
      corr,
      r.grants,
      r.leases,
    );
  }
  if hidden > 0 && !all {
    println!(
      "{}",
      sty.dim(&format!(
        "({hidden} retired identities hidden; kum agents --all \
         shows them)"
      ))
    );
  }
  println!(
    "{}",
    sty.dim(
      "counts, never scores: corr = live-chain writes corrected \
       by OTHERS; judgment stays yours; (unbound) = history from \
       before actors (D-056)"
    )
  );
  ExitCode::SUCCESS
}

/// Everything the ledger says about one agent in one window,
/// tallied in a single pass.
#[derive(Default)]
struct Tally {
  events: usize,
  recalls: usize,
  scopes: BTreeSet<String>,
  served_ids: BTreeSet<String>,
  briefings_served: usize,
  matters_served: usize,
  remembers: usize,
  supersedes: usize,
  tasks_filed: usize,
  briefings_left: usize,
  approved: usize,
  rejected: usize,
  secret_reads: Vec<String>,
  secret_refused: Vec<String>,
  secret_missing: Vec<String>,
  secret_execs: usize,
  secret_copies: usize,
}

fn within(at: &str, since: Option<&str>, until: Option<&str>) -> bool {
  let day = at.get(..10).unwrap_or(at);
  since.is_none_or(|s| day >= s) && until.is_none_or(|u| day <= u)
}

/// A calendar day, the docket-goal grammar.
fn valid_date(date: &str) -> Result<(), String> {
  let ok = date.len() == 10
    && kumbarium_util::parse_iso8601_ms(&format!("{date}T00:00:00.000Z"))
      .is_some();
  match ok {
    true => Ok(()),
    false => Err(format!("invalid date {date:?}; use YYYY-MM-DD")),
  }
}

/// The session card (`kum show` fall-through, fifth and last
/// resolver, D-048): a minted session id becomes addressable
/// wherever it was printed. A rendering, nothing written, not
/// witnessed.
pub(crate) fn show_session(
  state: &mut super::super::tools::ServerState,
  id: &str,
) -> Result<ExitCode, String> {
  let not_found = || {
    format!(
      "no entry, task, handoff, secret, or session with id \
       {id:?} (ids: the 8-char short form, the full id, or any \
       unique fragment of 4+ hex chars)"
    )
  };
  if id.len() < 4 {
    return Err(not_found());
  }
  let mut candidates = kumbarium_audit::sessions_matching(&state.audit, id)
    .map_err(|e| e.to_string())?;
  // Sessions too young to have witnessed anything exist only
  // in the presence registry.
  let procs_dir = super::super::paths::resolve()
    .map(|p| p.procs_dir)
    .map_err(|e| e.to_string())?;
  let live = super::super::procs::live(&procs_dir);
  for r in &live {
    if r.session.contains(id) && !candidates.contains(&r.session) {
      candidates.push(r.session.clone());
    }
  }
  // A pid is not an id (different namespace, and all-digits is a
  // valid hex fragment, so show never resolves it as one). But
  // pids are what kum processes shows first, so when the arg IS
  // a live pid, hand back its session instead of a bare
  // not-found: the error names the key that works.
  let pid_hint = || {
    live.iter().find(|r| r.pid.to_string() == id).map(|r| {
      let s = r
        .session
        .get(r.session.len().saturating_sub(8)..)
        .unwrap_or("");
      format!(
        "{id:?} is a live process pid, not an id; its session \
           is {s} (kum show {s}, or kum processes)"
      )
    })
  };
  match candidates.as_slice() {
    [] => return Err(pid_hint().unwrap_or_else(not_found)),
    [_] => {}
    many => {
      let shorts: Vec<&str> = many
        .iter()
        .map(|s| s.get(s.len().saturating_sub(8)..).unwrap_or(s))
        .collect();
      return Err(format!(
        "session fragment {id:?} is ambiguous: {}",
        shorts.join(", ")
      ));
    }
  }
  let session = candidates.remove(0);
  let short = session.get(session.len().saturating_sub(8)..).unwrap_or("");
  let sty = style::Style::detect();
  let story = kumbarium_audit::session_story(&state.audit, &session)
    .map_err(|e| e.to_string())?;
  let alive = live.iter().find(|r| r.session == session);
  println!("{}", sty.bold(&format!("session {short} (minted)")));
  println!("id:         {session}");
  let agent = story
    .as_ref()
    .map(|s| s.agent.clone())
    .or_else(|| alive.map(|r| r.agent.clone()))
    .unwrap_or_else(|| "unknown".into());
  println!("agent:      {agent}");
  match alive {
    Some(r) => println!(
      "alive:      yes; pid {} on {} since {} (kum processes)",
      r.pid,
      r.version,
      local_display(&r.since)
    ),
    None => println!("alive:      no (the serve process has exited)"),
  }
  match &story {
    Some(s) => {
      println!(
        "first act:  {}   last act: {}",
        local_display(&s.first_at),
        local_display(&s.last_at)
      );
      let kinds = s
        .by_kind
        .iter()
        .map(|(k, n)| format!("{n} {k}"))
        .collect::<Vec<_>>()
        .join(", ");
      println!("events:     {} ({kinds})", s.events);
      if !s.scopes.is_empty() {
        println!("scopes:     {}", s.scopes.join(", "));
      }
    }
    None => println!("witnessed:  nothing yet (alive, no tool calls so far)"),
  }
  if state.leases_path.exists() {
    let now = kumbarium_util::now_ms();
    let ttl = state.cfg.leases_ttl_minutes;
    if let Ok(conn) = state.leases()
      && let Ok(active) = kumbarium_leases::active_in(conn, None, now, ttl)
    {
      let held: Vec<String> = active
        .iter()
        .filter(|l| l.session_id == session)
        .map(|l| format!("{}/{}", l.namespace, l.resource))
        .collect();
      if !held.is_empty() {
        println!("leases:     {}", held.join(", "));
      }
    }
  }
  println!(
    "\n{}",
    sty.dim(&format!(
      "deep story: kum dossier {agent} --session {short}"
    ))
  );
  Ok(ExitCode::SUCCESS)
}

pub(crate) fn dossier_cmd(agent: &str, rest: &[&str]) -> ExitCode {
  let mut since: Option<String> = None;
  let mut until: Option<String> = None;
  let mut session: Option<String> = None;
  let mut it = rest.iter();
  while let Some(flag) = it.next() {
    if *flag == "--session" {
      match it.next() {
        Some(frag) => session = Some((*frag).to_string()),
        None => return fail("--session needs an id fragment"),
      }
      continue;
    }
    let slot = match *flag {
      "--since" => &mut since,
      "--until" => &mut until,
      other => return fail(&format!("unknown flag {other:?}")),
    };
    match it.next() {
      Some(date) => {
        if let Err(e) = valid_date(date) {
          return fail(&e);
        }
        *slot = Some((*date).to_string());
      }
      None => return fail(&format!("{flag} needs YYYY-MM-DD")),
    }
  }
  let (_, state) = match open_stores() {
    Ok(v) => v,
    Err(e) => return fail(&e),
  };
  let sty = style::Style::detect();

  // The chain check leads: a dossier that cannot vouch for its
  // own source says so before saying anything else.
  let verified = match kumbarium_audit::verify_chain(&state.audit) {
    Ok(kumbarium_audit::ChainStatus::Intact { events, head }) => {
      let head = head.unwrap_or_default();
      format!(
        "ledger verified: chain intact ({events} events, head {})",
        head.get(..12).unwrap_or(&head)
      )
    }
    Ok(kumbarium_audit::ChainStatus::Broken { index, .. }) => format!(
      "ledger COMPROMISED: chain breaks at event {index}; \
       everything below is untrustworthy from there on"
    ),
    Err(e) => return fail(&e.to_string()),
  };

  let events = match kumbarium_audit::events_asc(&state.audit) {
    Ok(v) => v,
    Err(e) => return fail(&e.to_string()),
  };
  // The subject (D-056): an actor by name or id (merges followed,
  // so a merged actor's dossier reads the survivor's whole
  // trail), else a claimed name for pre-actor history.
  let idx = match ActorIndex::load(&state.library, &events) {
    Ok(i) => i,
    Err(e) => return fail(&e),
  };
  let target = idx.resolve(&state.library, agent);
  let title = idx.label(&target);
  let mut t = Tally::default();
  let mut record: Vec<&kumbarium_audit::StoredEvent> = Vec::new();
  let mut sessions: BTreeSet<String> = BTreeSet::new();
  for ev in &events {
    if !within(&ev.at, since.as_deref(), until.as_deref()) {
      continue;
    }
    if let Some(frag) = &session
      && !ev.session_id.contains(frag.as_str())
    {
      continue;
    }
    let who = idx.who_event(ev);
    if who == target && !ev.session_id.is_empty() {
      sessions.insert(ev.session_id.clone());
    }
    let detail: serde_json::Value =
      serde_json::from_str(&ev.detail).unwrap_or_default();
    // Desk judgments name the agent as SUBMITTER on someone
    // else's event; everything else is the agent's own.
    if (ev.kind == "approve" || ev.kind == "reject")
      && idx.who_submitter(&detail).as_ref() == Some(&target)
    {
      match ev.kind.as_str() {
        "approve" => t.approved += 1,
        _ => t.rejected += 1,
      }
      record.push(ev);
      continue;
    }
    if who != target {
      continue;
    }
    t.events += 1;
    record.push(ev);
    if !ev.scope.is_empty() {
      t.scopes.insert(ev.scope.clone());
    }
    match ev.kind.as_str() {
      "recall" => {
        t.recalls += 1;
        if let Some(ids) = detail.get("returned").and_then(|r| r.as_array()) {
          for id in ids.iter().filter_map(|x| x.as_str()) {
            t.served_ids.insert(id.to_string());
          }
        }
        if detail
          .get("handoff_served")
          .and_then(|x| x.as_bool())
          .unwrap_or(false)
        {
          t.briefings_served += 1;
        }
        t.matters_served += detail
          .get("matters_served")
          .and_then(|x| x.as_u64())
          .unwrap_or(0) as usize;
      }
      "remember" => t.remembers += 1,
      "supersede" => t.supersedes += 1,
      "task_file" => t.tasks_filed += 1,
      "handoff_write" => t.briefings_left += 1,
      "secret_read" => {
        let name = detail
          .get("name")
          .and_then(|x| x.as_str())
          .unwrap_or("?")
          .to_string();
        let granted = detail
          .get("granted")
          .and_then(|x| x.as_bool())
          .unwrap_or(false);
        // Pre-fidelity events lack `found`; treat absent as
        // true so history renders as it was understood then.
        let found = detail
          .get("found")
          .and_then(|x| x.as_bool())
          .unwrap_or(true);
        if !granted {
          t.secret_refused.push(name);
        } else if !found {
          t.secret_missing.push(name);
        } else {
          t.secret_reads.push(name);
        }
      }
      "secret_exec" => t.secret_execs += 1,
      "secret_copy" => t.secret_copies += 1,
      _ => {}
    }
  }
  if record.is_empty() {
    println!("no witnessed events for {title} in that window");
    return ExitCode::SUCCESS;
  }

  // The estate: this agent's writes as they stand TODAY (state
  // outlives the window on purpose: a write from last month
  // superseded yesterday is exactly what a postmortem wants).
  let all_entries =
    match kumbarium_store::entries_in(&state.library, None, true) {
      Ok(v) => v,
      Err(e) => return fail(&e.to_string()),
    };
  let mut live = 0usize;
  let mut superseded_by_self = 0usize;
  let mut superseded_by_others = 0usize;
  let mut pending = 0usize;
  let mut rejected_writes = 0usize;
  for e in all_entries.iter().filter(|e| idx.who_entry(e) == target) {
    match e.status {
      kumbarium_store::Status::Pending => pending += 1,
      kumbarium_store::Status::Rejected => rejected_writes += 1,
      kumbarium_store::Status::Live => match &e.superseded_by {
        None => live += 1,
        Some(next) => {
          let same = kumbarium_store::get(&state.library, next)
            .map(|s| idx.who_entry(&s) == target)
            .unwrap_or(false);
          if same {
            superseded_by_self += 1;
          } else {
            superseded_by_others += 1;
          }
        }
      },
    }
  }

  let mut window = match (&since, &until) {
    (None, None) => "all time".to_string(),
    (Some(s), None) => format!("since {s}"),
    (None, Some(u)) => format!("through {u}"),
    (Some(s), Some(u)) => format!("{s} through {u}"),
  };
  if let Some(frag) = &session {
    window.push_str(&format!(", session ~{frag}"));
  }
  println!("{}", sty.bold(&format!("the dossier: {title}")));
  if let Some(a) = idx.actor(&target) {
    let mut about = format!(
      "actor {} ({})",
      kumbarium_store::short_id(&a.id),
      a.kind.as_str()
    );
    if !a.claimed.is_empty() {
      about.push_str(&format!(", claims {}", a.claimed));
    }
    if !a.workspace.is_empty() {
      about.push_str(&format!(
        ", works in {}",
        super::super::actor::display_path(&a.workspace)
      ));
    }
    println!("{}", sty.dim(&about));
  }
  println!("{}", sty.dim(&format!("window: {window}")));
  println!("{}", sty.dim(&verified));

  if !sessions.is_empty() {
    // A CLI-heavy agent mints one session per invocation;
    // sixteen inline ids is noise. List the recent few and
    // point at the narrowing flag.
    let shorts: Vec<&str> = sessions
      .iter()
      .map(|s| s.get(s.len().saturating_sub(8)..).unwrap_or(s))
      .collect();
    let listed = if shorts.len() <= 4 {
      shorts.join(", ")
    } else {
      format!(
        "{}, ... (+{} earlier)",
        shorts[shorts.len() - 4..].join(", "),
        shorts.len() - 4
      )
    };
    println!(
      "{}",
      sty.dim(&format!(
        "{} minted session(s): {listed} (narrow with \
         --session <frag>)",
        sessions.len()
      ))
    );
  }
  println!("\n{}", sty.bold("what it was served"));
  println!(
    "  {} across {} ({} distinct entries)",
    count(t.recalls, "recall"),
    count(t.scopes.len(), "scope"),
    t.served_ids.len()
  );
  println!(
    "  briefings served: {}; matters served: {}",
    t.briefings_served, t.matters_served
  );

  println!("\n{}", sty.bold("what it wrote, and how it fared"));
  println!(
    "  {} witnessed in window; the estate as it stands: {live} \
     live, {pending} pending, {rejected_writes} rejected",
    count(t.remembers, "memory write"),
  );
  println!(
    "  revised by itself: {superseded_by_self}; corrected by \
     OTHERS: {superseded_by_others} (the survival fact)"
  );
  println!(
    "  supersedes it performed: {}; tasks filed: {}; briefings \
     left: {}",
    t.supersedes, t.tasks_filed, t.briefings_left
  );
  if t.approved + t.rejected > 0 {
    println!(
      "  the desk's judgment of its submissions: {} approved, \
       {} rejected",
      t.approved, t.rejected
    );
  }

  if !t.secret_reads.is_empty()
    || !t.secret_refused.is_empty()
    || !t.secret_missing.is_empty()
    || t.secret_execs + t.secret_copies > 0
  {
    println!("\n{}", sty.bold("the restricted stacks"));
    if !t.secret_reads.is_empty() {
      println!(
        "  reads granted: {} ({})",
        t.secret_reads.len(),
        t.secret_reads.join(", ")
      );
    }
    if !t.secret_refused.is_empty() {
      println!(
        "  {} {} ({})",
        sty.red("REFUSED:"),
        t.secret_refused.len(),
        t.secret_refused.join(", ")
      );
    }
    if !t.secret_missing.is_empty() {
      println!(
        "  sought but not stocked: {} ({})",
        t.secret_missing.len(),
        t.secret_missing.join(", ")
      );
    }
    if t.secret_execs + t.secret_copies > 0 {
      println!(
        "  redacted execs: {}; concealed copies: {}",
        t.secret_execs, t.secret_copies
      );
    }
  }

  const COLS: &[Col] = &[
    Col {
      title: "at (local)",
      width: 19,
    },
    Col {
      title: "kind",
      width: 15,
    },
    Col {
      title: "scope",
      width: 20,
    },
    Col {
      title: "detail",
      width: 0,
    },
  ];
  println!("\n{}", sty.bold("the record, oldest first"));
  println!("{}", sty.dim(&table_header(COLS)));
  for ev in &record {
    let detail = kumbarium_audit::describe_event(&ev.kind, &ev.detail);
    let lines = hang(body_col(COLS), &detail);
    println!(
      "{} {} {} {}",
      sty.dim(&cell(COLS, 0, &local_display(&ev.at))),
      sty.event(&cell(COLS, 1, &ev.kind)),
      cell(COLS, 2, &ev.scope),
      lines[0]
    );
    for line in &lines[1..] {
      println!("{line}");
    }
  }
  ExitCode::SUCCESS
}

fn count(n: usize, noun: &str) -> String {
  match n {
    1 => format!("1 {noun}"),
    _ => format!("{n} {noun}s"),
  }
}
