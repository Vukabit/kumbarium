# Handoffs: what is mid-flight (D-036)

The coordination section's second resource. A handoff is the
note a departing session leaves for the next one: what is
mid-flight in this scope, what was decided but not finished,
where the sharp edges are. The problem statement is every
compacted conversation and every "continue where the last
session left off" that today leans on ad-hoc memory files.

Built as designed.

## The semantic: one standing note per shelf

Exactly ONE live handoff per namespace. Writing a handoff IS
superseding the previous one: the chain is the scope's
session-by-session narrative history, free from the same
discipline everything else uses (D-020). There is no delete;
a scope with nothing mid-flight gets a handoff that says so
("nothing mid-flight; vN shipped clean"), which is information,
not absence.

## Served first, literally

No read tool. The FIRST recall a session makes in a scope
returns the standing handoff PREPENDED to its results, marked
as such ("standing handoff for project/x, left by claude-code
2h ago: ..."). Later recalls in the same session stay clean.
This makes "served first to the next session" a mechanism
instead of an instruction: the agent cannot skip the briefing,
because the briefing rides the tool it was already told to call
first. (Session boundary = the server process; each MCP client
spawn is a session, which is exactly the granularity wanted.)

The snippet gains the other half of the ritual: before ending
substantive work, `handoff_write` what is mid-flight. Writing
is judgment the agent must exercise; reading is free.

## Shelving

`library/handoff.db`, its own file per D-033 (a shelf's file
does not exist until first use; per-shelf backups join the
rotation the way the docket's did). Namespace stored as the
validated PATH, gate-checked against the registry. Rows:

- id (UUIDv7), namespace, content, agent_id + source,
  superseded_by, note, status (D-027 verbatim), created_at,
  updated_at.
- Content is NARRATIVE: multi-line welcome, capped generously
  (4000 bytes) with the docket's refusal wording adapted; no
  auto-split (the inheritance contract holds: a briefing that
  cannot fit is hiding a design document that belongs in
  memory).
- No severity, no goal, no state machine: a handoff is not a
  matter, it is a note. The chain position IS its lifecycle
  (live head = the briefing; ancestors = the story so far).

## The injection note (sharpest of the three shelves)

A memory poisons belief, a task poisons action, a handoff
poisons a session's OPENING FRAME, delivered automatically at
the moment of maximum trust. So the desk applies with the most
teeth yet: a quarantined writer's handoff lands pending and is
NEVER served (recall prepends live heads only); the review
surface leads with provenance; and the served header always
names who left the note and when, so even a trusted-but-wrong
briefing carries its accountability on its face.

## Surfaces

MCP, ONE new tool (nine total):

- `handoff_write`: namespace, content. Supersedes the standing
  head implicitly; the response confirms what the next session
  will see. Policy decides live vs pending (D-027).

CLI:

```
kum handoff <ns> <note...>     leave the briefing (supersedes)
kum handoff <ns>               read the standing briefing
kum handoffs                   every shelf's standing briefing
```

`show`, `history`, and the desk verbs fall through to the
handoff shelf like every shelf (D-034: ids are building-wide);
`kum history <id>` on a handoff renders the session narrative,
which is the sleeper feature: `kum history` of a scope's
handoff chain reads as the project's diary.

The witness gains `handoff_write` (audit migration 0003), and
the recall event's detail records `handoff_served: true` when a
briefing rode along, so the ledger shows not just that the note
existed but that the next session actually received it: the
checkout of the briefing is provable, per the charter.

## Janitor (v2, named now)

A stale briefing misleads with authority: the janitor's
findings grow "stale handoffs" (standing head older than N
days in a scope with recall activity since; config
`handoff_stale_days`, default 14). Advisory, human-judged.

## Non-goals (v1)

- No per-agent handoffs (the note is the SCOPE's state, not a
  private letter; the daemon rung can revisit).
- No multiple live briefings per scope, no TTL/expiry (the
  janitor surfaces staleness; a human or the next session
  supersedes).
- Handoffs do not travel in bundles (a briefing is local
  context by nature; revisit if multi-machine sync demands).
- No structured fields (blockers/next-steps schemas): prose
  with judgment beats forms filled from habit. Structure can
  earn its way in from observed usage.

## Testing shape (when built)

Store: one-live-head invariant, supersession chains, pending
never served. rpc: first-recall-prepends / second-recall-clean;
handoff_served on the ledger. CLI round-trip + fall-throughs.
Persona harness: an arc where session one is told mid-flight
state and must handoff_write it (graded: token in the standing
head), session two must act consistently with the briefing
without re-asking (graded: no contradicting question, the
recall event carries handoff_served); the injection fixture:
a quarantined persona's briefing never reaches session two.
