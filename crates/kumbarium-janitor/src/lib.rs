//! The janitor: the designated mover of the confidence number
//! (D-004, D-025, D-040). A deterministic, stateless pass:
//! every run recomputes every live entry's confidence from the
//! full audit ledger, so reruns are idempotent and nothing
//! drifts. Survival (distinct agent-day exposures via recall)
//! is the backbone; explicit confirms and cross-agent link
//! votes are garnish with self-signals discounted; dormancy is
//! a per-kind finding for the human, never a penalty. v2 adds
//! the watchdog findings: pogo-sticking (the library served a
//! fact that was corrected right after), creeping matters,
//! unwitnessed grants (a tamper shape), and expired
//! credentials still stocked. Findings are advisory, zero
//! writes. Confidence informs, it never ranks (D-026).

#![forbid(unsafe_code)]

use std::collections::{BTreeSet, HashMap};

use kumbarium_audit::StoredEvent;
use kumbarium_store::{Connection, StoreError};

/// The neutral prior every entry starts at.
pub const PRIOR: f64 = 0.5;
/// Survival term: asymptote +0.30 (0.80 total).
const SURVIVAL_WEIGHT: f64 = 0.30;
const SURVIVAL_HALFWAY: f64 = 4.0;
/// Confirm term: asymptote +0.10 (demoted from 0.15 when link
/// authority joined, D-040: the 0.95 ceiling holds because
/// nothing inside the walls can prove application, so nothing
/// hits 1.0; garnish rebalances, the backbone never).
const CONFIRM_WEIGHT: f64 = 0.10;
const CONFIRM_HALFWAY: f64 = 1.0;
/// A confirm by the entry's own writer counts this much of one.
const SELF_CONFIRM: f64 = 0.25;
/// Link-authority term: asymptote +0.05. Inlinks are votes,
/// weighted by WHO linked (cross-agent full, self near zero:
/// the PageRank caveat solved by provenance).
const LINK_WEIGHT: f64 = 0.05;
const LINK_HALFWAY: f64 = 2.0;
/// A link from the entry's own writer counts this much of one.
const SELF_LINK: f64 = 0.1;
/// A supersede this soon after a recall that served the old
/// version is pogo-sticking: the library actively handed out a
/// fact that was wrong.
const POGO_WINDOW_MS: i64 = 48 * 3_600_000;
/// A goal that moved later this many times across a matter's
/// chain is creeping.
const CREEP_SLIPS: usize = 2;
/// Dormancy windows per kind, as multiples of the configured
/// dormant_days: project_state rots fast, preferences are
/// near-immortal (kind-deserves-freshness, D-040).
fn kind_window(kind: &str, dormant_days: i64) -> i64 {
  match kind {
    "project_state" => dormant_days / 2,
    "reference" => dormant_days * 2,
    "preference" => dormant_days * 4,
    _ => dormant_days,
  }
}
/// Proposals below this delta are noise, not evidence.
const MIN_DELTA: f64 = 0.01;

/// One proposed confidence change for a live entry.
#[derive(Debug, Clone)]
pub struct Proposal {
  pub id: String,
  pub namespace: String,
  pub old: f64,
  pub new: f64,
  pub basis: String,
}

/// A live entry never returned by any recall and older than the
/// dormancy window: a retire candidate for the human, listed
/// with its age. Zero confidence effect (no exposure is no
/// evidence).
#[derive(Debug, Clone)]
pub struct Dormant {
  pub id: String,
  pub namespace: String,
  pub age_days: i64,
}

/// Served-then-corrected: a supersede landed within the pogo
/// window of a recall that returned the old version. The
/// library actively handed out a fact that was wrong; stronger
/// signal than a cold correction. Advisory: the negative
/// evidence already landed on the dead version by definition.
#[derive(Debug, Clone)]
pub struct Pogo {
  pub id: String,
  pub scope: String,
  pub gap_hours: i64,
}

/// A matter whose goal slipped later CREEP_SLIPS+ times across
/// its chain: the deadline that keeps moving.
#[derive(Debug, Clone)]
pub struct Creep {
  pub id: String,
  pub namespace: String,
  pub slips: usize,
  pub goal: Option<String>,
}

