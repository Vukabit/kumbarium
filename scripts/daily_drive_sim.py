#!/usr/bin/env python3
"""Daily-drive simulator: fast-tracks the SYSTEMS half of the
observation period against a THROWAWAY library (KUMBARIUM_HOME),
never the real one. Simulated agents seed, recall, supersede,
retire, confirm, and link a synthetic corpus; a concurrency
storm runs parallel server processes; latency, retrieval
precision, growth, and error counts land in a markdown report.

What it cannot fast-track, by design: whether real LLMs recall
unprompted or store well. That stays a matter of real usage.

Usage:
  scripts/daily_drive_sim.py [--binary PATH] [--out REPORT.md]
"""

import argparse
import json
import os
import shutil
import statistics
import subprocess
import sys
import tempfile
import time

NAMESPACES = ["project/simfoo", "project/simbar", "project/simbaz"]
AGENTS = ["sim-claude", "sim-gemini", "sim-ollama"]
KINDS = ["preference", "project_state", "decision", "reference"]

SEED_COUNT = 300  # memories seeded across agents/namespaces
GOLDEN_COUNT = 60  # seeded facts later queried for precision
SUPERSEDES = 50
RETIRES = 20
CONFIRMS = 40
LINKS = 30
STORM_PROCS = 4
STORM_OPS = 50  # ops per concurrent process


class Session:
  """One serve process driven over stdio, timed per call."""

  def __init__(self, binary, home, agent):
    env = dict(os.environ, KUMBARIUM_HOME=home)
    self.proc = subprocess.Popen(
      [binary, "serve"],
      stdin=subprocess.PIPE,
      stdout=subprocess.PIPE,
      stderr=subprocess.DEVNULL,
      env=env,
      text=True,
    )
    self.next_id = 1
    self.latencies = {}
    self.errors = []
    self._request(
      "initialize",
      {
        "protocolVersion": "2025-06-18",
        "clientInfo": {"name": agent, "version": "sim"},
      },
    )

  def _request(self, method, params):
    rid = self.next_id
    self.next_id += 1
    msg = {
      "jsonrpc": "2.0",
      "id": rid,
      "method": method,
      "params": params,
    }
    start = time.perf_counter()
    self.proc.stdin.write(json.dumps(msg) + "\n")
    self.proc.stdin.flush()
    line = self.proc.stdout.readline()
    elapsed = (time.perf_counter() - start) * 1000.0
    return json.loads(line), elapsed

  def call(self, tool, args):
    reply, ms = self._request(
      "tools/call", {"name": tool, "arguments": args}
    )
    self.latencies.setdefault(tool, []).append(ms)
    result = reply.get("result", {})
    text = "\n".join(
      block.get("text", "") for block in result.get("content", [])
    )
    if result.get("isError"):
      self.errors.append(f"{tool}: {text.splitlines()[0]}")
    return text

  def close(self):
    self.proc.stdin.close()
    self.proc.wait(timeout=30)


def first_id(text):
  return text.split("id=")[1].split()[0] if "id=" in text else None


def paragraphs(token, n):
  body = f"The {token} subsystem decision and its rationale."
  para = f"{body} " + "Detail sentence for bulk. " * 6
  return "\n\n".join(f"{para} (part seed {i})" for i in range(n))


def seed(sessions, rng):
  ids, golden = [], []
  for i in range(SEED_COUNT):
    agent = i % len(AGENTS)
    ns = NAMESPACES[i % len(NAMESPACES)]
    token = f"factoken{i:04}"
    oversized = i % 10 == 0  # every 10th memory exercises split
    content = paragraphs(token, 6 if oversized else 1)
    text = sessions[agent].call(
      "remember",
      {
        "namespace": ns,
        "kind": KINDS[i % len(KINDS)],
        "content": content,
        "tags": [f"sim-{i % 7}"],
        "source": "daily_drive_sim",
      },
    )
    eid = first_id(text)
    ids.append((eid, ns, token))
  _ = rng
  return ids, []


