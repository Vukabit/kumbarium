//! The one config file for every tunable. Hand-rolled parser
//! for the TOML SUBSET the config actually uses: `# comments`,
//! `[sections]`, `key = <integer>` lines, and `key = "string"`
//! lines (bare words accepted too; approvals policy needs them). Anything else in
//! the file earns a warning and the default value; a missing
//! file is simply all defaults. Policy lives here and in the
//! callers; mechanics (splitting, backups) stay in the crates
//! that own them.

/// Effective tunables, defaults matching the shipped constants.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Config {
  pub backup_interval_hours: i64,
  pub library_recent: usize,
  pub library_dailies: usize,
  pub library_weeklies: usize,
  pub audit_recent: usize,
  pub audit_dailies: usize,
  pub audit_weeklies: usize,
  pub split_target: usize,
  pub collapse_max_changed_lines: usize,
  pub recall_default_limit: usize,
  pub janitor_dormant_days: i64,
  /// Write policy (D-027): what a write becomes for agents not
  /// listed in `pending_agents`. true = pending-by-default.
  pub approvals_default_pending: bool,
  /// Agents whose writes always land pending, comma-separated
  /// in config.
  pub approvals_pending_agents: Vec<String>,
}

impl Default for Config {
  fn default() -> Config {
    Config {
      backup_interval_hours: 12,
      library_recent: 2,
      library_dailies: 7,
      library_weeklies: 4,
      audit_recent: 2,
      audit_dailies: 3,
      audit_weeklies: 0,
      split_target: kumbarium_librarian::SPLIT_TARGET,
      collapse_max_changed_lines: 4,
      recall_default_limit: 8,
      janitor_dormant_days: 45,
      approvals_default_pending: false,
      approvals_pending_agents: Vec::new(),
    }
  }
}

/// The commented template `kum config --init` writes.
pub const TEMPLATE: &str = "\
# Kumbarium tunables. Missing keys use built-in defaults;
# this file supports comments, [sections], and integer values.

[backup]
# Hours between automatic snapshots (checked at serve startup).
interval_hours = 12
# Tiered retention: newest N, newest-per-day for N days,
# newest-per-week for N weeks.
library_recent = 2
library_dailies = 7
library_weeklies = 4
audit_recent = 2
audit_dailies = 3
audit_weeklies = 0

[write]
# Target part size in bytes before auto-split kicks in.
split_target = 1500

[history]
# A noted version collapses only when its diff changed at most
# this many lines (the diff decides, never the note).
collapse_max_changed_lines = 4

[recall]
# Hits returned when an agent omits `limit`.
default_limit = 8

[janitor]
# A live entry never returned by any recall and at least this
# old is flagged dormant (a finding for the human, never a
# confidence penalty).
dormant_days = 45

[approvals]
# Write policy (D-027). live = writes circulate immediately
# (personal tier); pending = every agent write waits for
# `kum approve` unless the agent is trusted elsewhere.
default_mode = \"live\"
# Agents whose writes ALWAYS land pending, comma-separated,
# e.g. \"intern-bot, contrib-scraper\".
pending_agents = \"\"
";

