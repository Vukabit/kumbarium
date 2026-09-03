//! The janitor: the designated mover of the confidence number
//! (D-004, D-025). v1 is a deterministic, stateless pass: every
//! run recomputes every live entry's confidence from the full
//! audit ledger, so reruns are idempotent and nothing drifts.
//! Survival (distinct agent-day exposures via recall) is the
//! backbone; explicit confirms are garnish with self-confirms
//! discounted; dormancy is a finding for the human, never a
//! penalty. Confidence informs, it never ranks (D-026).

#![forbid(unsafe_code)]

use std::collections::{BTreeSet, HashMap};

use kumbarium_audit::StoredEvent;
use kumbarium_store::{Connection, StoreError};

/// The neutral prior every entry starts at.
pub const PRIOR: f64 = 0.5;
/// Survival term: asymptote +0.30 (0.80 total).
const SURVIVAL_WEIGHT: f64 = 0.30;
const SURVIVAL_HALFWAY: f64 = 4.0;
/// Confirm term: asymptote +0.15 (0.95 total ceiling; nothing
/// inside the walls can prove application, so nothing hits 1.0).
const CONFIRM_WEIGHT: f64 = 0.15;
const CONFIRM_HALFWAY: f64 = 1.0;
/// A confirm by the entry's own writer counts this much of one.
const SELF_CONFIRM: f64 = 0.25;
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

/// What one pass concluded. `proposals` are appliable changes;
/// `dormant` is advisory only.
#[derive(Debug, Clone, Default)]
pub struct Report {
  pub proposals: Vec<Proposal>,
  pub dormant: Vec<Dormant>,
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
}

/// Run the pass: tally the ledger, recompute every live entry,
/// and report proposals (delta >= 0.01) plus dormancy findings.
/// Reads only; applying is the caller's --apply transaction.
pub fn pass(
  library: &Connection,
  events: &[StoredEvent],
  dormant_days: i64,
  now_ms: i64,
) -> Result<Report, StoreError> {
  let entries = kumbarium_store::entries_in(library, None, false)?;
  let writers: HashMap<&str, &str> = entries
    .iter()
    .map(|e| (e.id.as_str(), e.agent_id.as_str()))
    .collect();

  let mut evidence: HashMap<String, Evidence> = HashMap::new();
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
        for id in returned.iter().filter_map(|x| x.as_str()) {
          let e = evidence.entry(id.to_string()).or_default();
          e.recalls += 1;
          e.days.insert(day.clone());
          e.agents.insert(ev.agent_id.clone());
          e.agent_days.insert((ev.agent_id.clone(), day.clone()));
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
      _ => {}
    }
  }

  let mut report = Report::default();
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
      if age_days >= dormant_days {
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
  let Some(ev) =
    ev.filter(|e| e.recalls > 0 || e.confirms_other > 0 || e.confirms_self > 0)
  else {
    return (PRIOR, "no exposure yet; neutral prior".to_string());
  };
  let k = ev.agent_days.len() as f64;
  let c = ev.confirms_other as f64 + SELF_CONFIRM * ev.confirms_self as f64;
  let raw = PRIOR
    + SURVIVAL_WEIGHT * k / (k + SURVIVAL_HALFWAY)
    + CONFIRM_WEIGHT * c / (c + CONFIRM_HALFWAY);
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
      },
    )
    .unwrap();
    (conn, entry.id)
  }

  #[test]
  fn no_evidence_keeps_the_prior() {
    let (conn, _id) = store_with_entry();
    let report = pass(&conn, &[], 45, kumbarium_util::now_ms()).unwrap();
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
    let report = pass(&conn, &events, 45, kumbarium_util::now_ms()).unwrap();
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
    let report = pass(&conn, &events, 45, kumbarium_util::now_ms()).unwrap();
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
    let other = pass(&conn, &by_other, 45, kumbarium_util::now_ms()).unwrap();
    let this = pass(&conn, &by_self, 45, kumbarium_util::now_ms()).unwrap();
    // Other: 0.5 + 0.15 * 1/2 = 0.575 -> 0.57 (banker-free round).
    // Self: 0.5 + 0.15 * 0.25/1.25 = 0.53.
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
    let first = pass(&conn, &events, 45, now).unwrap();
    assert_eq!(first.proposals.len(), 1);
    let p = &first.proposals[0];
    kumbarium_store::set_confidence(&conn, &p.id, p.new, &p.basis).unwrap();
    let second = pass(&conn, &events, 45, now).unwrap();
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
    let report = pass(&conn, &events, 45, kumbarium_util::now_ms()).unwrap();
    assert!(report.proposals[0].new <= 0.95);
  }

  #[test]
  fn old_unrecalled_entry_is_flagged_dormant() {
    let (conn, id) = store_with_entry();
    // "Now" is 60 days after the entry was minted just now.
    let now = kumbarium_util::now_ms() + 60 * 86_400_000;
    let report = pass(&conn, &[], 45, now).unwrap();
    assert_eq!(report.dormant.len(), 1);
    assert_eq!(report.dormant[0].id, id);
    assert!(report.dormant[0].age_days >= 60);
    // Dormancy is a finding, never a confidence penalty.
    assert!(report.proposals.is_empty());
  }

  #[test]
  fn fresh_unrecalled_entry_is_not_dormant() {
    let (conn, _id) = store_with_entry();
    let report = pass(&conn, &[], 45, kumbarium_util::now_ms()).unwrap();
    assert!(report.dormant.is_empty());
  }
}
