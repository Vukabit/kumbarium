//! Fact content as HTML: the markdown shapes agents actually
//! write (headings, lists, quotes, fences, tables, rules, `code`,
//! **bold**, *em*, links), plus cross-references:
//! [[wiki]] names, D-numbers, short ids, and shelf paths resolve
//! to pages in the build. NOT CommonMark, and deliberately
//! forgiving: an unbalanced marker renders literally, and hostile
//! input can only ever look plain. Every byte of source text
//! leaves through `esc`; only http(s) and mailto links are ever
//! emitted as hrefs.

use super::esc;

/// Something in fact text that may name a page in the build.
pub(super) enum Ref<'s> {
  /// `[[name]]`.
  Wiki(&'s str),
  /// A bare token or a whole `code` span: a D-number, a short
  /// or full id, a shelf path.
  Token(&'s str),
}

/// How text finds pages: `resolve` answers a root-relative URL,
/// `root` climbs from the current page to the build root.
pub(super) struct Refs<'a> {
  pub root: &'a str,
  pub resolve: &'a dyn Fn(Ref<'_>) -> Option<String>,
}

/// Render a markdown body to HTML blocks.
pub(super) fn render(src: &str, refs: &Refs) -> String {
  let lines: Vec<&str> = src.lines().collect();
  let mut out = String::new();
  blocks(&lines, refs, &mut out);
  out
}

fn blocks(lines: &[&str], refs: &Refs, out: &mut String) {
  let mut para: Vec<&str> = Vec::new();
  let mut i = 0;
  while i < lines.len() {
    let line = lines[i];
    let t = line.trim();
    if t.is_empty() {
      flush_para(&mut para, refs, out);
      i += 1;
      continue;
    }
    if let Some(fence) = fence_open(t) {
      flush_para(&mut para, refs, out);
      let mut body = Vec::new();
      i += 1;
      while i < lines.len() && !lines[i].trim_start().starts_with(fence) {
        body.push(lines[i]);
        i += 1;
      }
      i += 1;
      out.push_str("<pre class=\"code\"><code>");
      out.push_str(&esc(&body.join("\n")));
      out.push_str("</code></pre>\n");
      continue;
    }
    if let Some((level, text)) = heading(t) {
      flush_para(&mut para, refs, out);
      // Fact headings sit under the page's own h1/h2.
      let h = (level + 2).min(6);
      out.push_str(&format!(
        "<h{h} class=\"md\">{}</h{h}>\n",
        inline(text, refs)
      ));
      i += 1;
      continue;
    }
    if is_rule(t) {
      flush_para(&mut para, refs, out);
      out.push_str("<hr>\n");
      i += 1;
      continue;
    }
    if t.starts_with('>') {
      flush_para(&mut para, refs, out);
      let mut inner = Vec::new();
      while i < lines.len() && lines[i].trim_start().starts_with('>') {
        let s = &lines[i].trim_start()[1..];
        inner.push(s.strip_prefix(' ').unwrap_or(s));
        i += 1;
      }
      out.push_str("<blockquote>\n");
      blocks(&inner, refs, out);
      out.push_str("</blockquote>\n");
      continue;
    }
    if list_marker(line).is_some() {
      flush_para(&mut para, refs, out);
      i = list(lines, i, refs, out);
      continue;
    }
    if t.starts_with('|') && i + 1 < lines.len() && is_table_sep(lines[i + 1]) {
      flush_para(&mut para, refs, out);
      i = table(lines, i, refs, out);
      continue;
    }
    para.push(line);
    i += 1;
  }
  flush_para(&mut para, refs, out);
}

/// Agents write both hard-wrapped prose and one-thought-per-line
/// notes. A long line that stops mid-sentence is a wrap (joined
/// with a space); anything else keeps its line break.
fn soft_break(prev: &str) -> bool {
  let t = prev.trim_end();
  !prev.ends_with("  ")
    && t.chars().count() >= 50
    && !t.ends_with(['.', ':', '!', '?', ';'])
}

