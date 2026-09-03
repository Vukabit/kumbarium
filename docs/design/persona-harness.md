# Persona harness design

Companion to `scripts/daily_drive_sim.py`. The sim proves the
SYSTEMS half (correctness, latency, concurrency, growth); this
harness measures the BEHAVIORAL half: do real LLM agents, given
only the standard onboarding (the `kum instructions` snippet and
the live tool schemas), actually use the library well across
multi-session arcs? Together they are the pre-launch validation
suite.

## Principles

1. The witness grades the exam. Scoring is deterministic reads
   of the throwaway library and audit log after each arc: no
   judge model, no rubric prompts. "Did a recall happen at
   session start" is a query, not an opinion.
2. Test the real surface. Tool definitions are fetched from the
   live server's `tools/list` at run time, and the system prompt
   is `help::SNIPPET` verbatim. The harness can never drift from
   what agents actually receive; if a model misuses a tool, that
   is documentation feedback.
3. Two model tiers, both on purpose. A low-cost Anthropic model
   (Haiku class) is the realism tier; a local Ollama model is
   the robustness floor: if a small local model uses the library
   correctly from the schemas alone, the surface is genuinely
   self-explanatory, which is what agent-agnostic means.
4. Scripted users by default; LLM user-sim on request
   (--user-agent). The rewrite keeps graded tokens verbatim
   (falls back to the canonical turn otherwise) but is NOT
   forced into imperative voice: real users are ambiguous, and
   an agent that acknowledges a fact without storing it has
   missed for real (D-024). The user-sim itself must never
   answer, acknowledge, or act on an intent, and runs with no
   MCP servers and an empty cwd.

## Components

### Fixtures: `scripts/personas/*.toml`

One file per scenario. Contents:

- project: name, namespace, background facts the driver
  pre-seeds (or leaves empty for a cold start).
- agent persona: provider/model (`anthropic:<model>` or
  `ollama:<model>`); the system prompt is always the snippet
  plus a one-line role.
- arc: ordered EPISODES. Each episode is a fresh session
  (fresh context: this is the point) with scripted user turns
  and declared EXPECTATIONS the grader checks:
  - `expects_recall`: a recall event in scope must precede the
    first write of the episode.
  - `expects_remember: ["token", ...]`: after the episode, some
    live entry must contain each token.
  - `correction: { stale_token, expect_note }`: a user turn
    corrects a fact ("throws a fit"); the grader requires a
    supersede event on the entry containing the token, and
    optionally a note.
  - `outcome_signal: { token }`: a user turn reports success
    ("CI passed with that config"); grader checks whether a
    confirm event followed (reported, not required: confirm is
    voluntary evidence by design).

### Driver: `scripts/persona_sim.py`

- Sandbox per run: `KUMBARIUM_HOME` temp dir; namespaces
  pre-registered by the driver (registered-only rule holds).
- Reuses the sim's `Session` (stdio bridge to `kumbarium
  serve`), one server process per episode, `clientInfo.name` =
  the persona name (provenance works for free).
- Tool loop per provider:
  - Anthropic: messages API with the six tool definitions
    translated from `tools/list`; standard use->result loop
    until end_turn; per-episode max-token cap.
  - Ollama: `/api/chat` with tools; same loop shape; skipped
    cleanly if no local daemon.
  - A provider without credentials is skipped with a note, so
    the harness degrades to whichever tiers are available.
- Cost posture: episodes are short (a few thousand tokens);
  a full arc battery on the Haiku tier costs cents. Caps are
  hard, not advisory.

### Grader and report

After each arc, open the throwaway dbs read-only and score the
expectations; render one scorecard per (fixture x model) into
`docs/reports/persona-sim-<date>.md` (marked `@generated`):

- recall-at-start rate across episodes
- remember coverage (expected tokens found live)
- correction handling (supersede fired; note present)
- confirm-after-outcome rate (informational)
- hygiene: unregistered-namespace errors, protocol errors,
  oversize handling (splits observed vs oversize failures)

Exit code is nonzero when any REQUIRED expectation fails on the
realism tier; the robustness floor is always informational.

## The seed scenario

`simcli.toml`, a four-episode arc on `project/simcli`:

1. Kickoff: user states three durable facts (a convention, a
   dependency decision, a constraint) inside a small task.
   Expect: remembers covering all three tokens.
2. Fresh session, adjacent task that silently depends on fact
   two. Expect: recall-at-start; the reply must not contradict
   the stored decision.
3. The fit: user corrects fact one ("no - we switched to X
   weeks ago"). Expect: supersede on the entry with the stale
   token, note encouraged.
4. Outcome: user reports fact three's constraint held in CI.
   Confirm observed -> reported.

## Non-goals (v1)

- LLM user-sim (v2), adversarial/poisoning personas (v2, lands
  with the approvals section), multi-agent behavioral interplay
  in one arc (v2; the sim already storms systems-level
  concurrency), and any judgment of prose QUALITY of stored
  memories (grader checks presence and lifecycle, not style).

## Fleet scenarios (v1.5): many agents, many repos

The single-persona arc measures one agent's habits; the fleet
fixture measures the FOUNDING CLAIM: knowledge crossing agents.
One library, N agent personas, M mock projects, interleaved
episodes. New expectation kinds:

- cross-agent recall: agent A's episode stores a fact that is
  cross-project by nature; a LATER episode of agent B (different
  model, different project context) depends on it. Grader:
  B's recall returned A's entry. This also behaviorally tests
  NAMESPACE JUDGMENT: the flow only works if A chose `global`
  over burying the fact in its project shelf.
- firewall: across the whole run, no recall event scoped to
  project X ever returned an entry living in sibling project Y.
  Pure ledger query; zero tolerance.
- provenance sanity: every entry's agent_id matches the persona
  that ran the episode that created it.

Double duty, by design: the fleet run's own audit minutes are
the DEMO ARTIFACT: "three agents, three repos, one brain, every
checkout on the record" is the launch narrative rendered by the
witness itself, and lands in docs/reports/ like any other run.