/// A grants row with no matching secret_grant ledger event: a
/// write that bypassed the librarian (direct sqlite is the
/// obvious path). The tamper shape; the sharpest finding here.
#[derive(Debug, Clone)]
pub struct UnwitnessedGrant {
  pub namespace: String,
  pub name: String,
  pub agent_id: String,
}

/// An expired-but-unreleased reading-room lease: the
/// crashed-agent shape. The janitor reports; the human breaks.
#[derive(Debug, Clone)]
pub struct StaleLease {
  pub namespace: String,
  pub resource: String,
  pub agent_id: String,
  pub session_id: String,
  pub last_active: String,
}

/// A live secret whose upstream expiry has passed: still
/// stocked, still served, rotation owed.
#[derive(Debug, Clone)]
pub struct ExpiredStock {
  pub namespace: String,
  pub name: String,
  pub expires_at: String,
}

/// One open matter's goal history, oldest first (the CLI walks
/// the chain; the janitor counts the slips).
#[derive(Debug, Clone)]
pub struct GoalChain {
  pub id: String,
  pub namespace: String,
  pub goals: Vec<String>,
}

/// A grants row, as stocked (for the witness cross-check).
#[derive(Debug, Clone)]
pub struct GrantRow {
  pub namespace: String,
  pub name: String,
  pub agent_id: String,
}

/// A live secret's expiry metadata (never a value).
#[derive(Debug, Clone)]
pub struct SecretStock {
  pub namespace: String,
  pub name: String,
  pub expires_at: Option<String>,
}

/// The v2 inputs beyond the library and ledger, extracted by
/// the caller so the pass stays pure computation. Empty slices
/// mean the shelf does not exist yet; nothing is guessed.
#[derive(Debug, Clone, Default)]
pub struct Shelves {
  pub goal_chains: Vec<GoalChain>,
  pub grants: Vec<GrantRow>,
  pub secrets: Vec<SecretStock>,
  pub stale_leases: Vec<StaleLease>,
}

/// What one pass concluded. `proposals` are appliable changes;
/// everything else is advisory, zero writes.
#[derive(Debug, Clone, Default)]
pub struct Report {
  pub proposals: Vec<Proposal>,
  pub dormant: Vec<Dormant>,
  pub pogo: Vec<Pogo>,
  pub creep: Vec<Creep>,
  pub unwitnessed_grants: Vec<UnwitnessedGrant>,
  pub expired_stock: Vec<ExpiredStock>,
  pub stale_leases: Vec<StaleLease>,
}

/// Per-entry evidence tallied from the ledger.
#[derive(Debug, Default)]
struct Evidence {
  recalls: u64,
  agent_days: BTreeSet<(String, String)>,
  days: BTreeSet<String>,
  agents: BTreeSet<String>,
  confirms_other: u64,
  confirms_self: u64,
  links_other: u64,
  links_self: u64,
}

