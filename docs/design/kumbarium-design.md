# Kumbarium design

The architecture of record. Sibling docs: `../thesis.md` (why),
`../decisions.md` (what was decided when, with reasoning).

## Shape

```
agents (Claude / Gemini / Ollama / ...)
        |  MCP: remember / recall / forget / supersede
        v
   the librarian        crates/kumbarium (+ -librarian)
        |
        +-- the Library  library.db   crates/kumbarium-store
        +-- the witness  audit.db     crates/kumbarium-audit
```

The librarian is the SOLE gatekeeper: agents never touch SQLite.
Every request carries a declared agent identity; every
transaction lands in the audit log. The guarantee is
auditability, not obedience.

## The Library (kumbarium-store)

SQLite, WAL mode, FTS5. One `library.db`. Schema highlights
(migrations/0001_init.sql):

- `entries`: UUIDv7 TEXT ids; kind enum (preference /
  project_state / decision / reference) driving type-aware
  decay; provenance (agent_id, source); confidence REAL;
  `superseded_by` chains (supersede, never delete); four
  timestamps (created / updated / last_accessed /
  last_confirmed). Age WITHOUT recent confirmation is the
  strong staleness signal.
- `namespaces`: registered slash-paths, no auto-create.
- `entry_tags`: tags are a filter facet, not primary retrieval.
- `entries_fts`: FTS5 external-content index, trigger-synced.

Numbered migrations, append-only, recorded in `schema_version`.

## Namespaces (kumbarium-librarian)

Registered slash-paths, max 3 segments, `[a-z0-9._-]`:
`global`, `project/<name>`, `agent/<id>/quarantine`. A query
scoped to a namespace searches its CHAIN (itself, ancestors,
global), never siblings: the cross-contamination firewall.
Per-agent ACLs attach to prefixes (post-v0.1).

## Retrieval

Hybrid FTS5 (+ sqlite-vec later) + recency/frequency weighting,
under a token budget: summaries + ids first, full entries on
follow-up. Every hit carries dual scores: `relevance`
(query-time) and `confidence` (entry property) plus a
`confidence_basis` string. Precision beats recall; bad memory is
worse than no memory.

## The witness (kumbarium-audit)

Separate `audit.db`; structured events only (who, when, kind,
scope, JSON detail). Meeting-minutes export is a deterministic
template over `ORDER BY at`; LLM narrative is optional on top.
Planned writer: bounded queue, halt/resume watermarks (~90/~50),
awaitable enqueue as backpressure, ~5s stall then explicit
error. Deliberate trade: availability sacrificed for audit
completeness.

## The janitor (post-v0.1)

LLM-assisted dedup / decay / contradiction detection. Proposes;
the human disposes via a dashboard review queue. Auto-applies
only exact duplicates. Confidence updates are its numeric
product.

## Persisted data

Resolved via `directories` (`kumbarium paths` prints the map):
data dir holds `library.db`, `audit.db`, `kumbarium.lock`
(single-instance, load-bearing), `backups/`, `exports/`,
`logs/`; config dir holds `config.toml` (all tunables, one
file). cache_dir is reserved for a future embedding cache.
Backups: `VACUUM INTO` -> integrity_check -> atomic rename;
every 12h or on launch if elapsed; flat timestamp-named files
with tiering computed by the pruner (2 recent + 7 dailies + 4
weeklies; audit shallower).

## Evals

`evals/` ships ~30 synthetic golden query->memory pairs; CI
scores retrieval against them. Runs against the real library are
audit events with per-query rank diffs vs the previous run. The
repo owns the yardstick; the data dir owns the measurements.

## v0.1 scope

Schema + migrations; FTS5-only retrieval; MCP server with
remember / recall / forget / supersede; synchronous audit
appends; backups. NO janitor, dashboard, vectors, or ACL
enforcement yet (identity is recorded from day one). Gate: if
FTS-only retrieval is not useful in daily driving, embeddings
will not save it.
