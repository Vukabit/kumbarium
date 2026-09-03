#!/usr/bin/env bash
# The local gate battery: run before a commit. Mirrors CI (fmt /
# clippy / test) plus the Kumbarium lint gates (80-column width,
# no-scaffolding, manifest hygiene, licenses). Runs EVERY gate,
# reports each pass/fail, and exits non-zero if any failed (it
# does not stop at the first failure, so one run surfaces every
# problem).
#
# Usage: scripts/gate.sh
set -u
cd "$(dirname "$0")/.." || exit 2

fail=0
run() {
  local name="$1"
  shift
  printf '\n=== %s ===\n' "$name"
  if "$@"; then
    printf '  PASS: %s\n' "$name"
  else
    printf '  FAIL: %s\n' "$name"
    fail=1
  fi
}

run "fmt"            cargo +nightly fmt --all --check
run "clippy"         cargo clippy --workspace --all-targets \
  -- -D warnings
run "test"           cargo test --workspace
run "width"          python3 scripts/width_gate.py
run "no-scaffolding" python3 scripts/no_scaffolding.py
run "cargo-hygiene"  python3 scripts/cargo_hygiene.py
# Supply chain: the license allowlist (cargo-deny). SKIPs cleanly
# when the dev tool is absent; CI enforces.
run "licenses"       python3 scripts/license_gate.py

printf '\n----------------------------------------\n'
if [ "$fail" -eq 0 ]; then
  printf 'gate: ALL PASS\n'
else
  printf 'gate: FAILURES above\n'
fi
exit "$fail"
