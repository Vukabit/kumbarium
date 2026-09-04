# Janitor design (the confidence pass and the watchdog)

The janitor is the designated mover of the confidence number
(D-004: writers never self-assess; confirm is evidence, never
judgment). v1 is DETERMINISTIC: a ledger-math pass with no LLM
anywhere, because the survival-first doctrine made the backbone
fully derivable from data the witness already records. The LLM
janitor duties (dedup, merge, contradiction) stay future work
and will land behind the same propose/dispose gate.

## Doctrine (settled 2026-09-03, restated)

There is no mechanical trigger for "fact proved correct":
success is silent, so explicit confirms chronically under-fire.
The asymmetry that rescues confidence: failure forces action
(a wrong fact forces a supersede or forget, both captured
writes) while success forces nothing. Therefore:

- SURVIVAL is the backbone: recalled repeatedly, across agents
  and days, and never corrected. Exposure without fits.
- Explicit confirms are sparse high-grade garnish: weighted
  above habitual signals, discounted when self-reported (the
  writing agent confirming its own entry).
- No exposure means no evidence: a never-recalled entry KEEPS
  the neutral prior. Dormancy is a finding for human review,
  never a confidence penalty.
- Access is provable, application never is (the epistemic
  charter): confidence measures how the fact has survived
  circulation, not whether it is true.

## The pass

Stateless and idempotent: every run recomputes every live
entry's confidence from the full ledger. Running it twice
changes nothing the second time; there is no incremental state
to drift.

Per live entry, from the audit ledger:

- exposures k: distinct (agent, day) pairs among recall events
  whose `returned` ids include this entry. Ten recalls by one
  agent in one sitting count once; the same fact consulted by
  three agents across a week counts heavily.
- confirms: confirm events for this id. A confirm by an agent
  OTHER than the entry's writer counts 1.0; a self-confirm
  counts 0.25 (provenance discount). Sum = c.

Confidence:

```
confidence = 0.50                       neutral prior
           + 0.30 * k / (k + 4)         survival, asymptote .80
           + 0.15 * c / (c + 1)         confirms, asymptote .95
```

Rounded to two decimals. The ceiling is 0.95 by construction:
nothing inside the library can prove application, so nothing
reaches 1.0. Evidence counts only events addressed to THIS
entry id: a supersession starts a new claim with fresh
evidence (revision resets survival; the chain records where it
came from).

v1 has no negative adjustments. The fit's negative evidence
lands on the superseded (dead) version by definition; spreading
it to semantic neighbors needs similarity machinery the v0.1
gate excluded. Named future work, not scope creep.

## Confidence never ranks (D-026)

The search-engine feedback loop is the trap named in advance:
the moment a quality score feeds retrieval ORDER, highly rated
entries get recalled more, survive more, and rate higher, a
self-reinforcing loop that manufactures its own evidence.
Recall ranks by relevance (bm25) alone; confidence is served
WITH results, never as a filter or sort key. The librarian
hands over the book and states its condition; it does not hide
the shabby ones. This also keeps survival statistics honest:
exposure stays driven by what agents ask, not by what the
janitor previously concluded.

## Search-engine lessons (the v2 signals, shipped)

Kumbarium's dual scoring already mirrors the core web-search
split (query-dependent relevance vs query-independent
authority), and survival is the "long click" (used without
bouncing back). Three more signals mapped cleanly and shipped
in v2, all deterministic and provenance-weighted:

- LINK AUTHORITY joined the formula (D-040): inlinks counted
  from the ledger's link events as votes for the LINKED-TO
  entry, weighted by who cast them (cross-agent 1.0, a
  self-link 0.1: the PageRank caveat solved by provenance).
  Term: + 0.05 * l / (l + 2). To hold the 0.95 ceiling, the
  confirm weight demoted 0.15 -> 0.10; the survival backbone
  (0.30) never moves.
- POGO-STICKING is a finding, not a penalty: a supersede
  landing within 48h of a recall that returned the old version
  means the library ACTIVELY served a wrong fact, stronger
  signal than a cold correction. The negative evidence already
  lands on the dead version by definition; the finding tells
  the human where circulation misfired.
- KIND-DESERVES-FRESHNESS: the dormancy window is per kind, as
  multiples of the one config knob: project_state 1/2x,
  decision 1x, reference 2x, preference 4x of dormant_days.

The disanalogy is the advantage: search ranks an adversarial
open web with opaque models because it must; the janitor ranks
a provenance-tracked private collection, so every signal stays
deterministic, explainable, and auditable.

## Findings (advisory, zero writes)

The pass proposes confidence changes and REPORTS everything
else; the janitor never retires, never revokes, never closes.

- dormant: live, older than its kind's window (see
  kind-deserves-freshness), never returned by any recall.
  Retire candidates for the human.
- served-then-corrected (pogo): entry id, scope, and the gap in
  hours between the serving recall and the correcting
  supersede. CROSS-AGENT only: the same agent recalling then
  superseding is the instructed correction ritual (recall the
  stale entry, supersede the id it returned), so it never
  counts; agent A served, agent B corrected is the library
  circulating a wrong fact.
- creeping matters: an open docket matter whose goal moved
  LATER two or more times across its chain (the CLI walks the
  chain, the janitor counts the slips). A pulled-in goal is not
  a slip.
- UNWITNESSED GRANTS, the tamper shape and the sharpest
  finding: a row in the secrets grants table with no matching
  secret_grant event on the ledger arrived around the
  librarian (direct sqlite is the obvious path). Rendered
  first, in red, with the remediation (witnessed revoke, then
  rotate). This is the watchdog the witness always implied:
  the tables claim what the ledger never saw.
- expired credentials still stocked: a live secret whose
  value-expiry metadata has passed; rotation owed.

The shelf inputs (goal chains, grant rows, expiry metadata;
never values) are extracted by the CLI so the pass stays pure
computation; a missing shelf is an empty input, never a
guess.

## Surface and sign-off

Same shape as revert: preview by default, `--apply` to commit,
CLI only.

- `kum janitor`: table of proposed changes (short id,
  namespace, old -> new, basis) plus the findings section.
  Proposes only deltas >= 0.01.
- `kum janitor --apply`: writes confidence + a stored
  `confidence_basis` (store migration 0005; the read path
  serves the stored basis so recall explains the number without
  touching the audit db), then witnesses ONE `janitor` audit
  event (audit migration 0005 widens the kind CHECK) whose
  detail carries every applied change `{id, from, to}` and the
  run's parameters. Batch event by design: one run, one ledger
  line; the detail holds the full manifest for the future
  hash-chained compliance packet.

## Placement

`crates/kumbarium-janitor`, the slot reserved in the crate map:
depends on kumbarium-store and kumbarium-audit (read), pure
computation inside; the binary crate wires CLI, config, and the
apply transaction. Config gains a `[janitor]` section:
`dormant_days` (default 45).

## Testing

Unit: formula properties (prior with no evidence, asymptotes,
self-confirm discount, idempotence). Integration: seed a
throwaway library, drive recalls/confirms through the real
tools, run the pass, assert exact numbers. The persona harness
gains a post-arc janitor invocation in a later change so
behavioral runs also exercise the pass on organically written
libraries.
