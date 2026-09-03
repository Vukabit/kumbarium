#!/usr/bin/env python3
"""Persona harness: real LLM agents run multi-session arcs
against a sandboxed library; the witness grades the exam.
Design: docs/design/persona-harness.md (D-023).

Stdlib only. Providers degrade cleanly: the Anthropic realism
tier needs ANTHROPIC_API_KEY; the Ollama robustness floor needs
a local daemon; whichever is missing is skipped with a note.

Usage:
  scripts/persona_sim.py [--binary PATH] [--fixture TOML]...
    [--anthropic-model M] [--ollama-model M] [--out REPORT.md]
"""

import argparse
import glob
import json
import os
import shutil
import sqlite3
import subprocess
import sys
import tempfile
import time
import tomllib
import urllib.error
import urllib.request

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from daily_drive_sim import Session  # noqa: E402

MAX_TOOL_ROUNDS = 8  # per user turn
MAX_TOKENS = 1024  # per model call


def http_json(url, payload, headers):
  req = urllib.request.Request(
    url,
    data=json.dumps(payload).encode(),
    headers={"content-type": "application/json", **headers},
  )
  try:
    with urllib.request.urlopen(req, timeout=120) as resp:
      return json.loads(resp.read())
  except urllib.error.HTTPError as e:
    body = e.read().decode(errors="replace")[:300]
    raise RuntimeError(f"{url}: HTTP {e.code}: {body}") from e


class Anthropic:
  tier = "realism"

  def __init__(self, model):
    self.model = model
    self.key = os.environ.get("ANTHROPIC_API_KEY")
    self.in_tok = 0
    self.out_tok = 0

  def available(self):
    return bool(self.key)

  def turn(self, system, messages, tools):
    """One model call; returns (blocks, wants_tools)."""
    reply = http_json(
      "https://api.anthropic.com/v1/messages",
      {
        "model": self.model,
        "max_tokens": MAX_TOKENS,
        "system": system,
        "messages": messages,
        "tools": [
          {
            "name": t["name"],
            "description": t["description"],
            "input_schema": t["inputSchema"],
          }
          for t in tools
        ],
      },
      {"x-api-key": self.key, "anthropic-version": "2023-06-01"},
    )
    usage = reply.get("usage", {})
    self.in_tok += usage.get("input_tokens", 0)
    self.out_tok += usage.get("output_tokens", 0)
    blocks = reply.get("content", [])
    return blocks, reply.get("stop_reason") == "tool_use"

  def plain(self, system, prompt):
    reply = http_json(
      "https://api.anthropic.com/v1/messages",
      {
        "model": self.model,
        "max_tokens": MAX_TOKENS,
        "system": system,
        "messages": [{"role": "user", "content": prompt}],
      },
      {"x-api-key": self.key, "anthropic-version": "2023-06-01"},
    )
    usage = reply.get("usage", {})
    self.in_tok += usage.get("input_tokens", 0)
    self.out_tok += usage.get("output_tokens", 0)
    return " ".join(
      b.get("text", "")
      for b in reply.get("content", [])
      if b.get("type") == "text"
    ).strip()

  @staticmethod
  def tool_calls(blocks):
    return [
      (b["id"], b["name"], b.get("input", {}))
      for b in blocks
      if b.get("type") == "tool_use"
    ]

  @staticmethod
  def tool_results(results):
    return {
      "role": "user",
      "content": [
        {
          "type": "tool_result",
          "tool_use_id": tid,
          "content": text[:4000],
        }
        for tid, text in results
      ],
    }


