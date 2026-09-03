//! CLI color, hand-rolled: a tiny ANSI palette behind one
//! `Style` value. Color is on only when stdout is a real
//! terminal AND the NO_COLOR convention is unset, so piped
//! output stays plain automatically. The `serve` path never
//! touches this module: protocol stdout stays byte-clean by
//! construction, and TTY detection would kill color there
//! anyway.

use std::io::IsTerminal;

const RESET: &str = "\x1b[0m";
const BOLD: &str = "\x1b[1m";
const DIM: &str = "\x1b[2m";
const RED: &str = "\x1b[31m";
const GREEN: &str = "\x1b[32m";
const YELLOW: &str = "\x1b[33m";
const BLUE: &str = "\x1b[34m";
const MAGENTA: &str = "\x1b[35m";
const CYAN: &str = "\x1b[36m";

#[derive(Clone, Copy)]
pub struct Style {
  pub on: bool,
}

impl Style {
  pub fn detect() -> Style {
    Style {
      on: std::io::stdout().is_terminal()
        && std::env::var_os("NO_COLOR").is_none(),
    }
  }

  fn wrap(&self, code: &str, text: &str) -> String {
    if self.on {
      format!("{code}{text}{RESET}")
    } else {
      text.to_string()
    }
  }

  pub fn bold(&self, t: &str) -> String {
    self.wrap(BOLD, t)
  }

  pub fn dim(&self, t: &str) -> String {
    self.wrap(DIM, t)
  }

  pub fn red(&self, t: &str) -> String {
    self.wrap(RED, t)
  }

  pub fn green(&self, t: &str) -> String {
    self.wrap(GREEN, t)
  }

  pub fn yellow(&self, t: &str) -> String {
    self.wrap(YELLOW, t)
  }

  pub fn cyan(&self, t: &str) -> String {
    self.wrap(CYAN, t)
  }

  pub fn magenta(&self, t: &str) -> String {
    self.wrap(MAGENTA, t)
  }

  /// Short ids, everywhere they appear.
  pub fn id(&self, t: &str) -> String {
    self.cyan(t)
  }

  /// Entry kinds get one hue each, stable across every command.
  /// Matches on trimmed text so already-padded table cells keep
  /// their alignment (ANSI codes confuse format-width padding,
  /// so callers pad first, then paint).
  pub fn kind(&self, kind: &str) -> String {
    let code = match kind.trim_end() {
      "preference" => GREEN,
      "project_state" => BLUE,
      "decision" => MAGENTA,
      "reference" => CYAN,
      _ => DIM,
    };
    self.wrap(code, kind)
  }

  /// Audit event kinds, colored by what they did to the store.
  pub fn event(&self, kind: &str) -> String {
    let code = match kind.trim_end() {
      "recall" => BLUE,
      "remember" | "import" => GREEN,
      "supersede" => YELLOW,
      "forget" => RED,
      "link" => CYAN,
      _ => DIM,
    };
    self.wrap(code, kind)
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn off_is_a_pure_passthrough() {
    let sty = Style { on: false };
    assert_eq!(sty.bold("x"), "x");
    assert_eq!(sty.kind("preference"), "preference");
    assert_eq!(sty.event("forget"), "forget");
  }

  #[test]
  fn on_wraps_and_resets() {
    let sty = Style { on: true };
    let painted = sty.red("gone");
    assert!(painted.starts_with("\x1b[31m"));
    assert!(painted.ends_with(RESET));
    assert!(painted.contains("gone"));
  }
}