def precision(sessions, golden):
  hits_top1 = hits_top3 = 0
  for i, (eid, ns, token) in enumerate(golden):
    text = sessions[i % len(sessions)].call(
      "recall",
      {"query": f"{token} subsystem rationale", "scope": ns},
    )
    returned = [
      line.split("id=")[1].split()[0]
      for line in text.splitlines()
      if "id=" in line
    ]
    if returned[:1] == [eid]:
      hits_top1 += 1
    if eid in returned[:3]:
      hits_top3 += 1
  return hits_top1, hits_top3


def churn(sessions, ids):
  for i in range(SUPERSEDES):
    eid, ns, token = ids[i]
    text = sessions[i % len(sessions)].call(
      "supersede",
      {
        "old_id": eid,
        "namespace": ns,
        "kind": "decision",
        "content": (
          f"The {token} subsystem decision and its rationale, "
          "revised in churn."
        ),
        "note": "sim churn revision",
      },
    )
    ids[i] = (first_id(text) or eid, ns, token)
  for i in range(CONFIRMS):
    eid, _, _ = ids[SUPERSEDES + i]
    sessions[i % len(sessions)].call("confirm", {"id": eid})
  for i in range(LINKS):
    a = ids[SUPERSEDES + i][0]
    b = ids[SUPERSEDES + i + 1][0]
    sessions[i % len(sessions)].call(
      "link", {"from_id": a, "to_id": b, "rel": "relates_to"}
    )


def storm(binary, home, ids):
  """Parallel serve processes hammering reads and writes."""
  workers = []
  for w in range(STORM_PROCS):
    workers.append(
      subprocess.Popen(
        [sys.executable, __file__, "--worker", str(w)],
        env=dict(
          os.environ,
          KUMBARIUM_HOME=home,
          SIM_BINARY=binary,
          SIM_IDS=",".join(e for e, _, _ in ids[:40]),
        ),
        stdout=subprocess.PIPE,
        text=True,
      )
    )
  messages = []
  for w in workers:
    out, _ = w.communicate(timeout=300)
    messages.extend(
      line for line in out.splitlines() if line.strip()
    )
  return messages


def worker(widx):
  binary = os.environ["SIM_BINARY"]
  home = os.environ["KUMBARIUM_HOME"]
  known = os.environ["SIM_IDS"].split(",")
  s = Session(binary, home, f"storm-{widx}")
  for i in range(STORM_OPS):
    if i % 3 == 0:
      s.call(
        "remember",
        {
          "namespace": NAMESPACES[i % len(NAMESPACES)],
          "kind": "reference",
          "content": f"storm {widx} fact number {i} stormtoken",
        },
      )
    else:
      s.call(
        "recall",
        {
          "query": f"factoken{i:04} subsystem",
          "scope": NAMESPACES[i % len(NAMESPACES)],
        },
      )
    if i % 10 == 0 and known[i % len(known)]:
      s.call("confirm", {"id": known[i % len(known)]})
  s.close()
  for e in s.errors:
    print(e)


def cli(binary, home, *args):
  return subprocess.run(
    [binary, *args],
    env=dict(os.environ, KUMBARIUM_HOME=home),
    capture_output=True,
    text=True,
  ).stdout


def stats_block(latencies):
  rows = []
  for tool in sorted(latencies):
    xs = sorted(latencies[tool])
    p95 = xs[int(len(xs) * 0.95) - 1] if len(xs) > 1 else xs[0]
    rows.append(
      f"| {tool:<9} | {len(xs):>5} | "
      f"{statistics.mean(xs):>8.2f} | {p95:>8.2f} |"
    )
  return rows