class Ollama:
  tier = "floor"

  def __init__(self, model):
    self.model = model
    self.in_tok = 0
    self.out_tok = 0

  def available(self):
    try:
      req = urllib.request.Request("http://localhost:11434/api/tags")
      with urllib.request.urlopen(req, timeout=3) as resp:
        tags = json.loads(resp.read())
      names = [m.get("name", "") for m in tags.get("models", [])]
      return any(n.startswith(self.model) for n in names)
    except (urllib.error.URLError, OSError):
      return False

  def turn(self, system, messages, tools):
    reply = http_json(
      "http://localhost:11434/api/chat",
      {
        "model": self.model,
        "stream": False,
        "messages": [{"role": "system", "content": system}]
        + messages,
        "tools": [
          {
            "type": "function",
            "function": {
              "name": t["name"],
              "description": t["description"],
              "parameters": t["inputSchema"],
            },
          }
          for t in tools
        ],
      },
      {},
    )
    self.in_tok += reply.get("prompt_eval_count", 0)
    self.out_tok += reply.get("eval_count", 0)
    msg = reply.get("message", {})
    calls = msg.get("tool_calls") or []
    return msg, bool(calls)

  def plain(self, system, prompt):
    reply = http_json(
      "http://localhost:11434/api/chat",
      {
        "model": self.model,
        "stream": False,
        "messages": [
          {"role": "system", "content": system},
          {"role": "user", "content": prompt},
        ],
      },
      {},
    )
    self.in_tok += reply.get("prompt_eval_count", 0)
    self.out_tok += reply.get("eval_count", 0)
    return (reply.get("message", {}).get("content") or "").strip()

  @staticmethod
  def tool_calls(msg):
    out = []
    for i, c in enumerate(msg.get("tool_calls") or []):
      fn = c.get("function", {})
      args = fn.get("arguments", {})
      if isinstance(args, str):
        try:
          args = json.loads(args)
        except json.JSONDecodeError:
          args = {}
      out.append((str(i), fn.get("name", ""), args))
    return out

  @staticmethod
  def tool_results(results):
    return [
      {"role": "tool", "content": text[:4000]}
      for _, text in results
    ]


ALLOWED_TOOLS = ",".join(
  f"mcp__kumbarium__{t}"
  for t in ["remember", "recall", "confirm", "supersede",
            "forget", "link"]
)


class ClaudeCLI:
  """Realism tier via the Claude Code CLI: subscription-billed,
  and it exercises the REAL production client (user CLAUDE.md
  and all), with --strict-mcp-config pointing the kumbarium
  server at the sandbox so only the throwaway library is
  reachable. Limitation: clientInfo is always "claude-code", so
  per-persona provenance attribution is unavailable on this
  tier (cross-agent flow is still validated)."""

  tier = "realism"
  cli = True

  def __init__(self, model):
    self.model = model
    self.in_tok = 0
    self.out_tok = 0

  def available(self):
    return shutil.which("claude") is not None

  def run_episode(self, binary, home, system, turns):
    cfg = os.path.join(home, "mcp.json")
    with open(cfg, "w", encoding="utf-8") as fh:
      json.dump(
        {
          "mcpServers": {
            "kumbarium": {
              "command": binary,
              "args": ["serve"],
              "env": {"KUMBARIUM_HOME": home},
            }
          }
        },
        fh,
      )
    transcript = []
    session_id = None
    for turn in turns:
      cmd = [
        "claude", "-p", turn,
        "--model", self.model,
        "--output-format", "json",
        "--mcp-config", cfg,
        "--strict-mcp-config",
        "--allowedTools", ALLOWED_TOOLS,
        "--append-system-prompt", system,
      ]
      if session_id:
        cmd += ["--resume", session_id]
      run = subprocess.run(
        cmd, capture_output=True, text=True, timeout=300
      )
      try:
        reply = json.loads(run.stdout)
      except json.JSONDecodeError:
        transcript.append((turn, [], f"CLI ERROR: {run.stderr[:200]}"))
        continue
      session_id = reply.get("session_id", session_id)
      usage = reply.get("usage", {})
      self.in_tok += usage.get("input_tokens", 0)
      self.out_tok += usage.get("output_tokens", 0)
      transcript.append(
        (turn, ["(see witnessed events)"],
         str(reply.get("result", ""))[:400])
      )
    return transcript

  def plain(self, system, prompt):
    run = subprocess.run(
      [
        "claude", "-p", prompt,
        "--model", self.model,
        "--output-format", "json",
        "--append-system-prompt", system,
      ],
      capture_output=True,
      text=True,
      timeout=180,
    )
    try:
      reply = json.loads(run.stdout)
    except json.JSONDecodeError:
      return ""
    usage = reply.get("usage", {})
    self.in_tok += usage.get("input_tokens", 0)
    self.out_tok += usage.get("output_tokens", 0)
    return str(reply.get("result", ""))