fn flush_para(para: &mut Vec<&str>, refs: &Refs, out: &mut String) {
  if para.is_empty() {
    return;
  }
  out.push_str("<p>");
  for (k, line) in para.iter().enumerate() {
    if k > 0 {
      out.push_str(if soft_break(para[k - 1]) {
        " "
      } else {
        "<br>\n"
      });
    }
    out.push_str(&inline(line.trim(), refs));
  }
  out.push_str("</p>\n");
  para.clear();
}

fn fence_open(t: &str) -> Option<&'static str> {
  if t.starts_with("```") {
    Some("```")
  } else if t.starts_with("~~~") {
    Some("~~~")
  } else {
    None
  }
}

/// `## text` (1 to 6 marks, then a space): (level, text). A bare
/// `#tag` is not a heading.
pub(super) fn heading(t: &str) -> Option<(usize, &str)> {
  let level = t.bytes().take_while(|b| *b == b'#').count();
  if level == 0 || level > 6 {
    return None;
  }
  let rest = &t[level..];
  if rest.is_empty() {
    return Some((level, ""));
  }
  rest
    .strip_prefix(' ')
    .map(|r| (level, r.trim().trim_end_matches('#').trim_end()))
}

fn is_rule(t: &str) -> bool {
  let squashed: String = t.chars().filter(|c| !c.is_whitespace()).collect();
  squashed.len() >= 3
    && ["-", "*", "_"]
      .iter()
      .any(|m| squashed.chars().all(|c| c.to_string() == *m))
}

/// A list line: (indent, ordered, byte offset where text starts).
fn list_marker(line: &str) -> Option<(usize, bool, usize)> {
  let indent_bytes = line.len() - line.trim_start().len();
  let indent: usize = line[..indent_bytes]
    .chars()
    .map(|c| if c == '\t' { 4 } else { 1 })
    .sum();
  let t = &line[indent_bytes..];
  for m in ["- ", "* ", "+ "] {
    if t.starts_with(m) {
      return Some((indent, false, indent_bytes + m.len()));
    }
  }
  let digits = t.bytes().take_while(u8::is_ascii_digit).count();
  if (1..=3).contains(&digits) {
    let after = &t[digits..];
    if after.starts_with(". ") || after.starts_with(") ") {
      return Some((indent, true, indent_bytes + digits + 2));
    }
  }
  None
}

/// Starts a block of its own (ends a list item's lazy text).
fn block_start(t: &str) -> bool {
  fence_open(t).is_some()
    || heading(t).is_some()
    || t.starts_with('>')
    || t.starts_with('|')
    || is_rule(t)
}

fn list(lines: &[&str], start: usize, refs: &Refs, out: &mut String) -> usize {
  let mut items: Vec<(usize, bool, Vec<&str>)> = Vec::new();
  let mut i = start;
  while i < lines.len() {
    let line = lines[i];
    if line.trim().is_empty() {
      let mut j = i + 1;
      while j < lines.len() && lines[j].trim().is_empty() {
        j += 1;
      }
      // A blank line continues the list only into a deeper item
      // or one of the same type (a `1.` after `-` starts anew).
      let next = lines.get(j).and_then(|l| list_marker(l));
      let continues = match (next, items.first()) {
        (Some((indent, ordered, _)), Some(&(base, base_ordered, _))) => {
          indent > base || ordered == base_ordered
        }
        _ => false,
      };
      if continues {
        i = j;
        continue;
      }
      break;
    }
    if let Some((indent, ordered, at)) = list_marker(line) {
      items.push((indent, ordered, vec![&line[at..]]));
      i += 1;
      continue;
    }
    let t = line.trim_start();
    let indented = t.len() < line.len();
    if !indented && block_start(t) {
      break;
    }
    if let Some(last) = items.last_mut() {
      last.2.push(t);
    }
    i += 1;
  }

  // Nesting by indent: deeper opens a list inside the open item,
  // shallower closes back out to the matching level.
  let mut stack: Vec<(usize, bool)> = Vec::new();
  for (indent, ordered, text) in items {
    while let Some(&(top, top_ordered)) = stack.last() {
      if indent < top && stack.len() > 1 {
        out.push_str(close_list(top_ordered));
        stack.pop();
      } else {
        break;
      }
    }
    match stack.last() {
      Some(&(top, _)) if indent <= top => out.push_str("</li>\n"),
      _ => {
        if !stack.is_empty() {
          out.push('\n');
        }
        out.push_str(if ordered { "<ol>\n" } else { "<ul>\n" });
        stack.push((indent, ordered));
      }
    }
    out.push_str("<li>");
    let mut body = String::new();
    for (k, l) in text.iter().enumerate() {
      if k > 0 {
        body.push_str(if soft_break(text[k - 1]) {
          " "
        } else {
          "<br>\n"
        });
      }
      body.push_str(&inline(l.trim(), refs));
    }
    // Task-list items: `[ ]` and `[x]` as check glyphs.
    let body = if let Some(rest) = body.strip_prefix("[ ] ") {
      format!("<span class=\"check\">\u{2610}</span> {rest}")
    } else if let Some(rest) = body
      .strip_prefix("[x] ")
      .or_else(|| body.strip_prefix("[X] "))
    {
      format!("<span class=\"check done\">\u{2611}</span> {rest}")
    } else {
      body
    };
    out.push_str(&body);
  }
  while let Some((_, ordered)) = stack.pop() {
    out.push_str(close_list(ordered));
  }
  i
}

