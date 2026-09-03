#!/usr/bin/env python3
"""Cargo.toml consistency across the workspace.

Per crate manifest, checks:
- `publish = false` -- these are internal crates, never published.
- an `edition` is set (ideally `edition.workspace = true`) -- no crate
  drifts onto rustc's default edition.
- a dep that ALSO exists in `[workspace.dependencies]` is inherited with
  `name.workspace = true`, not re-pinned with a local version literal
  (which drifts the version across crates).

Line-based (no toml dependency): it tracks the current `[section]` and the
`name = ...` shape. Path deps (`{ path = ... }`) and workspace-inherited
deps are fine; a bare/`{ version = }` literal whose name is a workspace dep
is the drift signal.

Exit 0 clean, 1 on any finding, 2 on a usage error.

Usage: scripts/cargo_hygiene.py
"""

import os
import re
import sys

DEP_SECTIONS = {"dependencies", "dev-dependencies", "build-dependencies"}
SECTION = re.compile(r"^\[([^\]]+)\]")
# `name = ...` at the start of a line (a dependency or a key).
KEY = re.compile(r'^([A-Za-z0-9_-]+)(\.workspace)?\s*=\s*(.*)$')


def manifests():
  out = []
  for base in ["crates", "."]:
    if not os.path.isdir(base):
      continue
    for name in sorted(os.listdir(base)):
      m = os.path.join(base, name, "Cargo.toml")
      if os.path.isfile(m):
        out.append(m)
  return out


def read(path):
  with open(path, "r", encoding="utf-8", errors="strict") as handle:
    return handle.read().split("\n")


def workspace_deps():
  """Names declared in the root [workspace.dependencies]."""
  names = set()
  if not os.path.isfile("Cargo.toml"):
    return names
  section = None
  for line in read("Cargo.toml"):
    s = SECTION.match(line.strip())
    if s:
      section = s.group(1)
      continue
    if section == "workspace.dependencies":
      k = KEY.match(line.strip())
      if k:
        names.add(k.group(1))
  return names


def check(path, ws_deps):
  """Return a list of finding strings for one manifest."""
  lines = read(path)
  text = "\n".join(lines)
  findings = []
  is_package = "[package]" in text
  if is_package:
    if "publish = false" not in text:
      findings.append(f"{path}: [package] missing `publish = false`")
    if "edition" not in text:
      findings.append(f"{path}: [package] sets no `edition`")

  section = None
  for i, raw in enumerate(lines, 1):
    line = raw.strip()
    s = SECTION.match(line)
    if s:
      section = s.group(1)
      continue
    if section not in DEP_SECTIONS:
      continue
    k = KEY.match(line)
    if not k:
      continue
    name, workspace_key, rhs = k.group(1), k.group(2), k.group(3)
    if workspace_key or "workspace = true" in rhs or "path =" in rhs:
      continue  # inherited or a local path dep -- fine
    if name in ws_deps:
      findings.append(
        f"{path}:{i}: `{name}` is a workspace dep -- inherit it with "
        f"`{name}.workspace = true`, not a local version"
      )
  return findings


def main():
  try:
    ws = workspace_deps()
    findings = []
    for path in manifests():
      findings.extend(check(path, ws))
  except (OSError, UnicodeDecodeError) as err:
    print(f"cargo_hygiene: {err}", file=sys.stderr)
    return 2

  for f in findings:
    print(f)
  if findings:
    print(f"\ncargo_hygiene: {len(findings)} finding(s).", file=sys.stderr)
    return 1
  print("cargo_hygiene: clean.")
  return 0


if __name__ == "__main__":
  sys.exit(main())