def make_provider(spec, defaults):
  """Parse an agent spec: claude[:model] | api[:model] |
  ollama:<model>."""
  kind, _, model = spec.partition(":")
  if kind == "claude":
    return ClaudeCLI(model or defaults["claude"])
  if kind == "api":
    return Anthropic(model or defaults["api"])
  if kind == "ollama":
    return Ollama(model or defaults["ollama"])
  raise SystemExit(f"unknown agent spec {spec!r}")


def probe_agents(defaults):
  """Deterministic roster of runnable agents."""
  rows = []
  cli = ClaudeCLI(defaults["claude"])
  rows.append(
    (
      f"claude:{defaults['claude']}",
      "available (subscription CLI)"
      if cli.available()
      else "unavailable (claude CLI not on PATH)",
    )
  )
  api = Anthropic(defaults["api"])
  rows.append(
    (
      f"api:{defaults['api']}",
      "available (ANTHROPIC_API_KEY)"
      if api.available()
      else "unavailable (no ANTHROPIC_API_KEY)",
    )
  )
  try:
    req = urllib.request.Request("http://localhost:11434/api/tags")
    with urllib.request.urlopen(req, timeout=3) as resp:
      tags = json.loads(resp.read())
    names = sorted(
      m.get("name", "") for m in tags.get("models", [])
    )
    for n in names:
      rows.append((f"ollama:{n}", "available (local)"))
    if not names:
      rows.append(("ollama:<model>", "daemon up, no models"))
  except (urllib.error.URLError, OSError):
    rows.append(("ollama:<model>", "unavailable (no daemon)"))
  return rows


def rewrite_turns(user_provider, persona, turns):
  """v2-lite user-sim: canonical intents become natural user
  messages; technical tokens must survive verbatim so the
  grader's expectations stay valid."""
  system = (
    "You simulate a software developer talking to their coding "
    f"agent. Persona: {persona or 'a busy, direct developer'}. "
    "Rewrite the given intent as ONE natural chat message in "
    "that voice. Preserve every technical term, name, and "
    "number EXACTLY as written. Output only the message."
  )
  out = []
  for turn in turns:
    rewritten = user_provider.plain(system, turn).strip()
    out.append(rewritten if rewritten else turn)
  return out


def final_text(provider, blocks):
  if isinstance(provider, Anthropic):
    return " ".join(
      b.get("text", "")
      for b in blocks
      if b.get("type") == "text"
    ).strip()
  return (blocks.get("content") or "").strip()


def run_episode(provider, session, system, tools, turns):
  """Scripted user turns against one fresh agent context.
  Returns a transcript: per turn, the tools the agent used and
  an excerpt of its final reply."""
  messages = []
  transcript = []
  for user_turn in turns:
    messages.append({"role": "user", "content": user_turn})
    used, reply_text = [], ""
    for _ in range(MAX_TOOL_ROUNDS):
      try:
        blocks, wants = provider.turn(system, messages, tools)
      except (RuntimeError, OSError) as e:
        reply_text = f"PROVIDER ERROR: {e}"
        break
      if isinstance(provider, Anthropic):
        messages.append({"role": "assistant", "content": blocks})
      else:
        messages.append(blocks)
      reply_text = final_text(provider, blocks) or reply_text
      if not wants:
        break
      results = []
      for tid, name, args in provider.tool_calls(blocks):
        text = session.call(name, args)
        used.append(name)
        results.append((tid, text))
      extra = provider.tool_results(results)
      if isinstance(extra, list):
        messages.extend(extra)
      else:
        messages.append(extra)
    transcript.append((user_turn, used, reply_text))
  return transcript


def db(path):
  conn = sqlite3.connect(f"file:{path}?mode=ro", uri=True)
  conn.row_factory = sqlite3.Row
  return conn