fn close_list(ordered: bool) -> &'static str {
  if ordered {
    "</li>\n</ol>\n"
  } else {
    "</li>\n</ul>\n"
  }
}

fn is_table_sep(line: &str) -> bool {
  let t = line.trim();
  t.starts_with('|')
    && t.contains('-')
    && t.chars().all(|c| matches!(c, '|' | '-' | ':' | ' '))
}

fn cells(line: &str) -> Vec<&str> {
  let t = line.trim();
  let t = t.strip_prefix('|').unwrap_or(t);
  let t = t.strip_suffix('|').unwrap_or(t);
  t.split('|').map(str::trim).collect()
}

fn table(lines: &[&str], start: usize, refs: &Refs, out: &mut String) -> usize {
  out.push_str("<div class=\"table-wrap\"><table class=\"md\">\n<thead><tr>");
  for c in cells(lines[start]) {
    out.push_str(&format!("<th>{}</th>", inline(c, refs)));
  }
  out.push_str("</tr></thead>\n<tbody>\n");
  let mut i = start + 2;
  while i < lines.len() && lines[i].trim().starts_with('|') {
    out.push_str("<tr>");
    for c in cells(lines[i]) {
      out.push_str(&format!("<td>{}</td>", inline(c, refs)));
    }
    out.push_str("</tr>\n");
    i += 1;
  }
  out.push_str("</tbody></table></div>\n");
  i
}

/// Inline pass: `code` spans first (nothing styles inside them;
/// a span that is exactly a reference links whole), then the
/// emphasis and link spans in the plain segments between.
pub(super) fn inline(s: &str, refs: &Refs) -> String {
  let mut out = String::new();
  let mut rest = s;
  while let Some(start) = rest.find('`') {
    let after = &rest[start + 1..];
    let Some(end) = after.find('`') else { break };
    out.push_str(&spans(&rest[..start], refs));
    let code = &after[..end];
    let target = (!code.contains(char::is_whitespace))
      .then(|| (refs.resolve)(Ref::Token(code)))
      .flatten();
    match target {
      Some(url) => out.push_str(&format!(
        "<a class=\"ref\" href=\"{}{}\"><code>{}</code></a>",
        refs.root,
        esc(&url),
        esc(code)
      )),
      None => out.push_str(&format!("<code>{}</code>", esc(code))),
    }
    rest = &after[end + 1..];
  }
  out.push_str(&spans(rest, refs));
  out
}

