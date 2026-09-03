#!/usr/bin/env python3
"""Catch leftover debug / scaffolding in SHIPPED library code.

`dbg!`, `todo!`, `unimplemented!`, and stray `println!` / `eprintln!` are
review scaffolding or unfinished stubs; none should ship in a library's
non-test code. This scans `crates/*/src/**.rs` and flags them, using the
tests-last convention to skip the `#[cfg(test)]` region (where a `println!`
in a test is fine). CLI crates (`xtask`) and `examples/` are out of scope
by construction: `src/` excludes `examples/`, and this scans `crates/`
only, not `xtask`, where progress prints are legitimate.

Exit 0 clean, 1 on any hit, 2 on a usage error.

Usage: scripts/no_scaffolding.py
"""

import os
import re
import sys

# Scaffolding macros that must not ship in library non-test code.
SCAFFOLDING = re.compile(
  r"\b(dbg|todo|unimplemented|println|eprintln)!\s*\("
)
CFG_TEST = re.compile(r"#\[cfg\(test\)\]")


# The CLI binary crate is exempt: printing to the terminal is its job.
# Library crates (kumbarium-*) stay in scope.
CLI_CRATES = {"kumbarium"}


def src_files():
  """Every `.rs` under a crate's `src/` (excludes `examples/`, `tests/`)."""
  out = []
  base = "crates"
  if not os.path.isdir(base):
    return out
  for name in sorted(os.listdir(base)):
    if name in CLI_CRATES:
      continue
    src = os.path.join(base, name, "src")
    if not os.path.isdir(src):
      continue
    for dirpath, _dirs, files in os.walk(src):
      for fname in files:
        if fname.endswith(".rs"):
          out.append(os.path.join(dirpath, fname))
  return out


def main():
  hits = 0
  for path in src_files():
    try:
      with open(path, "r", encoding="utf-8", errors="strict") as handle:
        lines = handle.read().split("\n")
    except (OSError, UnicodeDecodeError):
      continue
    for i, line in enumerate(lines, 1):
      # Tests-last: once the test region starts, stop; the rest is test
      # code where these macros are fine.
      if CFG_TEST.search(line):
        break
      # Skip a `//`-comment mention.
      code = line.split("//", 1)[0]
      m = SCAFFOLDING.search(code)
      if m:
        hits += 1
        print(f"{path}:{i}: `{m.group(1)}!` in shipped library code")
        print(f"    {line.strip()}")

  if hits:
    print(f"\nno_scaffolding: {hits} scaffolding hit(s).", file=sys.stderr)
    return 1
  print("no_scaffolding: clean; no debug/stub macros in library code.")
  return 0


if __name__ == "__main__":
  sys.exit(main())
