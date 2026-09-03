//! Line-oriented markdown highlighting for memory bodies:
//! deliberately NOT a markdown parser. Each line is styled by
//! its shape (heading, bullet, quote, fence) and a small inline
//! pass handles `code`, **bold**, and [[wiki-links]]. Unbalanced
//! markers render literally; hostile input can only ever look
//! plain, never break.

use super::style::Style;

/// Render a memory body for terminal display. With style off
/// this is the identity function.
pub fn render(body: &str, sty: &Style) -> String {
  if !sty.on {
    return body.to_string();
  }
  let mut out = Vec::new();
  let mut in_fence = false;
  for line in body.lines() {
    let t = line.trim_start();
    if t.starts_with("```") {
      in_fence = !in_fence;
      out.push(sty.dim(line));
    } else if in_fence {
      out.push(sty.dim(line));
    } else if t.starts_with('#') {
      out.push(sty.bold(&sty.magenta(line)));
    } else if t.starts_with("> ") || t == ">" {
      out.push(sty.dim(line));
    } else if let Some(rest) = bullet_split(line) {
      let (marker, tail) = rest;
      out.push(format!("{}{}", sty.yellow(marker), inline(tail, sty)));
    } else {
      out.push(inline(line, sty));
    }
  }
  out.join("\n")
}

/// Split a list line into (indent + marker, rest), or None.
fn bullet_split(line: &str) -> Option<(&str, &str)> {
  let indent = line.len() - line.trim_start().len();
  let t = &line[indent..];
  for marker in ["- ", "* ", "+ "] {
    if let Some(rest) = t.strip_prefix(marker) {
      let cut = indent + marker.len();
      let _ = rest;
      return Some((&line[..cut], &line[cut..]));
    }
  }
  None
}

/// Inline pass: `code` spans (cyan, styling suppressed inside),
/// then **bold** and [[wiki-links]] in plain segments.
fn inline(line: &str, sty: &Style) -> String {
  let mut out = String::new();
  let mut rest = line;
  while let Some(start) = rest.find('`') {
    let after = &rest[start + 1..];
    let Some(end) = after.find('`') else { break };
    out.push_str(&emphasis(&rest[..start], sty));
    out.push_str(&sty.cyan(&format!("`{}`", &after[..end])));
    rest = &after[end + 1..];
  }
  out.push_str(&emphasis(rest, sty));
  out
}

/// **bold** and [[wiki-link]] spans in a code-free segment.
fn emphasis(seg: &str, sty: &Style) -> String {
  let mut out = String::new();
  let mut rest = seg;
  loop {
    let bold = rest.find("**");
    let wiki = rest.find("[[");
    match (bold, wiki) {
      (None, None) => {
        out.push_str(rest);
        return out;
      }
      (b, w) => {
        let take_bold =
          matches!((b, w), (Some(bp), Some(wp)) if bp <= wp) || w.is_none();
        if take_bold {
          let bp = b.unwrap();
          let after = &rest[bp + 2..];
          let Some(end) = after.find("**") else {
            out.push_str(rest);
            return out;
          };
          out.push_str(&rest[..bp]);
          out.push_str(&sty.bold(&after[..end]));
          rest = &after[end + 2..];
        } else {
          let wp = wiki.unwrap();
          let after = &rest[wp + 2..];
          let Some(end) = after.find("]]") else {
            out.push_str(rest);
            return out;
          };
          out.push_str(&rest[..wp]);
          out.push_str(&sty.cyan(&format!("[[{}]]", &after[..end])));
          rest = &after[end + 2..];
        }
      }
    }
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  const ON: Style = Style { on: true };
  const OFF: Style = Style { on: false };

  #[test]
  fn off_is_identity() {
    let body = "## head\n- item with `code` and **bold**";
    assert_eq!(render(body, &OFF), body);
  }

  #[test]
  fn shapes_get_their_styles() {
    let out = render("## heading\n- bullet\n> quote\n```\nfenced\n```", &ON);
    let lines: Vec<&str> = out.lines().collect();
    assert!(lines[0].contains("\x1b[35m"), "heading magenta");
    assert!(lines[1].starts_with("\x1b[33m"), "bullet yellow");
    assert!(lines[2].starts_with("\x1b[2m"), "quote dim");
    assert!(lines[4].starts_with("\x1b[2m"), "fence body dim");
  }

  #[test]
  fn inline_spans_style_and_unbalanced_stay_literal() {
    let out = render("see `x` and **y** and [[z]]", &ON);
    assert!(out.contains("\x1b[36m`x`"));
    assert!(out.contains("\x1b[1my"));
    assert!(out.contains("\x1b[36m[[z]]"));
    // Unbalanced markers render as-is, uncolored.
    let plain = render("a ** b [[ c ` d", &ON);
    assert!(plain.contains("** b [[ c ` d"));
  }

  #[test]
  fn code_spans_suppress_inner_styling() {
    let out = render("`**not bold**`", &ON);
    assert!(!out.contains("\x1b[1m"), "no bold inside code");
  }
}