def audit_rows(home):
  conn = db(os.path.join(home, "audit.db"))
  rows = [
    dict(r)
    for r in conn.execute(
      "SELECT agent_id, kind, scope, detail FROM events"
      " ORDER BY at, id"
    )
  ]
  conn.close()
  return rows


def entry_ns(home):
  conn = db(os.path.join(home, "library.db"))
  out = {
    r["id"]: (r["path"], r["agent_id"])
    for r in conn.execute(
      "SELECT e.id, ns.path, e.agent_id FROM entries e"
      " JOIN namespaces ns ON ns.id = e.namespace_id"
    )
  }
  conn.close()
  return out


def live_like(home, token):
  conn = db(os.path.join(home, "library.db"))
  rows = [
    dict(r)
    for r in conn.execute(
      "SELECT e.id, e.agent_id, e.superseded_by, e.note"
      " FROM entries e WHERE e.content LIKE ?",
      (f"%{token}%",),
    )
  ]
  conn.close()
  return rows


def chain_of(scope):
  parts = scope.split("/")
  chain = ["/".join(parts[: i + 1]) for i in range(len(parts))]
  return set(chain[::-1] + ["global"])


def grade_episode(ep, new_events, home, agent):
  checks = []
  writes = [
    e for e in new_events if e["kind"] in ("remember", "supersede")
  ]
  if ep.get("expects_recall"):
    ok = False
    for e in new_events:
      if e["kind"] in ("remember", "supersede"):
        break
      if e["kind"] == "recall" and e["scope"] == ep["scope"]:
        ok = True
        break
    checks.append(("recall-at-start", ok, True))
  for token in ep.get("expects_remember", []):
    found = any(
      r["superseded_by"] is None for r in live_like(home, token)
    )
    checks.append((f"remember[{token}]", found, True))
  if "correction" in ep:
    stale = ep["correction"]["stale_token"]
    rows = live_like(home, stale)
    superseded = any(r["superseded_by"] for r in rows)
    checks.append((f"supersede[{stale}]", superseded, True))
    if ep["correction"].get("expect_note"):
      noted = any(
        e["kind"] == "supersede"
        and json.loads(e["detail"]).get("note")
        for e in new_events
      )
      checks.append(("note-on-fit", noted, False))
  if "outcome" in ep:
    confirmed = any(e["kind"] == "confirm" for e in new_events)
    checks.append(("confirm-after-outcome", confirmed, False))
  if "expects_recall_token" in ep:
    tok = ep["expects_recall_token"]
    tok_ids = {r["id"] for r in live_like(home, tok)}
    got = False
    for e in new_events:
      if e["kind"] != "recall":
        continue
      returned = json.loads(e["detail"]).get("returned", [])
      hit = tok_ids & set(returned)
      for hid in hit:
        rows = live_like(home, tok)
        owner = next(
          (r["agent_id"] for r in rows if r["id"] == hid), agent
        )
        if owner != agent:
          got = True
    checks.append((f"cross-agent[{tok}]", got, True))
  _ = writes
  return checks


def fleet_checks(home):
  checks = []
  ns_of = entry_ns(home)
  breach = 0
  for e in audit_rows(home):
    if e["kind"] != "recall" or not e["scope"]:
      continue
    allowed = chain_of(e["scope"])
    for rid in json.loads(e["detail"]).get("returned", []):
      ns = ns_of.get(rid, (None, None))[0]
      if ns is not None and ns not in allowed:
        breach += 1
  checks.append(("firewall-breaches==0", breach == 0, True))
  mismatch = 0
  for e in audit_rows(home):
    if e["kind"] != "remember":
      continue
    eid = json.loads(e["detail"]).get("id")
    owner = ns_of.get(eid, (None, None))[1]
    if owner is not None and owner != e["agent_id"]:
      mismatch += 1
  checks.append(("provenance-sane", mismatch == 0, True))
  return checks


