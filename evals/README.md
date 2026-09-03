# Retrieval evals

The yardstick: synthetic golden query->memory pairs that CI
scores retrieval against. Without this, a ranking change's
effect is invisible (memory failures are non-events: the agent
just does not know something it should have been told).

Rules:

- SYNTHETIC only. No real memories, ever; this ships publicly.
- Grows toward ~30 cases as retrieval lands in v0.1.
- Runs against a real library are audit events with per-query
  rank diffs vs the previous run; those live in audit.db, not
  here. Repo owns the yardstick, data dir owns measurements.

`golden.toml` format: each `[[case]]` seeds `entries` into a
fresh in-memory store, issues `query` in `scope`, and asserts
the entry tagged `expect` ranks first. A case marked
`semantic = true` is beyond lexical FTS reach (zero term
overlap, D-011): the runner reports its outcome but does not
fail on it; these cases are the yardstick for whether embeddings
later earn their place. The runner is
`crates/kumbarium/tests/golden_evals.rs` (runs in `cargo test`,
so gate.sh and CI both execute it).
