#!/usr/bin/env python3
"""The license gate: `cargo deny check`.

Runs cargo-deny against deny.toml, whose [licenses] section allowlists
permissive SPDX licenses compatible with this MIT/Apache-2.0 project. Any
dependency under a copyleft / source-available license (GPL / AGPL / LGPL /
SSPL / ...) is absent from the allowlist and FAILS here, so a license that
would encumber the project cannot enter the tree unreviewed. Also runs the
bans (wildcard requirements) and sources (unknown registry / git) checks
from deny.toml.

cargo-deny is a DEV tool, not a workspace dependency. If it is not installed,
this SKIPS with an install hint and exits 0, so the local gate stays green for
a dev without it; CI (with the tool installed) is where the policy is enforced.

Exit code is cargo-deny's when it runs (0 clean, non-zero on a policy breach);
0 when skipped, 2 on a usage error.

Usage: scripts/license_gate.py
"""

import os
import shutil
import subprocess
import sys

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))


def main():
  if sys.argv[1:]:
    print("Usage: scripts/license_gate.py")
    return 2
  if shutil.which("cargo-deny") is None:
    print(
      "SKIP: cargo-deny not installed "
      "(cargo install cargo-deny); licenses not checked."
    )
    return 0
  return subprocess.run(
    ["cargo", "deny", "check", "licenses", "bans", "sources"],
    cwd=ROOT,
  ).returncode


if __name__ == "__main__":
  sys.exit(main())