def run_fixture(path, provider, binary, snippet, user_provider):
  with open(path, "rb") as fh:
    fx = tomllib.load(fh)
  home = tempfile.mkdtemp(prefix="kumbarium-persona-")
  for ns in fx["fixture"]["namespaces"]:
    subprocess.run(
      [binary, "namespace", "add", ns, "persona sim"],
      env=dict(os.environ, KUMBARIUM_HOME=home),
      capture_output=True,
    )
  roles = {a["name"]: a.get("role", "") for a in fx["agents"]}
  results, transcripts = [], []
  seen = 0
  user_persona = fx.get("user", {}).get("persona", "")
  for ep in fx["episodes"]:
    agent = ep["agent"]
    turns = ep["turns"]
    if user_provider is not None:
      turns = rewrite_turns(user_provider, user_persona, turns)
    system = (
      f"{snippet}\n\n{roles.get(agent, '')}\n"
      f"The current project scope is {ep['scope']}."
    )
    if getattr(provider, "cli", False):
      transcript = provider.run_episode(
        binary, home, system, turns
      )
    else:
      session = Session(binary, home, agent)
      tools = session._request("tools/list", {})[0]["result"][
        "tools"
      ]
      transcript = run_episode(
        provider, session, system, tools, turns
      )
      session.close()
    events = audit_rows(home)
    new_events = events[seen:]
    seen = len(events)
    ep_name = ep.get("name", agent)
    results.append(
      (ep_name, grade_episode(ep, new_events, home, agent))
    )
    transcripts.append((ep_name, agent, transcript, new_events))
  if fx["fixture"].get("fleet"):
    results.append(("fleet", fleet_checks(home)))
  env = dict(os.environ, KUMBARIUM_HOME=home)
  status = subprocess.run(
    [binary, "status"], env=env, capture_output=True, text=True
  ).stdout
  minutes = subprocess.run(
    [binary, "audit", "export", "--stdout", "--raw"],
    env=env,
    capture_output=True,
    text=True,
  ).stdout
  shutil.rmtree(home, ignore_errors=True)
  return fx["fixture"]["name"], results, transcripts, status, minutes