/// Run the pass: tally the ledger, recompute every live entry,
/// and report proposals (delta >= 0.01) plus the findings.
/// Reads only; applying is the caller's --apply transaction.
pub fn pass(
  library: &Connection,
  events: &[StoredEvent],
  shelves: &Shelves,
  dormant_days: i64,
  now_ms: i64,
) -> Result<Report, StoreError> {
  let entries = kumbarium_store::entries_in(library, None, false)?;
  let writers: HashMap<&str, &str> = entries
    .iter()
    .map(|e| (e.id.as_str(), e.agent_id.as_str()))
    .collect();

  let mut evidence: HashMap<String, Evidence> = HashMap::new();
  // Every recall of every id, ms-stamped with its asker, for
  // the pogo check (the id may be dead by now; that is the
  // point).
  let mut recalled_at: HashMap<String, Vec<(i64, String)>> = HashMap::new();
  let mut report = Report::default();
  for ev in events {
    let Ok(detail) = serde_json::from_str::<serde_json::Value>(&ev.detail)
    else {
      continue;
    };
    match ev.kind.as_str() {
      "recall" => {
        let Some(returned) = detail.get("returned").and_then(|r| r.as_array())
        else {
          continue;
        };
        let day = ev.at.get(..10).unwrap_or(&ev.at).to_string();
        let at_ms = kumbarium_util::parse_iso8601_ms(&ev.at);
        for id in returned.iter().filter_map(|x| x.as_str()) {
          let e = evidence.entry(id.to_string()).or_default();
          e.recalls += 1;
          e.days.insert(day.clone());
          e.agents.insert(ev.agent_id.clone());
          e.agent_days.insert((ev.agent_id.clone(), day.clone()));
          if let Some(ms) = at_ms {
            recalled_at
              .entry(id.to_string())
              .or_default()
              .push((ms, ev.agent_id.clone()));
          }
        }
      }
      "confirm" => {
        let Some(id) = detail.get("id").and_then(|x| x.as_str()) else {
          continue;
        };
        let e = evidence.entry(id.to_string()).or_default();
        if writers.get(id) == Some(&ev.agent_id.as_str()) {
          e.confirms_self += 1;
        } else {
          e.confirms_other += 1;
        }
      }
      "link" => {
        // Authority accrues to the LINKED-TO entry; the vote is
        // weighted by who cast it.
        let Some(to_id) = detail.get("to_id").and_then(|x| x.as_str()) else {
          continue;
        };
        let e = evidence.entry(to_id.to_string()).or_default();
        if writers.get(to_id) == Some(&ev.agent_id.as_str()) {
          e.links_self += 1;
        } else {
          e.links_other += 1;
        }
      }
      "supersede" => {
        let Some(old_id) = detail.get("old_id").and_then(|x| x.as_str()) else {
          continue;
        };
        let Some(sup_ms) = kumbarium_util::parse_iso8601_ms(&ev.at) else {
          continue;
        };
        // Only a CROSS-AGENT serve counts: the same agent
        // recalling and then superseding is the instructed
        // correction ritual (recall the stale entry, supersede
        // the id it returned), not the library misfiring.
        // Agent A served, agent B corrected: that fact
        // circulated wrong.
        let served_recently = recalled_at.get(old_id).and_then(|times| {
          times
            .iter()
            .filter(|(t, agent)| {
              *t <= sup_ms
                && sup_ms - *t <= POGO_WINDOW_MS
                && *agent != ev.agent_id
            })
            .map(|(t, _)| sup_ms - *t)
            .min()
        });
        if let Some(gap) = served_recently {
          report.pogo.push(Pogo {
            id: old_id.to_string(),
            scope: ev.scope.clone(),
            gap_hours: gap / 3_600_000,
          });
        }
      }
      _ => {}
    }
  }

  // Creeping matters: goals that moved later, repeatedly.
  for chain in &shelves.goal_chains {
    let slips = chain.goals.windows(2).filter(|w| w[1] > w[0]).count();
    if slips >= CREEP_SLIPS {
      report.creep.push(Creep {
        id: chain.id.clone(),
        namespace: chain.namespace.clone(),
        slips,
        goal: chain.goals.last().cloned(),
      });
    }
  }

  // Unwitnessed grants: every grants row must have a matching
  // secret_grant event on the ledger; a row without one arrived
  // around the librarian.
  let witnessed: std::collections::HashSet<(String, String, String)> = events
    .iter()
    .filter(|ev| ev.kind == "secret_grant")
    .filter_map(|ev| {
      let d = serde_json::from_str::<serde_json::Value>(&ev.detail).ok()?;
      Some((
        ev.scope.clone(),
        d.get("name")?.as_str()?.to_string(),
        d.get("grantee")?.as_str()?.to_string(),
      ))
    })
    .collect();
  for g in &shelves.grants {
    let key = (g.namespace.clone(), g.name.clone(), g.agent_id.clone());
    if !witnessed.contains(&key) {
      report.unwitnessed_grants.push(UnwitnessedGrant {
        namespace: g.namespace.clone(),
        name: g.name.clone(),
        agent_id: g.agent_id.clone(),
      });
    }
  }

  // The reading room's abandoned cards pass straight through:
  // the caller computed staleness with the live config ttl,
  // the janitor gives them a place in the one report.
  report.stale_leases = shelves.stale_leases.clone();

  // Expired credentials still stocked (value expiry is
  // metadata; this is where it gets surfaced with teeth).
  let today = kumbarium_util::format_iso8601_ms(now_ms);
  for sec in &shelves.secrets {
    if let Some(d) = &sec.expires_at
      && today.get(..10).is_some_and(|t| t > d.as_str())
    {
      report.expired_stock.push(ExpiredStock {
        namespace: sec.namespace.clone(),
        name: sec.name.clone(),
        expires_at: d.clone(),
      });
    }
  }

  for entry in &entries {
    let ev = evidence.get(&entry.id);
    let (new, basis) = score(ev);
    if (new - entry.confidence).abs() >= MIN_DELTA {
      report.proposals.push(Proposal {
        id: entry.id.clone(),
        namespace: entry.namespace.clone(),
        old: entry.confidence,
        new,
        basis,
      });
    }
    if ev.is_none_or(|e| e.recalls == 0) {
      let age_days = kumbarium_util::parse_iso8601_ms(&entry.created_at)
        .map(|ms| (now_ms - ms) / 86_400_000)
        .unwrap_or(0);
      let window = kind_window(entry.kind.as_str(), dormant_days);
      if age_days >= window {
        report.dormant.push(Dormant {
          id: entry.id.clone(),
          namespace: entry.namespace.clone(),
          age_days,
        });
      }
    }
  }
  Ok(report)
}

