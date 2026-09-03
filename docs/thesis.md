# Thesis

Every fresh session with any agent starts from zero: your
conventions, your project state, your past decisions, all
re-explained or re-derived, in tokens and wrong first attempts.
Platform memory features fix this inside one walled garden each.
Nobody fixes it ACROSS agents, because no vendor has an incentive
to share memory with a competitor's product.

Kumbarium's bet is that the durable value of a personal memory
system is exactly the two things platforms structurally will not
provide: portability across agents, and ownership. One SQLite
file, on your disk, readable without any of the tools that wrote
it.

## The librarian, not the database

Storage is the easy tenth. The product is the memory lifecycle:

- Write path: salience, dedup, contradiction resolution
  (supersede, never delete), provenance.
- Read path: retrieval under a token budget; ranked, scored,
  iterative. Bad memory is worse than no memory, because a stale
  recalled fact is injected with the authority of known context
  and models trust it. Precision beats recall.
- Maintenance: a janitor that PROPOSES (dedup, decay, staleness)
  and a human who DISPOSES. No model silently rewrites ground
  truth.

The enforcement boundary is the librarian process: agents never
touch SQLite. What the harness guarantees is auditability, not
obedience: an agent was told X at 14:32 and ignored it is a
provable statement. That is the same guarantee code review has,
and it is the honest one.

## The longer arc

Identity + gatekeeper + ACLs + audit + dashboard is not a memory
manager; it is an agent control plane, and memory is only the
first resource that flows through it. Shared task state between
agents, config distribution, usage metrics, and rate limits all
fit the same skeleton. Memory ships to completion first; the
boundaries are drawn as if the rest is coming, because it is.