def main():
  ap = argparse.ArgumentParser(
    description=(
      "Persona harness: real LLM agents run multi-session "
      "arcs against a sandboxed library; the witness grades "
      "the exam (docs/design/persona-harness.md). Tiers skip "
      "cleanly when unavailable."
    )
  )
  ap.add_argument(
    "--binary",
    default="target/release/kumbarium",
    help="kumbarium binary to drive (default: release build)",
  )
  ap.add_argument(
    "--fixture",
    action="append",
    default=None,
    help=(
      "fixture toml; repeatable (default: all of "
      "scripts/personas/*.toml)"
    ),
  )
  ap.add_argument(
    "--claude-model",
    default="haiku",
    help="Claude Code CLI model alias (subscription tier)",
  )
  ap.add_argument(
    "--anthropic-model",
    default="claude-haiku-4-5-20251001",
    help="API model id (needs ANTHROPIC_API_KEY)",
  )
  ap.add_argument(
    "--ollama-model",
    default="llama3.1",
    help="local Ollama model (robustness floor)",
  )
  ap.add_argument(
    "--out",
    default=None,
    help=(
      "report path (default: docs/reports/<date>-persona"
      "-sim.md)"
    ),
  )
  ap.add_argument(
    "--ai-agent",
    action="append",
    default=None,
    help=(
      "agent spec to run: claude[:model] | api[:model] | "
      "ollama:<model>; repeatable (default: every available "
      "tier). Bare invocation lists the roster."
    ),
  )
  ap.add_argument(
    "--user-agent",
    default="scripted",
    help=(
      "who plays the USER: 'scripted' (default, deterministic) "
      "or an agent spec that rewrites the fixture's intents "
      "into natural messages (tokens preserved verbatim)"
    ),
  )
  opts = ap.parse_args()
  defaults = {
    "claude": opts.claude_model,
    "api": opts.anthropic_model,
    "ollama": opts.ollama_model,
  }
  if len(sys.argv) == 1:
    print("available agents (deterministic probe order):")
    for spec, status in probe_agents(defaults):
      print(f"  {spec:<36} {status}")
    print("\nrun with --ai-agent <spec>; see --help")
    return 0
  binary = os.path.abspath(opts.binary)
  fixtures = opts.fixture or sorted(
    glob.glob("scripts/personas/*.toml")
  )
  snippet = subprocess.run(
    [binary, "instructions", "--snippet"],
    capture_output=True,
    text=True,
  ).stdout

  if opts.ai_agent:
    providers = [
      make_provider(spec, defaults) for spec in opts.ai_agent
    ]
  else:
    providers = [
      ClaudeCLI(opts.claude_model),
      Anthropic(opts.anthropic_model),
      Ollama(opts.ollama_model),
    ]
  user_provider = None
  if opts.user_agent != "scripted":
    user_provider = make_provider(opts.user_agent, defaults)
    if not user_provider.available():
      raise SystemExit(
        f"user agent {opts.user_agent!r} is unavailable"
      )
  lines = [
    "<!-- @generated by scripts/persona_sim.py -->",
    "# Persona simulation report",
    "",
    f"Run: {time.strftime('%Y-%m-%d %H:%M:%S')}",
    f"User side: {opts.user_agent}",
    "",
  ]
  required_failed = 0
  for provider in providers:
    label = f"{type(provider).__name__} {provider.model}"
    if not provider.available():
      lines += [f"## {label}: SKIPPED (unavailable)", ""]
      continue
    lines += [f"## {label} ({provider.tier} tier)", ""]
    for path in fixtures:
      try:
        name, results, transcripts, status, minutes = run_fixture(
          path, provider, binary, snippet, user_provider
        )
      except Exception as e:  # tier survives its own wreckage
        lines += [
          f"### {os.path.basename(path)}: RUN FAILED",
          "",
          f"- {type(e).__name__}: {str(e)[:300]}",
          "",
        ]
        if provider.tier == "realism":
          required_failed += 1
        continue
      lines.append(f"### {name}: scorecard")
      lines.append("")
      for ep_name, checks in results:
        for check, ok, required in checks:
          verdict = "PASS" if ok else "MISS"
          tag = "" if required else " (informational)"
          lines.append(f"- {ep_name}: {check}: {verdict}{tag}")
          if required and not ok and provider.tier == "realism":
            required_failed += 1
      lines.append("")
      lines.append(f"### {name}: transcripts")
      lines.append("")
      for ep_name, agent, transcript, events in transcripts:
        lines.append(f"#### {ep_name} (agent {agent})")
        lines.append("")
        for turn, used, reply in transcript:
          lines.append(f"- user: {turn[:160]}")
          lines.append(
            f"  - tools: {', '.join(used) if used else 'none'}"
          )
          lines.append(f"  - agent: {reply[:240]}")
        lines.append(
          f"  - witnessed events: "
          + ", ".join(e["kind"] for e in events)
        )
        lines.append("")
      lines.append(f"### {name}: library state after run")
      lines += ["", "```", status.rstrip(), "```", ""]
      lines.append(f"### {name}: full minutes (the demo artifact)")
      lines += ["", minutes.rstrip(), ""]
    lines.append(
      f"Token usage this tier: {provider.in_tok} in / "
      f"{provider.out_tok} out"
    )
    if isinstance(provider, Anthropic):
      cost = (
        provider.in_tok * 1.0 + provider.out_tok * 5.0
      ) / 1_000_000
      lines.append(f"Estimated cost: ~${cost:.4f}")
    if isinstance(provider, ClaudeCLI):
      lines.append("Billing: subscription (Claude Code CLI)")
    lines.append("")

  text = "\n".join(lines) + "\n"
  out = opts.out or time.strftime(
    "docs/reports/%Y-%m-%d-%H%M%S-persona-sim.md"
  )
  os.makedirs(os.path.dirname(out), exist_ok=True)
  with open(out, "w", encoding="utf-8") as fh:
    fh.write(text)
  print(text)
  print(f"report: {out}", file=sys.stderr)
  return 1 if required_failed else 0


if __name__ == "__main__":
  sys.exit(main())