def main():
  ap = argparse.ArgumentParser()
  ap.add_argument("--binary", default="target/release/kumbarium")
  ap.add_argument("--out", default=None)
  ap.add_argument("--worker", default=None)
  opts = ap.parse_args()
  if opts.worker is not None:
    worker(int(opts.worker))
    return 0

  binary = os.path.abspath(opts.binary)
  home = tempfile.mkdtemp(prefix="kumbarium-sim-")
  started = time.strftime("%Y-%m-%d %H:%M:%S")
  wall = time.perf_counter()

  # Namespaces first (registered-only rule holds in the sim too).
  for ns in NAMESPACES:
    cli(binary, home, "namespace", "add", ns, "sim")

  sessions = [Session(binary, home, a) for a in AGENTS]
  ids, _ = seed(sessions, None)
  churn(sessions, ids)
  # Golden set built AFTER churn so expectations track the LIVE
  # ids (supersessions replace; recall correctly hides originals).
  golden = ids[:GOLDEN_COUNT]
  for i in range(RETIRES):
    eid = ids[SEED_COUNT - 1 - i][0]
    cli(binary, home, "retire", eid)
  top1, top3 = precision(sessions, golden)
  storm_errors = storm(binary, home, ids)
  backup_out = cli(binary, home, "backup")
  status_out = cli(binary, home, "status")
  for s in sessions:
    s.close()

  latencies, errors = {}, []
  for s in sessions:
    errors.extend(s.errors)
    for tool, xs in s.latencies.items():
      latencies.setdefault(tool, []).extend(xs)

  lib_kb = os.path.getsize(os.path.join(home, "library.db")) // 1024
  audit_kb = os.path.getsize(os.path.join(home, "audit.db")) // 1024
  elapsed = time.perf_counter() - wall

  report = [
    "<!-- @generated by scripts/daily_drive_sim.py -->",
    "# Daily-drive simulation report",
    "",
    f"Run: {started}; wall time {elapsed:.1f}s; throwaway home",
    "(deleted after run). Systems-side only: agent BEHAVIOR",
    "still requires real usage.",
    "",
    "## Workload",
    "",
    f"- {len(AGENTS)} agents, {len(NAMESPACES)} namespaces",
    f"- {SEED_COUNT} memories seeded (1 in 10 oversized ->",
    "  auto-split sets)",
    f"- {SUPERSEDES} noted supersessions, {RETIRES} retires,",
    f"  {CONFIRMS} confirms, {LINKS} links",
    f"- storm: {STORM_PROCS} concurrent server processes x",
    f"  {STORM_OPS} mixed ops (WAL multi-process, D-015)",
    "",
    "## Retrieval precision (golden queries)",
    "",
    f"- expected memory ranked #1: {top1}/{GOLDEN_COUNT}",
    f"- expected memory in top 3:  {top3}/{GOLDEN_COUNT}",
    "",
    "## Latency (ms per MCP call, seed+churn sessions)",
    "",
    "| tool      | calls |     mean |      p95 |",
    "|-----------|-------|----------|----------|",
    *stats_block(latencies),
    "",
    "## Concurrency storm",
    "",
    f"- tool-level failures across workers: "
    f"{len(storm_errors)}",
    *[f"  - {e}" for e in storm_errors[:10]],
    "",
    "## Integrity and growth",
    "",
    f"- library.db: {lib_kb} KB; audit.db: {audit_kb} KB",
    f"- backup: {backup_out.strip().splitlines()[0]}",
    "- status after run:",
    "",
    "```",
    status_out.rstrip(),
    "```",
    "",
    "## Errors in driven sessions",
    "",
  ]
  if errors:
    report += [f"- {e}" for e in errors[:20]]
    if len(errors) > 20:
      report.append(f"- ... and {len(errors) - 20} more")
  else:
    report.append("- none")
  report.append("")

  text = "\n".join(report)
  out = opts.out or time.strftime(
    "docs/reports/%Y-%m-%d-daily-drive-sim.md"
  )
  os.makedirs(os.path.dirname(out), exist_ok=True)
  with open(out, "w", encoding="utf-8") as fh:
    fh.write(text)
  shutil.rmtree(home, ignore_errors=True)
  print(text)
  print(f"\nreport: {out}", file=sys.stderr)
  return 1 if (errors or storm_errors) else 0


if __name__ == "__main__":
  sys.exit(main())