/// Parse config text over the defaults. Unknown or malformed
/// lines never fail startup: each yields a warning and the
/// default stands (loud, but an agent's server still comes up).
pub fn parse(text: &str) -> (Config, Vec<String>) {
  let mut cfg = Config::default();
  let mut warnings = Vec::new();
  let mut section = String::new();
  for (lineno, raw) in text.lines().enumerate() {
    let line = raw.split('#').next().unwrap_or("").trim();
    if line.is_empty() {
      continue;
    }
    if let Some(name) = line.strip_prefix('[').and_then(|r| r.strip_suffix(']'))
    {
      section = name.trim().to_string();
      continue;
    }
    let Some((key, value)) = line.split_once('=') else {
      warnings.push(format!(
        "config line {}: not `key = value`; ignored",
        lineno + 1
      ));
      continue;
    };
    let key = format!("{section}.{}", key.trim());
    let raw_value = value.trim().trim_matches('"').to_string();
    // String-valued keys first; everything else is an integer.
    match key.as_str() {
      "approvals.default_mode" => {
        match raw_value.as_str() {
          "live" => cfg.approvals_default_pending = false,
          "pending" => cfg.approvals_default_pending = true,
          other => warnings.push(format!(
            "config approvals.default_mode: {other:?} is not \
             live|pending; default kept"
          )),
        }
        continue;
      }
      "approvals.pending_agents" => {
        cfg.approvals_pending_agents = raw_value
          .split(',')
          .map(str::trim)
          .filter(|a| !a.is_empty())
          .map(str::to_string)
          .collect();
        continue;
      }
      _ => {}
    }
    let Ok(value) = value.trim().parse::<i64>() else {
      warnings.push(format!("config {key}: not an integer; default kept"));
      continue;
    };
    let as_usize = usize::try_from(value).unwrap_or(0);
    match key.as_str() {
      "backup.interval_hours" => {
        cfg.backup_interval_hours = value.max(1);
      }
      "backup.library_recent" => cfg.library_recent = as_usize,
      "backup.library_dailies" => cfg.library_dailies = as_usize,
      "backup.library_weeklies" => {
        cfg.library_weeklies = as_usize;
      }
      "backup.audit_recent" => cfg.audit_recent = as_usize,
      "backup.audit_dailies" => cfg.audit_dailies = as_usize,
      "backup.audit_weeklies" => cfg.audit_weeklies = as_usize,
      "write.split_target" => {
        cfg.split_target = as_usize.max(200);
      }
      "history.collapse_max_changed_lines" => {
        cfg.collapse_max_changed_lines = as_usize;
      }
      "recall.default_limit" => {
        cfg.recall_default_limit = as_usize.max(1);
      }
      "janitor.dormant_days" => {
        cfg.janitor_dormant_days = value.max(1);
      }
      other => {
        warnings.push(format!("config {other}: unknown key; ignored"));
      }
    }
  }
  (cfg, warnings)
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn missing_file_semantics_are_all_defaults() {
    let (cfg, warnings) = parse("");
    assert_eq!(cfg, Config::default());
    assert!(warnings.is_empty());
  }

  #[test]
  fn template_parses_clean_and_matches_defaults() {
    let (cfg, warnings) = parse(TEMPLATE);
    assert_eq!(cfg, Config::default());
    assert!(warnings.is_empty(), "{warnings:?}");
  }

  #[test]
  fn overrides_apply_and_junk_warns_with_defaults_kept() {
    let text = "\
[backup]
interval_hours = 6
library_weeklies = 8

[write]
split_target = twelve
mystery = 4
just a stray line
";
    let (cfg, warnings) = parse(text);
    assert_eq!(cfg.backup_interval_hours, 6);
    assert_eq!(cfg.library_weeklies, 8);
    assert_eq!(cfg.split_target, Config::default().split_target);
    assert_eq!(warnings.len(), 3, "{warnings:?}");
  }

  #[test]
  fn approvals_policy_parses() {
    let (cfg, warnings) = parse(
      "[approvals]\ndefault_mode = \"pending\"\n\
       pending_agents = \"intern-bot, scraper\"\n",
    );
    assert!(cfg.approvals_default_pending);
    assert_eq!(
      cfg.approvals_pending_agents,
      vec!["intern-bot".to_string(), "scraper".to_string()]
    );
    assert!(warnings.is_empty(), "{warnings:?}");
    let (cfg, warnings) = parse("[approvals]\ndefault_mode = sideways\n");
    assert!(!cfg.approvals_default_pending, "junk keeps default");
    assert_eq!(warnings.len(), 1);
  }

  #[test]
  fn insane_values_clamp() {
    let (cfg, _) = parse(
      "[backup]\ninterval_hours = 0\n[write]\nsplit_target = 5\n\
       [recall]\ndefault_limit = 0\n",
    );
    assert_eq!(cfg.backup_interval_hours, 1);
    assert_eq!(cfg.split_target, 200);
    assert_eq!(cfg.recall_default_limit, 1);
  }
}