/// **bold**, *em*, [[wiki]], [text](url), and bare http(s) URLs;
/// the text between them goes through `text` for cross-reference
/// tokens.
fn spans(s: &str, refs: &Refs) -> String {
  let mut out = String::new();
  let mut plain = 0;
  let mut i = 0;
  while i < s.len() {
    let rest = &s[i..];
    let hit: Option<(String, usize)> = if let Some(r) = rest.strip_prefix("**")
    {
      r.find("**")
        .filter(|&e| e > 0)
        .map(|e| (format!("<strong>{}</strong>", spans(&r[..e], refs)), e + 4))
    } else if let Some(r) = rest.strip_prefix("[[") {
      r.find("]]")
        .filter(|&e| e > 0)
        .map(|e| (wiki(&r[..e], refs), e + 4))
    } else if rest.starts_with('[') {
      md_link(rest)
    } else if let Some(r) = rest.strip_prefix('*') {
      let opens = r
        .chars()
        .next()
        .is_some_and(|c| !c.is_whitespace() && c != '*');
      opens
        .then(|| r.find('*'))
        .flatten()
        .filter(|&e| e > 0 && !r[..e].ends_with(char::is_whitespace))
        .map(|e| (format!("<em>{}</em>", spans(&r[..e], refs)), e + 2))
    } else if (rest.starts_with("https://") || rest.starts_with("http://"))
      && s[..i].chars().last().is_none_or(|c| !c.is_alphanumeric())
    {
      let end = rest
        .find(|c: char| c.is_whitespace() || matches!(c, '<' | '>' | '"' | ')'))
        .unwrap_or(rest.len());
      let url = rest[..end].trim_end_matches(['.', ',', ';', ':']);
      Some((
        format!("<a href=\"{0}\" rel=\"noopener\">{0}</a>", esc(url)),
        url.len(),
      ))
    } else {
      None
    };
    match hit {
      Some((html, len)) => {
        out.push_str(&text(&s[plain..i], refs));
        out.push_str(&html);
        i += len;
        plain = i;
      }
      None => i += rest.chars().next().map_or(1, char::len_utf8),
    }
  }
  out.push_str(&text(&s[plain..], refs));
  out
}

fn wiki(name: &str, refs: &Refs) -> String {
  match (refs.resolve)(Ref::Wiki(name.trim())) {
    Some(url) => format!(
      "<a class=\"wiki\" href=\"{}{}\">{}</a>",
      refs.root,
      esc(&url),
      esc(name)
    ),
    None => format!(
      "<span class=\"wiki missing\" title=\"not in this build\">{}</span>",
      esc(name)
    ),
  }
}

/// `[text](url)`, emitted only for http(s) and mailto targets;
/// anything else (javascript:, data:, relative) stays literal.
fn md_link(rest: &str) -> Option<(String, usize)> {
  let close = rest.find("](")?;
  let label = &rest[1..close];
  if label.is_empty() || label.contains('[') {
    return None;
  }
  let after = &rest[close + 2..];
  let end = after.find(')')?;
  let url = after[..end].trim();
  let lower = url.to_ascii_lowercase();
  let safe = lower.starts_with("https://")
    || lower.starts_with("http://")
    || lower.starts_with("mailto:");
  if !safe || url.contains(char::is_whitespace) {
    return None;
  }
  Some((
    format!(
      "<a href=\"{}\" rel=\"noopener\">{}</a>",
      esc(url),
      esc(label)
    ),
    close + 2 + end + 1,
  ))
}

/// Plain text: escaped, with every token the build can resolve
/// (D-numbers, short ids, shelf paths) turned into a link.
fn text(seg: &str, refs: &Refs) -> String {
  let is_tok =
    |c: char| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.' | '/');
  let mut ranges = Vec::new();
  let mut start = None;
  for (idx, c) in seg.char_indices() {
    if is_tok(c) {
      start.get_or_insert(idx);
    } else if let Some(a) = start.take() {
      ranges.push((a, idx));
    }
  }
  if let Some(a) = start {
    ranges.push((a, seg.len()));
  }
  let mut out = String::new();
  let mut last = 0;
  for (a, mut z) in ranges {
    while z > a && matches!(seg.as_bytes()[z - 1], b'.' | b'-' | b'/' | b'_') {
      z -= 1;
    }
    let tok = &seg[a..z];
    if tok.len() < 5 {
      continue;
    }
    if let Some(url) = (refs.resolve)(Ref::Token(tok)) {
      out.push_str(&esc(&seg[last..a]));
      out.push_str(&format!(
        "<a class=\"ref\" href=\"{}{}\">{}</a>",
        refs.root,
        esc(&url),
        esc(tok)
      ));
      last = z;
    }
  }
  out.push_str(&esc(&seg[last..]));
  out
}

