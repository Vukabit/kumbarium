#!/usr/bin/env python3
"""The Kumbarium 80-column lint gate for text files.

`rustfmt` targets 80 columns on `.rs` but treats `max_width` as a soft
goal (it won't split some long strings / paths). This gate is the HARD
80-column enforcer across every text file under the repo, `.rs` included
(display width, not just rustfmt's soft pass) plus `.md`, `.toml`, `.py`,
`.wgsl`, and the rest, because the project's rule is an 80-column max
width on every file under the repo, measured in DISPLAY COLUMNS.

Width is DISPLAY columns: East-Asian wide / fullwidth code points count 2,
zero-width / combining marks count 0, everything else (an em-dash
included) counts 1.

EXEMPTIONS (never linted; reformatting them is wrong):
  - Gitignored / untracked-ignored files. The whole-tree scan lints only
    what git considers part of the repo (tracked + untracked, minus
    ignored: the gitignored per-developer config + internal design docs,
    ...), matching CI, which only ever sees committed + trackable
    content. Pass an explicit PATH to lint a file regardless of git status.
  - Byte-exact data directories: `assets/` (vendored fonts / SVGs /
    license text, provenance-hashed), `testdata/` (KAT / reference
    vectors), and `proptest-regressions/` (proptest's generated failure-
    seed corpus). All must stay byte-exact; wrapping their long data lines
    is wrong, and none is prose/source the 80-column rule is meant for.
  - Machine-generated files (a leading `// @generated` / `# @generated`
    marker, or a name matching a GENERATED_NAMES entry). Regenerate them;
    do not hand-wrap.
  - Binary files, and the usual build/VCS dirs (`target`, `.git`, ...).

Exit status is 0 when clean, 1 when any file has an over-width line, 2 on
a usage error, so it drops straight into CI next to fmt / clippy.

Usage:
  scripts/width_gate.py            # scan the whole working tree
  scripts/width_gate.py PATH ...   # scan only these files / dirs
"""

import os
import subprocess
import sys
import unicodedata

MAX_WIDTH = 80

# Directory names pruned entirely from the walk.
SKIP_DIRS = {".git", "target", "node_modules", ".vendor", "vendor"}

# Text extensions we lint. Anything else is treated as binary / opaque and
# skipped (a font, an image, a lockfile-style blob).
TEXT_EXTS = {
  ".rs", ".md", ".toml", ".py", ".wgsl", ".sh", ".yml", ".yaml",
  ".txt", ".json", ".cfg", ".ron", ".css", ".html",
}

# Generated files exempted by name (belt to the `@generated` marker's
# suspenders, for generators that cannot emit a leading marker line).
GENERATED_NAMES = set()


def display_width(line):
  """DISPLAY columns of `line`: wide=2, zero-width/combining=0, else 1."""
  width = 0
  for ch in line:
    if unicodedata.combining(ch):
      continue
    ea = unicodedata.east_asian_width(ch)
    width += 2 if ea in ("W", "F") else 1
  return width


def is_data_dir(path):
  """True for a path inside a byte-exact data directory (`assets/`,
  `testdata/`, `proptest-regressions/`): data blobs, not width-gated."""
  parts = path.replace(os.sep, "/").split("/")
  return bool({"assets", "testdata", "proptest-regressions"} & set(parts))


def is_generated(path):
  """True if the file is machine-generated (name allowlist or a leading
  `@generated` marker in the first few lines)."""
  if os.path.basename(path) in GENERATED_NAMES:
    return True
  try:
    with open(path, "r", encoding="utf-8", errors="strict") as handle:
      for _ in range(3):
        line = handle.readline()
        if not line:
          break
        if "@generated" in line:
          return True
  except (OSError, UnicodeDecodeError):
    return False
  return False


def lint_file(path):
  """Return a list of (line_no, width, text) for over-width lines, or []
  when the file is clean / exempt / not lintable text."""
  if is_data_dir(path) or is_generated(path):
    return []
  try:
    with open(path, "r", encoding="utf-8", errors="strict") as handle:
      lines = handle.read().split("\n")
  except (OSError, UnicodeDecodeError):
    return []  # binary or unreadable -> not our concern
  over = []
  for i, line in enumerate(lines, 1):
    width = display_width(line)
    if width > MAX_WIDTH:
      over.append((i, width, line))
  return over


def git_listed_files():
  """Files git considers part of the repo: tracked + untracked, minus
  ignored (respects `.gitignore`, global excludes, `.git/info/exclude`).
  Returns None when this is not a git repo / git is unavailable, so the
  caller can fall back to a raw filesystem walk."""
  try:
    out = subprocess.run(
      ["git", "ls-files", "--cached", "--others", "--exclude-standard",
       "-z"],
      capture_output=True,
      text=True,
      check=True,
    ).stdout
  except (OSError, subprocess.CalledProcessError):
    return None
  return [p for p in out.split("\0") if p]


def iter_targets(roots):
  """Yield every candidate text file under `roots` (files or dirs).

  A whole-tree scan (`.`) yields git's file list so gitignored per-
  developer config + the internal design docs are excluded, matching CI.
  An explicit file/dir root is walked directly, so a caller can lint any
  path regardless of git status."""
  for root in roots:
    if os.path.isfile(root):
      yield root
      continue
    listed = git_listed_files() if root in (".", "./") else None
    if listed is not None:
      for path in listed:
        if os.path.splitext(path)[1] in TEXT_EXTS:
          yield path
      continue
    for dirpath, dirnames, filenames in os.walk(root):
      dirnames[:] = [d for d in dirnames if d not in SKIP_DIRS]
      for name in filenames:
        if os.path.splitext(name)[1] in TEXT_EXTS:
          yield os.path.join(dirpath, name)


def main(argv):
  roots = argv[1:] or ["."]
  for root in roots:
    if not os.path.exists(root):
      print(f"width_gate: no such path: {root}", file=sys.stderr)
      return 2

  violations = 0
  for path in sorted(set(iter_targets(roots))):
    for line_no, width, text in lint_file(path):
      violations += 1
      rel = os.path.relpath(path)
      print(f"{rel}:{line_no}: {width} cols (max {MAX_WIDTH})")
      print(f"    {text}")

  if violations:
    print(f"\nwidth_gate: {violations} over-width line(s)", file=sys.stderr)
    return 1
  return 0


if __name__ == "__main__":
  sys.exit(main(sys.argv))