/// The formula (D-025), with its explanation. Deterministic:
/// the same evidence always yields the same number and basis.
fn score(ev: Option<&Evidence>) -> (f64, String) {
  let Some(ev) = ev.filter(|e| {
    e.recalls > 0
      || e.confirms_other > 0
      || e.confirms_self > 0
      || e.links_other > 0
      || e.links_self > 0
  }) else {
    return (PRIOR, "no exposure yet; neutral prior".to_string());
  };
  let k = ev.agent_days.len() as f64;
  let c = ev.confirms_other as f64 + SELF_CONFIRM * ev.confirms_self as f64;
  let l = ev.links_other as f64 + SELF_LINK * ev.links_self as f64;
  let raw = PRIOR
    + SURVIVAL_WEIGHT * k / (k + SURVIVAL_HALFWAY)
    + CONFIRM_WEIGHT * c / (c + CONFIRM_HALFWAY)
    + LINK_WEIGHT * l / (l + LINK_HALFWAY);
  let mut parts = Vec::new();
  if ev.recalls > 0 {
    parts.push(format!(
      "survived {} across {} over {}",
      count(ev.recalls, "recall"),
      count(ev.agents.len() as u64, "agent"),
      count(ev.days.len() as u64, "day"),
    ));
  }
  let confirms = ev.confirms_other + ev.confirms_self;
  if confirms > 0 {
    let mut c_str = format!("confirmed {confirms}x");
    if ev.confirms_self > 0 {
      c_str.push_str(&format!(" ({} self)", ev.confirms_self));
    }
    parts.push(c_str);
  }
  let links = ev.links_other + ev.links_self;
  if links > 0 {
    let mut l_str = format!("linked-to {links}x");
    if ev.links_self > 0 {
      l_str.push_str(&format!(" ({} self)", ev.links_self));
    }
    parts.push(l_str);
  }
  parts.push("never corrected".to_string());
  (round2(raw), parts.join("; "))
}

fn round2(x: f64) -> f64 {
  (x * 100.0).round() / 100.0
}

