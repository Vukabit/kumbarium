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

/// One column of a table. A table's whole geometry is a
/// `&[Col]`: the header, every cell pad, and the wrap column
/// all derive from the same spec, so they cannot drift apart.
/// (The recurring bug this retires: headers space-padded by
/// hand to widths the rows quietly stopped using, and wrapped
/// rows continuing at column zero.)
pub(crate) struct Col {
  pub title: &'static str,
  pub width: usize,
}

/// The header line for a spec: titles padded to their widths,
/// single-space separated; the LAST column never pads, it runs
/// to the margin. Style it dim at the call site.
pub(crate) fn table_header(cols: &[Col]) -> String {
  let mut out = String::new();
  for (i, c) in cols.iter().enumerate() {
    if i + 1 == cols.len() {
      out.push_str(c.title);
    } else {
      out.push_str(&format!("{:<w$} ", c.title, w = c.width));
    }
  }
  out
}

/// Pad cell text to column `i`'s spec width. Pad FIRST, style
/// after: ANSI escapes are zero display width but count as
/// bytes, so `{:<w$}` over styled text misaligns.
pub(crate) fn cell(cols: &[Col], i: usize, text: &str) -> String {
  format!("{:<w$}", text, w = cols[i].width)
}

/// The display column where the last, free-running field
/// starts: every earlier width plus its separator. This is the
/// hanging indent for wrapped rows.
pub(crate) fn body_col(cols: &[Col]) -> usize {
  cols[..cols.len() - 1].iter().map(|c| c.width + 1).sum()
}

/// Wrap a row's last field for the terminal: element 0 is the
/// remainder printed on the row line itself, the rest arrive
/// pre-indented to `col` so a wrapped row still reads as one
/// row. Piped output is always a single line (rows stay
/// grep-able for scripts).
pub(crate) fn hang(col: usize, body: &str) -> Vec<String> {
  match term_width().filter(|w| *w > col + 16) {
    Some(w) => {
      let chunks = wrap_words(body, w - col);
      let mut out = vec![chunks.first().cloned().unwrap_or_default()];
      for chunk in chunks.iter().skip(1) {
        out.push(format!("{:col$}{chunk}", ""));
      }
      out
    }
    None => vec![body.to_string()],
  }
}

/// A free paragraph under a uniform indent, wrapped to the
/// terminal (blank source lines survive as paragraph breaks);
/// piped output keeps the source lines verbatim, indented.
pub(crate) fn indent_block(indent: usize, text: &str) -> Vec<String> {
  let width = term_width().filter(|w| *w > indent + 16);
  let mut out = Vec::new();
  for line in text.lines() {
    if line.trim().is_empty() {
      out.push(String::new());
      continue;
    }
    match width {
      Some(w) => {
        for chunk in wrap_words(line, w - indent) {
          out.push(format!("{:indent$}{chunk}", ""));
        }
      }
      None => out.push(format!("{:indent$}{line}", "")),
    }
  }
  out
}

#[cfg(test)]
mod tests {
  use super::*;

  const SPEC: &[Col] = &[
    Col {
      title: "id",
      width: 8,
    },
    Col {
      title: "kind",
      width: 13,
    },
    Col {
      title: "detail",
      width: 0,
    },
  ];

  #[test]
  fn geometry_derives_from_one_spec() {
    assert_eq!(table_header(SPEC), "id       kind          detail");
    assert_eq!(cell(SPEC, 0, "abcd1234"), "abcd1234");
    assert_eq!(cell(SPEC, 1, "recall"), "recall       ");
    // The wrap column is the header's own arithmetic: no
    // hand-counted "8+2 + 13+1" comments to go stale.
    assert_eq!(body_col(SPEC), 9 + 14);
  }

  #[test]
  fn hang_is_single_line_when_piped() {
    // Tests run piped (term_width None): one grep-able line.
    let lines = hang(23, &"long ".repeat(50));
    assert_eq!(lines.len(), 1);
  }
}
