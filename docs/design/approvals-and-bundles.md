# Approvals and bundles: the circulation desk

Designed together on purpose: bundles are how memories travel
between libraries, approvals are how untrusted memories earn
shelf space, and the one genuinely hard merge case (forked
supersession) resolves THROUGH the approvals queue. This doc is
design only; each piece gets its build pass on sign-off.

## Doctrine

The OSS accountability model, restated: untrusted contributors
write to quarantine BY DEFAULT; a human promotes via approval.
Blame does not shift, it LANDS where judgment happened,
provably: provenance shows who submitted, the approval event
shows who promoted having seen what. This converts memory
poisoning from a security problem into a governance problem,
exactly as pull requests did for untrusted code. Kumbarium
sells the provable chain of custody; users own every judgment
made on it.

Constraints inherited from standing decisions:

- D-020: content is immutable. Approval never edits; it only
  changes circulation status. "You approved this at T" is
  undeniable because what was approved cannot have changed.
- Approve/reject are HUMAN-ONLY (CLI, later the dashboard
  inbox), like retire and revert. Agents submit; people judge.
- Keep-never-delete: a rejected memory is retained evidence of
  a judgment, not garbage. `forget` remains the separate,
  human-only tool for wrong-or-sensitive content.
- The review view shows real content, provenance, and the
  collision surface, never the writer's self-description
  (the diff-decides principle applied to promotion).

## Entry status (the schema change, when built)

Entries gain a `status` column: `live` (default, today's
behavior), `pending` (in quarantine, awaiting judgment), and
`rejected` (judged and declined, kept for the record).

- `pending` and `rejected` entries never surface in recall,
  list defaults, grep defaults, or namespace chain search.
  The firewall claim extends: no recall ever returns a
  non-live entry, and the persona harness gets a fixture
  expectation to prove it behaviorally.
- Status is orthogonal to retirement (`retired_at` hides a
  LIVE entry from suggestion surfaces) and to supersession
  (chain position). A pending entry can be superseded by its
  submitter before judgment (revising the PR); the chain head
  stays pending.
- The entry keeps its TARGET namespace from day one. No
  quarantine namespaces: location is where a fact belongs,
  status is whether it is in circulation. (The early
  `agent/<id>/quarantine` sketch is superseded by this.)

## Who lands in quarantine

Write policy per agent identity, in config:

```
[approvals]
# Default write mode for agents not listed: live | pending
default_mode = live
# Per-agent overrides, e.g. quarantine one untrusted writer.
# pending_agents = intern-bot, contrib-scraper
```

Personal tier keeps today's behavior (default live, zero
ceremony). Teams and OSS flip the default to pending and
whitelist trusted writers. Honesty about the trust boundary,
in ink: at the stdio tier identity is self-declared
(clientInfo), so quarantine-by-default is a correctness
mechanism, not yet an enforcement one; authn hardens the
boundary at the daemon rung, and the mechanism is built
correct-first so enforcement has something real to guard.

## The surfaces

```
kum inbox                     pending entries, oldest first
kum review <id>               one pending entry, full view
kum approve <id>              promote to live (witnessed)
kum reject <id> [reason]      decline, keep for the record
```

`kum review` is the load-bearing one: content (full, split
sets stitched), provenance (agent, source, submitted-at), and
the COLLISION SURFACE: a recall-style FTS pass over the target
scope listing live entries the candidate may duplicate or
contradict. The reviewer sees what the shelf already holds
before promoting; approving with eyes open is the product.

Audit gains two kinds: `approve` and `reject`, detail carrying
the entry id, submitter, and reason. The GUI chapter starts
here when it starts: the approvals inbox is the dashboard's
home page, and every verb above is already witnessed.

## Bundles: memories in motion

```
kum bundle <scope> [--out FILE]     export a shelf
kum import bundle <FILE> [--pending]
```

One deterministic JSON file: a versioned header (format
version, exported-at, source library fingerprint, content
hash), then entries (full provenance, tags, notes, timestamps,
supersession pointers) and edges, in stable id order. Content
is byte-identical to the library's (D-020 travels). The hash
lets a PR conversation name a bundle unambiguously and lets
the importer verify the file matches what was reviewed
(SHA-256; implementation vendored into util when built, shared
later by the audit hash chain).

Solves, in one primitive: multi-machine personal sync without
a daemon; serverless memory sharing; and the OSS "memory PR",
a bundle attached to a pull request, reviewed and imported by
a maintainer.

### Merge semantics

Union-merge, made nearly free by immutable entries and UUIDv7
ids:

- An id already present is skipped (idempotent re-import;
  content divergence on the SAME id is a hard error, since it
  can only mean tampering or corruption).
- New entries import with their original provenance intact.
  The importer's judgment is recorded separately: the import
  event names who imported, from which bundle, with what hash.
- Import respects the approvals policy: `--pending` (or a
  pending default_mode) lands every new entry in quarantine.
  Personal sync between one person's own machines imports
  live; a stranger's bundle goes through the desk.
- Supersession pointers to entries the local library lacks
  import fine (the bundle carries the chain); pointers into
  entries both sides know reconcile as chain extensions.

### Forked supersession (the hard case)

Both libraries superseded the same entry, differently. Two
live heads for one fact is the contradiction disease (the same
reason branching was rejected), so the merge never chooses:

- The incoming rival head imports as PENDING, always, even
  when the rest of the bundle imports live.
- A `contradicts` edge links the two heads; the local head
  stays live and circulating.
- The queue resolves it: approve the rival then retire or
  supersede the local head (the review view shows both sides
  and their diff), or reject the rival. Either way a human
  judged, and the ledger shows who and when.

## Non-goals (v1)

- No partial-bundle cherry-picking (import whole files; reject
  individually from the queue).
- No bundle encryption or signing (transport security is the
  carrier's job at this tier; signing arrives with the daemon
  trust boundary).
- No auto-approval rules, ever, except the one exact case
  already decided: nothing. Even exact-duplicate imports skip
  by id, not by judgment.
- No delegation of approvals (enterprise chapter).

## Testing shape (when built)

Store: status transitions, recall/list/grep exclusion of
non-live, forked-supersession import creating pending + edge.
CLI: inbox/review/approve/reject round-trip with witnessed
events. Bundle: export-import round-trip is id-identical and
idempotent; hash verification catches a flipped byte. Persona
harness: a fleet fixture where an untrusted persona's writes
land pending, never surface in a sibling's recall, and appear
in the inbox; the firewall check extends to status.