fn count(n: u64, noun: &str) -> String {
  if n == 1 {
    format!("1 {noun}")
  } else {
    format!("{n} {noun}s")
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  fn event(kind: &str, agent: &str, at: &str, detail: &str) -> StoredEvent {
    StoredEvent {
      id: kumbarium_util::generate_id(),
      at: at.to_string(),
      agent_id: agent.to_string(),
      session_id: String::new(),
      kind: kind.to_string(),
      scope: String::new(),
      detail: detail.to_string(),
    }
  }

  fn store_with_entry() -> (Connection, String) {
    let mut conn = kumbarium_store::open_in_memory().unwrap();
    let entry = kumbarium_store::remember(
      &mut conn,
      &kumbarium_store::NewEntry {
        namespace: "global".into(),
        kind: kumbarium_store::Kind::Decision,
        content: "the vantrike crate parses arguments".into(),
        agent_id: "writer".into(),
        source: "test".into(),
        tags: vec![],
        status: kumbarium_store::Status::Live,
      },
    )
    .unwrap();
    (conn, entry.id)
  }

  #[test]
  fn no_evidence_keeps_the_prior() {
    let (conn, _id) = store_with_entry();
    let report = pass(
      &conn,
      &[],
      &Shelves::default(),
      45,
      kumbarium_util::now_ms(),
    )
    .unwrap();
    assert!(report.proposals.is_empty(), "prior stays 0.50");
  }

  #[test]
  fn survival_raises_confidence() {
    let (conn, id) = store_with_entry();
    let detail = format!("{{\"query\":\"x\",\"returned\":[\"{id}\"]}}");
    let events = vec![
      event("recall", "a1", "2026-09-01T10:00:00.000Z", &detail),
      event("recall", "a2", "2026-09-02T10:00:00.000Z", &detail),
    ];
    let report = pass(
      &conn,
      &events,
      &Shelves::default(),
      45,
      kumbarium_util::now_ms(),
    )
    .unwrap();
    assert_eq!(report.proposals.len(), 1);
    let p = &report.proposals[0];
    // k = 2 distinct agent-days: 0.5 + 0.3 * 2/6 = 0.60.
    assert_eq!(p.new, 0.60);
    assert!(p.basis.contains("2 recalls"), "basis: {}", p.basis);
    assert!(p.basis.contains("2 agents"), "basis: {}", p.basis);
  }

  #[test]
  fn repeat_recalls_same_agent_same_day_count_once() {
    let (conn, id) = store_with_entry();
    let detail = format!("{{\"query\":\"x\",\"returned\":[\"{id}\"]}}");
    let events: Vec<_> = (0..10)
      .map(|i| {
        event(
          "recall",
          "a1",
          &format!("2026-09-01T10:0{i}:00.000Z"),
          &detail,
        )
      })
      .collect();
    let report = pass(
      &conn,
      &events,
      &Shelves::default(),
      45,
      kumbarium_util::now_ms(),
    )
    .unwrap();
    // k = 1: 0.5 + 0.3 * 1/5 = 0.56.
    assert_eq!(report.proposals[0].new, 0.56);
  }

  #[test]
  fn self_confirm_is_discounted() {
    let (conn, id) = store_with_entry();
    let confirm = format!("{{\"id\":\"{id}\"}}");
    let by_other = vec![event(
      "confirm",
      "someone-else",
      "2026-09-01T10:00:00.000Z",
      &confirm,
    )];
    let by_self = vec![event(
      "confirm",
      "writer",
      "2026-09-01T10:00:00.000Z",
      &confirm,
    )];
    let other = pass(
      &conn,
      &by_other,
      &Shelves::default(),
      45,
      kumbarium_util::now_ms(),
    )
    .unwrap();
    let this = pass(
      &conn,
      &by_self,
      &Shelves::default(),
      45,
      kumbarium_util::now_ms(),
    )
    .unwrap();
    // Other: 0.5 + 0.10 * 1/2 = 0.55.
    // Self: 0.5 + 0.10 * 0.25/1.25 = 0.52.
    assert!(other.proposals[0].new > this.proposals[0].new);
    assert!(this.proposals[0].basis.contains("(1 self)"));
  }

  #[test]
  fn pass_is_idempotent_after_apply() {
    let (conn, id) = store_with_entry();
    let detail = format!("{{\"query\":\"x\",\"returned\":[\"{id}\"]}}");
    let events =
      vec![event("recall", "a1", "2026-09-01T10:00:00.000Z", &detail)];
    let now = kumbarium_util::now_ms();
    let first = pass(&conn, &events, &Shelves::default(), 45, now).unwrap();
    assert_eq!(first.proposals.len(), 1);
    let p = &first.proposals[0];
    kumbarium_store::set_confidence(&conn, &p.id, p.new, &p.basis).unwrap();
    let second = pass(&conn, &events, &Shelves::default(), 45, now).unwrap();
    assert!(second.proposals.is_empty(), "recompute proposes nothing");
  }

  #[test]
  fn ceiling_stays_below_certainty() {
    let (conn, id) = store_with_entry();
    let detail = format!("{{\"query\":\"x\",\"returned\":[\"{id}\"]}}");
    let confirm = format!("{{\"id\":\"{id}\"}}");
    let mut events = Vec::new();
    for day in 1..=28 {
      for agent in ["a1", "a2", "a3", "a4"] {
        events.push(event(
          "recall",
          agent,
          &format!("2026-08-{day:02}T10:00:00.000Z"),
          &detail,
        ));
        events.push(event(
          "confirm",
          agent,
          &format!("2026-08-{day:02}T10:00:00.000Z"),
          &confirm,
        ));
      }
    }
    let report = pass(
      &conn,
      &events,
      &Shelves::default(),
      45,
      kumbarium_util::now_ms(),
    )
    .unwrap();
    assert!(report.proposals[0].new <= 0.95);
  }

  #[test]
  fn old_unrecalled_entry_is_flagged_dormant() {
    let (conn, id) = store_with_entry();
    // "Now" is 60 days after the entry was minted just now.
    let now = kumbarium_util::now_ms() + 60 * 86_400_000;
    let report = pass(&conn, &[], &Shelves::default(), 45, now).unwrap();
    assert_eq!(report.dormant.len(), 1);
    assert_eq!(report.dormant[0].id, id);
    assert!(report.dormant[0].age_days >= 60);
    // Dormancy is a finding, never a confidence penalty.
    assert!(report.proposals.is_empty());
  }

  #[test]
  fn fresh_unrecalled_entry_is_not_dormant() {
    let (conn, _id) = store_with_entry();
    let report = pass(
      &conn,
      &[],
      &Shelves::default(),
      45,
      kumbarium_util::now_ms(),
    )
    .unwrap();
    assert!(report.dormant.is_empty());
  }

  #[test]
  fn cross_agent_links_raise_and_self_links_barely_do() {
    let (conn, id) = store_with_entry();
    let link = format!(
      "{{\"from_id\":\"other\",\"to_id\":\"{id}\",\"rel\":\"relates_to\"}}"
    );
    let by_other = vec![event(
      "link",
      "someone-else",
      "2026-09-01T10:00:00.000Z",
      &link,
    )];
    let by_self =
      vec![event("link", "writer", "2026-09-01T10:00:00.000Z", &link)];
    let other = pass(
      &conn,
      &by_other,
      &Shelves::default(),
      45,
      kumbarium_util::now_ms(),
    )
    .unwrap();
    let this = pass(
      &conn,
      &by_self,
      &Shelves::default(),
      45,
      kumbarium_util::now_ms(),
    )
    .unwrap();
    // Other: 0.5 + 0.05 * 1/3 = 0.517 -> 0.52. Self: delta
    // 0.05 * 0.1/2.1 = 0.002, under MIN_DELTA: no proposal.
    assert_eq!(other.proposals[0].new, 0.52);
    assert!(other.proposals[0].basis.contains("linked-to 1x"));
    assert!(this.proposals.is_empty());
  }

  #[test]
  fn pogo_flags_served_then_corrected() {
    let (conn, id) = store_with_entry();
    let recall = format!("{{\"query\":\"x\",\"returned\":[\"{id}\"]}}");
    let sup = format!("{{\"old_id\":\"{id}\",\"new_id\":\"n1\"}}");
    let events = vec![
      event("recall", "a1", "2026-09-01T10:00:00.000Z", &recall),
      event("supersede", "a2", "2026-09-01T13:00:00.000Z", &sup),
    ];
    let report = pass(
      &conn,
      &events,
      &Shelves::default(),
      45,
      kumbarium_util::now_ms(),
    )
    .unwrap();
    assert_eq!(report.pogo.len(), 1);
    assert_eq!(report.pogo[0].id, id);
    assert_eq!(report.pogo[0].gap_hours, 3);
    // The same agent recalling then superseding is the
    // instructed correction ritual, never pogo.
    let ritual = vec![
      event("recall", "a1", "2026-09-01T10:00:00.000Z", &recall),
      event("supersede", "a1", "2026-09-01T13:00:00.000Z", &sup),
    ];
    let report = pass(
      &conn,
      &ritual,
      &Shelves::default(),
      45,
      kumbarium_util::now_ms(),
    )
    .unwrap();
    assert!(report.pogo.is_empty());
    // A cold correction (days later) is not pogo either.
    let cold = vec![
      event("recall", "a1", "2026-09-01T10:00:00.000Z", &recall),
      event("supersede", "a2", "2026-09-08T10:00:00.000Z", &sup),
    ];
    let report = pass(
      &conn,
      &cold,
      &Shelves::default(),
      45,
      kumbarium_util::now_ms(),
    )
    .unwrap();
    assert!(report.pogo.is_empty());
  }

  #[test]
  fn creep_needs_repeated_slips() {
    let (conn, _id) = store_with_entry();
    let shelves = Shelves {
      goal_chains: vec![
        GoalChain {
          id: "creeper".into(),
          namespace: "project/x".into(),
          goals: vec![
            "2026-09-01".into(),
            "2026-09-10".into(),
            "2026-10-01".into(),
          ],
        },
        GoalChain {
          id: "one-slip".into(),
          namespace: "project/x".into(),
          goals: vec!["2026-09-01".into(), "2026-09-10".into()],
        },
        GoalChain {
          id: "pulled-in".into(),
          namespace: "project/x".into(),
          goals: vec!["2026-10-01".into(), "2026-09-10".into()],
        },
      ],
      ..Default::default()
    };
    let report =
      pass(&conn, &[], &shelves, 45, kumbarium_util::now_ms()).unwrap();
    assert_eq!(report.creep.len(), 1);
    assert_eq!(report.creep[0].id, "creeper");
    assert_eq!(report.creep[0].slips, 2);
    assert_eq!(report.creep[0].goal.as_deref(), Some("2026-10-01"));
  }

  #[test]
  fn unwitnessed_grant_is_the_tamper_shape() {
    let (conn, _id) = store_with_entry();
    let shelves = Shelves {
      grants: vec![
        GrantRow {
          namespace: "project/x".into(),
          name: "api-key".into(),
          agent_id: "honest-agent".into(),
        },
        GrantRow {
          namespace: "project/x".into(),
          name: "api-key".into(),
          agent_id: "sneaky-agent".into(),
        },
      ],
      ..Default::default()
    };
    let witnessed = event(
      "secret_grant",
      "kumbarium-cli",
      "2026-09-01T10:00:00.000Z",
      "{\"name\":\"api-key\",\"grantee\":\"honest-agent\"}",
    );
    let witnessed = StoredEvent {
      scope: "project/x".into(),
      ..witnessed
    };
    let report =
      pass(&conn, &[witnessed], &shelves, 45, kumbarium_util::now_ms())
        .unwrap();
    assert_eq!(report.unwitnessed_grants.len(), 1);
    assert_eq!(report.unwitnessed_grants[0].agent_id, "sneaky-agent");
  }

  #[test]
  fn expired_stock_surfaces_and_future_does_not() {
    let (conn, _id) = store_with_entry();
    let shelves = Shelves {
      secrets: vec![
        SecretStock {
          namespace: "global".into(),
          name: "old-cert".into(),
          expires_at: Some("2020-01-01".into()),
        },
        SecretStock {
          namespace: "global".into(),
          name: "fresh-cert".into(),
          expires_at: Some("2999-01-01".into()),
        },
        SecretStock {
          namespace: "global".into(),
          name: "no-expiry".into(),
          expires_at: None,
        },
      ],
      ..Default::default()
    };
    let report =
      pass(&conn, &[], &shelves, 45, kumbarium_util::now_ms()).unwrap();
    assert_eq!(report.expired_stock.len(), 1);
    assert_eq!(report.expired_stock[0].name, "old-cert");
  }

  #[test]
  fn project_state_goes_dormant_twice_as_fast() {
    let mut conn = kumbarium_store::open_in_memory().unwrap();
    kumbarium_store::remember(
      &mut conn,
      &kumbarium_store::NewEntry {
        namespace: "global".into(),
        kind: kumbarium_store::Kind::ProjectState,
        content: "mid-flight state that rots".into(),
        agent_id: "writer".into(),
        source: "test".into(),
        tags: vec![],
        status: kumbarium_store::Status::Live,
      },
    )
    .unwrap();
    // 30 days old: inside the flat 45-day window, but past the
    // project_state window (45/2 = 22).
    let now = kumbarium_util::now_ms() + 30 * 86_400_000;
    let report = pass(&conn, &[], &Shelves::default(), 45, now).unwrap();
    assert_eq!(report.dormant.len(), 1);
  }
}
