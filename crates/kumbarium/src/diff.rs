//! Hand-rolled line diff (LCS): small, deterministic, and only
//! ever run on memory-sized texts, so the O(n*m) table is fine.
//! Presentation-layer code: lives in the CLI, not the store.

/// One diff line: ' ' unchanged, '-' removed, '+' added.
pub type DiffLine = (char, String);

/// Line-level diff from `old` to `new`.
pub fn lines(old: &str, new: &str) -> Vec<DiffLine> {
  let a: Vec<&str> = old.lines().collect();
  let b: Vec<&str> = new.lines().collect();
  // LCS length table.
  let mut t = vec![vec![0usize; b.len() + 1]; a.len() + 1];
  for i in (0..a.len()).rev() {
    for j in (0..b.len()).rev() {
      t[i][j] = if a[i] == b[j] {
        t[i + 1][j + 1] + 1
      } else {
        t[i + 1][j].max(t[i][j + 1])
      };
    }
  }
  let mut out = Vec::new();
  let (mut i, mut j) = (0, 0);
  while i < a.len() && j < b.len() {
    if a[i] == b[j] {
      out.push((' ', a[i].to_string()));
      i += 1;
      j += 1;
    } else if t[i + 1][j] >= t[i][j + 1] {
      out.push(('-', a[i].to_string()));
      i += 1;
    } else {
      out.push(('+', b[j].to_string()));
      j += 1;
    }
  }
  out.extend(a[i..].iter().map(|l| ('-', l.to_string())));
  out.extend(b[j..].iter().map(|l| ('+', l.to_string())));
  out
}

#[cfg(test)]
mod tests {
  use super::*;

  fn apply(diff: &[DiffLine]) -> (String, String) {
    let old: Vec<&str> = diff
      .iter()
      .filter(|(c, _)| *c != '+')
      .map(|(_, l)| l.as_str())
      .collect();
    let new: Vec<&str> = diff
      .iter()
      .filter(|(c, _)| *c != '-')
      .map(|(_, l)| l.as_str())
      .collect();
    (old.join("\n"), new.join("\n"))
  }

  #[test]
  fn diff_reconstructs_both_sides() {
    let old = "alpha\nbeta\ngamma\ndelta";
    let new = "alpha\nBETA\ngamma\nepsilon\ndelta";
    let d = lines(old, new);
    let (o, n) = apply(&d);
    assert_eq!(o, old);
    assert_eq!(n, new);
  }

  #[test]
  fn identical_texts_have_no_changes() {
    let d = lines("same\ntext", "same\ntext");
    assert!(d.iter().all(|(c, _)| *c == ' '));
  }

  #[test]
  fn disjoint_texts_are_full_replacement() {
    let d = lines("only old", "only new");
    let marks: Vec<char> = d.iter().map(|(c, _)| *c).collect();
    assert_eq!(marks, ['-', '+']);
  }

  #[test]
  fn empty_sides_behave() {
    assert!(
      lines("", "").iter().all(|(c, _)| *c == ' ') || lines("", "").is_empty()
    );
    let added = lines("", "a\nb");
    assert!(added.iter().all(|(c, _)| *c == '+'));
  }
}
