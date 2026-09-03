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

## D-011: porter stemming + OR recall (2026-09-02)

The FTS index uses the porter tokenizer (queries phrase things
differently than stored content). Recall sanitizes every token
into a quoted phrase (agent input can never raise an FTS syntax
error) and joins with OR: bm25 still ranks multi-term matches
first, but one missing word cannot blank a result. Known limit,
by design: zero-term-overlap semantic queries miss entirely; the
eval set keeps such cases as the measure of whether embeddings
later earn their place.

## D-012: minimal-dependency stance (2026-09-02)

Permissive licenses only (deny.toml enforces) AND as few deps as
possible: prefer building our own code; supply-chain attacks are
rising. The standing stance: vendor exactly what
we need, nothing more. Weighs directly on the MCP SDK choice
(D-009): hand-rolling the stdio JSON-RPC transport is on the
table.

## D-013: migrations squash to a v0.1 baseline (2026-09-02)

Append-only migration discipline binds from the FIRST database
anyone would be sad to lose, which is the author's own library the
day daily-driving starts (the MCP server landing), NOT public
launch. Until that day, squashing into one 0001_init baseline is
allowed and was done here (0002_fts_porter folded in). After
that day: shipped migrations are frozen, changes are new
numbered files, no exceptions.

## D-014: hand-rolled MCP stdio server, no async (2026-09-02)

Resolves D-009. The MCP layer is written in-tree: newline-
delimited JSON-RPC 2.0 over stdio, legacy-era handshake
(initialize / initialized / ping / tools/list / tools/call),
which every dual-era client supports through the protocol
transition. Rationale: D-012 (rmcp pulls tokio + ~dozens of
transitive crates for what is ~400 lines here), and a blocking
single-thread loop pairs with sync rusqlite with no runtime
bridge. NO async runtime in the tree. The modern stateless
revision (2026-07-28, server/discover + per-request _meta) gets
added in-tree when clients start requiring it; rmcp (Apache-2.0,
allowlist-clean) stays the escape hatch if scope ever explodes
(e.g. streamable HTTP daemon mode).

## D-015: process lock scoped to maintenance (2026-09-02)

stdio transport means each client spawns its own server process,
so concurrent agents = concurrent librarian processes. SQLite
WAL handles that safely; the sole-gatekeeper guarantee is a
property of the code path (agents never touch SQLite directly),
not of a single PID. kumbarium.lock therefore guards
MAINTENANCE operations only (backups, migrations beyond open,
future janitor), never request serving. A single-daemon + HTTP
shape stays open for later.

## D-016: typed edges, not a linked list (2026-09-02)

Relations between entries are one `entry_links` table
(from, to, rel: continues / relates_to / duplicates /
contradicts) rather than per-purpose pointer columns. A split
memory's parts chain with 'continues' edges (the linked list is
one row-type of the mechanism); imported [[wiki-links]] land as
'relates_to'; the janitor's future findings have a home.
`superseded_by` stays a COLUMN: load-bearing for recall
filtering and enforced-linear. Recall renders a hit's edges as
ids only; agents fetch siblings on demand (token budgeting, no
auto-chain inflation). Shipped as migrations 0002 in BOTH dbs,
the first real append-only migrations (audit's rebuilds the
events table to widen the kind CHECK with 'link'/'import').

## D-017: librarian-side auto-split on every write (2026-09-03)

Oversized content is split by the LIBRARIAN, not the writer:
deterministic paragraph-boundary packing (blank-line blocks,
markdown headings break early once a part is half full, a
paragraph is never cut internally; an indivisible oversized
paragraph passes whole). Parts chain with `continues` edges
(later part points at its predecessor); part 1 is the head and
carries the request's tags and explicit links; all parts share
tags and source. One shared write path (tools::store_split)
serves agent remember/supersede AND the importer, so origin
never changes split behavior. Target: SPLIT_TARGET = 1500 bytes
in kumbarium-librarian. Consequences, documented in the tool
descriptions: parts rank independently in FTS (sharper, cheaper
recall); supersede and forget operate per PART, which is the
fine-grained history we want. Smarter semantic splitting is a
future Curator job, never this deterministic path. The
importer's advisory oversize warning is replaced by the split
itself. Also: `kum` ships as a second bin target aliasing the
full CLI.

## D-018: law lives in repos; Kumbarium authors it later (2026-09-03)

Repo-level instruction files (CLAUDE.md and kin) STAY in repos.
Three properties make them law that the store cannot replace:
harness-side unconditional injection (recall is voluntary and
ranked; law must be forced), versioning with the code (clones,
branches, CI machines get the law without our database), and
change review native to git. The future shape, when rules
multiply across repos or contributors: Kumbarium as system of
record (entries tagged as law, with supersession history and
audit) and `kum export law <scope>` GENERATING the files
deterministically, committed like lockfiles; drift becomes a
gate check; promoting memory to law becomes a human sign-off.
That is the first concrete face of the control-plane policy
layer. Deliberately NOT built yet: two law files and one
contributor make hand-maintenance free. Authority tiers to
remember: gates (hard) > injected law (soft) > recallable
memory (reference); rules live at the lowest tier that works
and graduate on evidence.
