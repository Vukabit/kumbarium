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
the entry tagged `expect` ranks first.
