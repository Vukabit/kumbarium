//! Terminal plumbing shared by every command: width, wrap,
//! local time, path quoting, and the doors out of the terminal
//! (file explorer, $EDITOR).

use std::process::ExitCode;

use super::super::style;

/// Reveal a file in the platform's file explorer. macOS and
/// Windows select the file itself; Linux has no portable
/// reveal-and-select, so the containing shelf opens instead.
pub(crate) fn reveal(path: &std::path::Path) -> Result<(), String> {
  #[cfg(target_os = "macos")]
  let mut cmd = {
    let mut c = std::process::Command::new("open");
    c.arg("-R").arg(path);
    c
  };
  #[cfg(target_os = "windows")]
  let mut cmd = {
    let mut c = std::process::Command::new("explorer");
    c.arg(format!("/select,{}", path.display()));
    c
  };
  #[cfg(not(any(target_os = "macos", target_os = "windows")))]
  let mut cmd = {
    let mut c = std::process::Command::new("xdg-open");
    c.arg(path.parent().unwrap_or(path));
    c
  };
  let status = cmd
    .status()
    .map_err(|e| format!("launching file explorer: {e}"))?;
  if status.success() {
    Ok(())
  } else {
    Err("file explorer exited nonzero".into())
  }
}

/// Open a file in $VISUAL (then $EDITOR), announcing which one
/// won before handing over the terminal. The editor inherits
/// stdio and is waited on, so terminal editors behave.
pub(crate) fn open_in_editor(path: &std::path::Path) -> Result<(), String> {
  let sty = style::Style::detect();
  let found = ["VISUAL", "EDITOR"].iter().find_map(|var| {
    std::env::var(var)
      .ok()
      .filter(|v| !v.trim().is_empty())
      .map(|v| (*var, v))
  });
  let Some((var, editor)) = found else {
    return Err("no $VISUAL or $EDITOR set; use --show instead".into());
  };
  println!("{}\n", sty.dim(&format!("${var} = {editor}")));
  // The variable may carry flags ("code --wait"): first token is
  // the binary, the rest pass through.
  let mut parts = editor.split_whitespace();
  let bin = parts.next().ok_or("empty editor value")?;
  let status = std::process::Command::new(bin)
    .args(parts)
    .arg(path)
    .status()
    .map_err(|e| format!("launching {editor:?}: {e}"))?;
  if status.success() {
    Ok(())
  } else {
    Err(format!("{editor} exited nonzero"))
  }
}

/// Quote a path for copy-paste when a HUMAN is reading (the
/// macOS data dir contains a space) but print it bare into a
/// pipe or command substitution, where literal quotes would
/// corrupt the path. Same tty rule the color system uses.
pub(crate) fn shell_quote(path: &str) -> String {
  use std::io::IsTerminal;
  let plain = path
    .bytes()
    .all(|b| b.is_ascii_alphanumeric() || b"/._-+:@%".contains(&b));
  if plain || !std::io::stdout().is_terminal() {
    path.to_string()
  } else {
    format!("'{}'", path.replace('\'', "'\\''"))
  }
}

/// Render a stored UTC timestamp in the machine's local time
/// for interactive display. Storage stays strict UTC (D-005);
/// this is presentation only. Non-unix or unparseable input
/// passes through unchanged.
pub(crate) fn local_display(iso_utc: &str) -> String {
  #[cfg(unix)]
  {
    let Some(ms) = kumbarium_util::parse_iso8601_ms(iso_utc) else {
      return iso_utc.to_string();
    };
    let secs = ms.div_euclid(1000) as libc::time_t;
    let mut tm: libc::tm = unsafe { std::mem::zeroed() };
    let ok = unsafe { !libc::localtime_r(&secs, &mut tm).is_null() };
    if !ok {
      return iso_utc.to_string();
    }
    format!(
      "{:04}-{:02}-{:02} {:02}:{:02}:{:02}",
      tm.tm_year + 1900,
      tm.tm_mon + 1,
      tm.tm_mday,
      tm.tm_hour,
      tm.tm_min,
      tm.tm_sec
    )
  }
  #[cfg(not(unix))]
  {
    iso_utc.to_string()
  }
}

/// The terminal's column count, when stdout is a terminal.
pub(crate) fn term_width() -> Option<usize> {
  #[cfg(unix)]
  {
    use std::io::IsTerminal;
    if !std::io::stdout().is_terminal() {
      return None;
    }
    let mut ws: libc::winsize = unsafe { std::mem::zeroed() };
    let ok =
      unsafe { libc::ioctl(libc::STDOUT_FILENO, libc::TIOCGWINSZ, &mut ws) }
        == 0;
    (ok && ws.ws_col > 0).then_some(ws.ws_col as usize)
  }
  #[cfg(not(unix))]
  {
    None
  }
}

/// Word-wrap `text` to `width` columns; a word longer than the
/// width is hard-cut rather than overflowing.
pub(crate) fn wrap_words(text: &str, width: usize) -> Vec<String> {
  let mut lines = Vec::new();
  let mut current = String::new();
  for word in text.split_whitespace() {
    let mut word = word;
    while word.len() > width {
      if !current.is_empty() {
        lines.push(std::mem::take(&mut current));
      }
      let (head, rest) = word.split_at(width);
      lines.push(head.to_string());
      word = rest;
    }
    let sep = if current.is_empty() { 0 } else { 1 };
    if !current.is_empty() && current.len() + sep + word.len() > width {
      lines.push(std::mem::take(&mut current));
    }
    if !current.is_empty() {
      current.push(' ');
    }
    current.push_str(word);
  }
  if !current.is_empty() {
    lines.push(current);
  }
  if lines.is_empty() {
    lines.push(String::new());
  }
  lines
}

/// Expand a leading `~/` (or bare `~`) so a quoted --out path
/// still lands where the human meant.
pub(crate) fn expand_home(raw: &str) -> std::path::PathBuf {
  if let Some(rest) = raw.strip_prefix("~/")
    && let Ok(home) = std::env::var("HOME")
  {
    return std::path::PathBuf::from(home).join(rest);
  }
  if raw == "~"
    && let Ok(home) = std::env::var("HOME")
  {
    return std::path::PathBuf::from(home);
  }
  std::path::PathBuf::from(raw)
}

pub(crate) fn fail(message: &str) -> ExitCode {
  eprintln!("kumbarium: {message}");
  ExitCode::FAILURE
}