/// Markdown marks stripped, whitespace collapsed: text for
/// titles, summaries, and the search index.
pub(super) fn plain(s: &str) -> String {
  let mut t = s.trim();
  if let Some((_, h)) = heading(t) {
    t = h;
  }
  for m in ["- ", "* ", "+ ", "> "] {
    if let Some(r) = t.strip_prefix(m) {
      t = r;
    }
  }
  let t = t
    .replace("**", "")
    .replace('`', "")
    .replace("[[", "")
    .replace("]]", "");
  t.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Cut at `max` chars on a word boundary, with an ellipsis.
pub(super) fn clip(s: &str, max: usize) -> String {
  if s.chars().count() <= max {
    return s.to_string();
  }
  let cut: String = s.chars().take(max).collect();
  let cut = match cut.rfind(' ') {
    Some(sp) if sp > max / 2 => cut[..sp].to_string(),
    _ => cut,
  };
  format!("{}\u{2026}", cut.trim_end_matches([',', ';', ':', ' ']))
}

/// The first sentence of `p` and what follows it; a sentence
/// ends at `. `, `? `, or `! ` past the first eight chars (so
/// `e.g. ` never ends one).
fn split_sentence(p: &str) -> (&str, &str) {
  let bytes = p.as_bytes();
  for i in 8..bytes.len().saturating_sub(1) {
    if matches!(bytes[i], b'.' | b'?' | b'!') && bytes[i + 1] == b' ' {
      return (&p[..=i], p[i + 2..].trim_start());
    }
  }
  (p, "")
}

/// A fact's (title, summary line), as a reference listing shows
/// it. A leading
/// heading is the title; otherwise the first sentence is. The
/// summary is the first sentence of what follows.
pub(super) fn title_and_summary(content: &str) -> (String, String) {
  let mut lines = content.lines().map(str::trim).filter(|l| !l.is_empty());
  let Some(first) = lines.next() else {
    return ("(empty)".into(), String::new());
  };
  let remaining: Vec<&str> = lines
    .filter(|l| fence_open(l).is_none() && !is_rule(l))
    .collect();
  let (title, mut rest) = if heading(first).is_some() {
    (plain(first), String::new())
  } else {
    let p = plain(first);
    let (t, r) = split_sentence(&p);
    (t.to_string(), r.to_string())
  };
  if rest.is_empty() {
    // The next paragraph: consecutive non-heading lines.
    let para: Vec<String> = remaining
      .iter()
      .skip_while(|l| heading(l).is_some())
      .take_while(|l| heading(l).is_none())
      .take(4)
      .map(|l| plain(l))
      .collect();
    rest = para.join(" ");
  }
  let (summary, _) = split_sentence(&rest);
  let title = if title.is_empty() {
    "(untitled)".into()
  } else {
    title
  };
  (clip(&title, 90), clip(summary, 170))
}

#[cfg(test)]
mod tests {
  use super::*;

  fn with_refs<T>(f: impl FnOnce(&Refs) -> T) -> T {
    let resolve = |r: Ref<'_>| -> Option<String> {
      match r {
        Ref::Wiki("known") => Some("fact/k.html".into()),
        Ref::Token("D-054") => Some("fact/d.html".into()),
        Ref::Token("1cb8e972") => Some("fact/1.html".into()),
        Ref::Token("project/a") => Some("shelf/project/a/index.html".into()),
        _ => None,
      }
    };
    f(&Refs {
      root: "../",
      resolve: &resolve,
    })
  }

  #[test]
  fn blocks_render_their_shapes() {
    let html = with_refs(|r| {
      render(
        concat!(
          "## Head\n\n- one\n- two\n  - nested\n- three\n\n",
          "1. first\n2. second\n\n> quoted\n\n```\n<raw>\n```\n\n",
          "| a | b |\n|---|---|\n| 1 | 2 |\n\n---\ntext",
        ),
        r,
      )
    });
    assert!(html.contains("<h4 class=\"md\">Head</h4>"));
    let nested = concat!(
      "<ul>\n<li>one</li>\n<li>two\n<ul>\n<li>nested</li>\n</ul>\n",
      "</li>\n<li>three</li>\n</ul>",
    );
    assert!(html.contains(nested), "{html}");
    assert!(html.contains("<ol>\n<li>first</li>\n<li>second</li>\n</ol>"));
    assert!(html.contains("<blockquote>\n<p>quoted</p>\n</blockquote>"));
    assert!(
      html.contains("<pre class=\"code\"><code>&lt;raw&gt;</code></pre>")
    );
    assert!(html.contains("<th>a</th><th>b</th>"));
    assert!(html.contains("<td>1</td><td>2</td>"));
    assert!(html.contains("<hr>"));
  }

  #[test]
  fn inline_spans_and_cross_references() {
    let html = with_refs(|r| {
      inline(
        concat!(
          "**b** *e* `code` [[known]] [[unknown]] ",
          "see D-054, 1cb8e972 and project/a.",
        ),
        r,
      )
    });
    assert!(html.contains("<strong>b</strong>"));
    assert!(html.contains("<em>e</em>"));
    assert!(html.contains("<code>code</code>"));
    assert!(
      html.contains("<a class=\"wiki\" href=\"../fact/k.html\">known</a>")
    );
    assert!(html.contains("wiki missing"));
    assert!(
      html.contains("<a class=\"ref\" href=\"../fact/d.html\">D-054</a>,")
    );
    assert!(html.contains("href=\"../fact/1.html\">1cb8e972</a>"));
    assert!(
      html.contains("href=\"../shelf/project/a/index.html\">project/a</a>.")
    );
  }

  #[test]
  fn hostile_input_stays_inert() {
    let html = with_refs(|r| {
      render(
        concat!(
          "<script>x</script>\n[click](javascript:alert(1))\n",
          "[ok](https://e.x/?a=\"b\")\n<img src=x onerror=y>",
        ),
        r,
      )
    });
    assert!(!html.contains("<script"));
    assert!(!html.contains("<img"));
    assert!(!html.contains("href=\"javascript"));
    assert!(html.contains("href=\"https://e.x/?a=&quot;b&quot;\""));
    // Unbalanced markers are literal.
    let lit = with_refs(|r| inline("a ** b [[ c ` d * e", r));
    assert_eq!(lit, "a ** b [[ c ` d * e");
  }

  #[test]
  fn wrapped_prose_joins_and_notes_keep_their_lines() {
    let html = with_refs(|r| {
      render(
        concat!(
          "this line is long enough to be a hard wrap and it keeps\n",
          "going on the next line.\n**Why:** short\n**How:** also",
        ),
        r,
      )
    });
    assert!(html.contains("keeps going"));
    assert!(html.contains("line.<br>\n<strong>Why:</strong> short<br>"));
  }

  #[test]
  fn titles_and_summaries_read_like_a_reference() {
    let (t, s) = title_and_summary(
      "## Architecture (as built)\n- Agent <=> Librarian. More.",
    );
    assert_eq!(t, "Architecture (as built)");
    assert_eq!(s, "Agent <=> Librarian.");
    let (t, s) =
      title_and_summary("The grelvix runs hot. It needs a fan. Always.");
    assert_eq!(
      (t.as_str(), s.as_str()),
      ("The grelvix runs hot.", "It needs a fan.")
    );
    let (t, _) = title_and_summary(&"word ".repeat(40));
    assert!(t.ends_with('\u{2026}') && t.chars().count() <= 91);
  }
}
