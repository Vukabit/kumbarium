# The reading room: coordination leases (D-043)

The coordination section's third resource, after the docket
(what is owed) and the handoffs (what is mid-flight): leases
say what agents are DOING, right now, so parallel sessions
stop colliding by accident. A lease is a reading-room
reservation: a signed card on the table saying who is working
where, visible to everyone who walks in.

## The four stances, settled

- A lease names a NAMESPACE plus a free RESOURCE string
  (`project/kumbarium` + `crates/kumbarium-store`). Fine
  enough for parallel work inside one repo; spelling drift
  between agents is the accepted cost, tolerable exactly
  because collisions warn rather than block.
- Collisions WARN, NEVER BLOCK. Taking a lease someone else
  holds succeeds, loudly: the taker is told who holds it and
  since when, and both leases stand. Honest at this trust
  tier (the grants-are-POLICY argument again): identity is
  self-reported, so blocking would be theater, and a crashed
  agent must never padlock the library. Real mutual exclusion
  can arrive at the daemon rung with authn, on this same
  table, without a migration.
- TTL WITH ACTIVITY RENEWAL. A lease lives `ttl_minutes`
  (config, default 120) past its last renewal, and ANY
  witnessed event by the holder renews every lease it holds:
  the ledger is already the heartbeat, so there is no renewal
  protocol to forget. An expired lease is simply not served;
  the janitor reports expired-but-unreleased leases as a
  finding (the crashed-agent shape), and release stays a
  courtesy, not a duty.
- TOOLS PLUS SERVING. `lease_take` and `lease_release` are
  the agent's intent surface; the ROOM ITSELF rides the first
  recall in a scope (D-037): active leases in the chain are
  prepended alongside the briefing and urgent matters, so an
  agent learns the room is occupied without a read tool it
  could forget to call. Humans read `kum leases [ns]` and can
  clear a stuck card with `kum lease break <id>`, witnessed.

## Shelving

`library/leases.db` per D-033: lazy file, per-shelf backups,
namespace stored as the validated path. One table:

- leases: id (UUIDv7), namespace, resource, agent_id,
  session_id, note, taken_at, renewed_at, released_at (NULL =
  standing), created_at. SESSIONS ARE MINTED, AGENTS ARE
  CLAIMED (D-044): the librarian mints a session id per serve
  process at initialize, and a holder is (agent, session), so
  two sessions of the SAME agent name are different holders
  and warn each other, which is the room's primary case
  (self-reported names alone made same-name sessions invisible
  to each other, and shared-name activity would have kept
  zombie cards alive forever). Activity renews per session,
  never per name. A minted id disambiguates, it does not
  authenticate: spoofing stays the daemon rung's problem.
  ACTIVE means released_at IS NULL and now is within ttl of
  renewed_at; expiry is computed at read time
  from config, never stored, so a config change re-prices the
  room instantly and there is no reaper to schedule.

Rows are kept when released or broken (the card goes in the
drawer, not the bin); the ledger carries the story anyway:
lease_take, lease_release, lease_break witness kinds (audit
migration 0006). Expiry is an absence, not an event: nothing
fires when a lease lapses, it just stops being served, which
is why the janitor's finding exists.

## What a lease is NOT

Not a lock (warns only), not knowledge (nothing here enters
recall ranking or the collection), not enforcement (that is
the daemon rung's to add), and never served as content: the
serving is a line in the room's register, names and times
only.

## Surfaces

MCP: `lease_take` (namespace, resource, optional note;
returns the card, plus who else holds overlapping cards),
`lease_release` (namespace, resource; releases the caller's
own card only). First-recall serving per D-037.

CLI: `kum leases [ns]` (the register: holder, resource, age,
freshness), `kum lease break <id>` (human clears a stuck
card, witnessed as lease_break with the holder named).

Janitor: expired-but-unreleased leases surface as a watchdog
finding with the holder and how stale; advisory, zero writes,
like every finding.

## Testing shape

Crate: take/overlap/release round-trip; renewal moves
renewed_at; active_in respects ttl and released_at; break
releases someone else's card. rpc: take warns on overlap;
first recall serves the room; release only releases your own.
Janitor: stale lease surfaces, active one does not.
