# Decisions

Running log, newest last. One entry per decision that shapes the
code; the reasoning lives here so it never has to be re-argued.

## D-001: the name (2026-09-02)

Kumbarium: Swahili *kumbuka* (remember) + Latin *-arium* (place
of). Verified free on crates.io; zero GitHub repo collisions.

## D-002: the librarian is software, not an LLM (2026-09-02)

The gatekeeper process is deterministic Rust. An LLM (Ollama) is
a tool it calls on the WRITE path (dedup / merge / contradiction)
and in the janitor, never in the hot read path: reads stay
millisecond-fast, deterministic, and debuggable.

## D-003: supersede, never delete (2026-09-02)

Contradictions chain forward via `superseded_by`. History is
free, bad merges are reversible, and the janitor gets a numeric
product (confidence) instead of a delete button.

## D-004: dual scores on every recall hit (2026-09-02)

Relevance (query-time match) and confidence (entry
trustworthiness) are different numbers; both travel to the agent
with a human-readable `confidence_basis`. Conflating them makes
a shaky-but-relevant memory indistinguishable from a solid
tangent.

## D-005: strict ISO-8601 TEXT timestamps (2026-09-02)

kumbarium-util's canonical ms-precision UTC format. Lexicographic
order equals chronological order, rows stay human-readable, and
parse/format is already property-tested in the vendored util.

## D-006: vendored util, intentional fork (2026-09-02)

[redacted]-util is COPIED in ([redacted] @ 848d14e), not depended on:
each project's utils diverge on purpose. Cross-port fixes by
hand.

## D-007: audit writes are synchronous for now (2026-09-02)

v0.1 appends directly. The designed bounded buffered writer
(two watermarks, halt ~90% / resume ~50%, awaitable enqueue as
backpressure, stall timeout ~5s, availability traded for audit
completeness) replaces it when traffic warrants; the event
schema is already shaped for it.

## D-008: publish = false until launch (2026-09-02)

Crates stay unpublishable while pre-v0.1 (cargo_hygiene.py
enforces it). Flipping to publishable is a deliberate
launch-time decision, with the hygiene check updated in the same
commit.

## D-009: async runtime undecided (2026-09-02)

Nothing yet forces tokio. The MCP SDK choice will decide this;
until then, no async in the tree.

## D-010: CLI crate exempt from no_scaffolding (2026-09-02)

`crates/kumbarium` may println; that is its job. Library crates
(kumbarium-*) stay in scope for the gate.
