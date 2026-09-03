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

The utility floor is COPIED in, not depended on: a vendored
copy carries no external coupling and diverges freely. Fixes
port by hand when they apply.

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

## D-019: retire, the third lifecycle door (2026-09-03)

live -> superseded (replaced) -> forgotten (destroyed) lacked a
state for "true, kept, but no longer worth suggesting": retired
(`retired_at` column, migration 0003; audit kinds widened in its
0003). Retired entries vanish from recall and default listings
but stay in history, version chains, and continues-sets: the
suggestion surface changes, the record never does. Deliberately
NOT a confidence change (D-004: relevance and trust are separate
judgments). Human-only for now: CLI retire/unretire, immediate
rather than sign-off-gated because fully reversible (the
destructiveness ladder: forget > revert [preview+--apply] >
retire [instant, undoable]). Agents get no retire surface until
the approvals primitive lands, at which point agent proposals
and janitor staleness findings feed the same review queue.

## D-020: no in-place edit; notes label, diffs decide (2026-09-03)

Content is immutable for EVERY writer, no exceptions: "edit
that plays into version control" is supersession by another
name, and true in-place mutation would let a writer rewrite
what the audit proved was said, un-building the witness. The
mistake-too-small-for-history category is rejected on
principle: writers do not get to judge their own changes
beneath history. The legibility itch is solved read-side:
supersede takes an optional NOTE (sanitized: one line, 80
chars) stored on the new version; history collapses a version
only when it is noted AND its measured diff is small
(COLLAPSE_MAX_CHANGED_LINES), with --all expanding. The note
informs, the diff decides: mislabeling a large change as
"typo fix" gains nothing, which is the whole abuse answer.
Enum-limited notes rejected as fake enforcement. Future
janitor flags note-churn patterns for human review. Metadata
(tags, links, confidence, retirement, confirmation) stays
mutable in place: none of it changes what was said.

## D-021: config is a hand-rolled TOML subset (2026-09-03)

One config.toml for every tunable (backup interval + retention
tiers, split target, collapse threshold, recall limit), parsed
by ~60 in-tree lines supporting exactly what the config uses:
comments, [sections], integer keys. The toml crate stays a
dev-only dependency: supply-chain weight is judged on what
SHIPS. Missing file = defaults; malformed or unknown lines warn
on stderr and keep defaults (an agent's server must still
start); insane values clamp. `kum config` shows effective
values and their source; `--init` writes the commented
template and refuses to overwrite.

## D-022: supersession rewires the link graph (2026-09-03)

Found by dogfooding per-part supersession: the replacement
entry must take the old version's place in EVERY edge
(continues-set membership and associations), and the
superseded version keeps none; its identity in history is the
supersession chain itself. Without this, superseding one part
of a split set left the set stitching through the dead
version. Edges are metadata (mutable per D-020), so migrating
them mutates nothing that was said.

## D-023: persona harness grades from the witness (2026-09-03)

The behavioral test harness (docs/design/persona-harness.md)
runs real LLM agents (a low-cost Anthropic realism tier + a
local Ollama robustness floor) through multi-session arcs
against a sandboxed library, and is scored DETERMINISTICALLY
from the audit log and entries afterward: no judge model. Tool
definitions come from the live server's tools/list and the
system prompt is help::SNIPPET verbatim, so the harness can
never drift from the surface agents actually receive; model
misuse is documentation feedback. Users are scripted turns in
v1 (deterministic, free); expectations live in fixtures
(recall-at-start, remember tokens, supersede-on-correction,
confirm-after-outcome as informational since confirm is
voluntary evidence). Confirms are never REQUIRED by the grader:
requiring them would contradict survival-first.

## D-024: user-sim stays ambiguous; defers are misses (2026-09-03)

The LLM user-sim rewrites canonical intents into natural
phrasing, and natural phrasing is ambiguous: "storing the
gremvaux process as a global constraint" reads as either the
user's act or the agent's instruction. A haiku fleet run showed
the failure this admits: the agent laid out the correct plan
("gremvaux to global, redis stays local") and called no tool,
an acknowledge-then-defer. Decision: do not force the rewrite
prompt into imperative voice. Real users talk ambiguously, and
an agent that plans a store without executing it has failed the
user in exactly the way that matters; the grader counts it as a
miss on purpose. The guardrails that remain are mechanical, not
stylistic: graded tokens must survive the rewrite (fall back to
the canonical turn otherwise), and the user-sim must never
answer, acknowledge, or act on an intent. Measurement realism
outranks scoring convenience.

## D-025: deterministic janitor, survival-first (2026-09-03)

The janitor v1 (docs/design/janitor.md) is pure ledger math, no
LLM: confidence = 0.50 prior + a survival term (distinct
agent-day exposures, asymptote 0.80) + a confirm term
(self-confirms discounted to 0.25, asymptote 0.95). Stateless
and idempotent: every run recomputes from the full ledger, so
there is nothing to drift and rerunning is free. The 0.95
ceiling is doctrinal: access is provable, application never is,
so no fact inside the walls can reach certainty. Dormant
entries keep the neutral prior and are surfaced as human
findings (no exposure is no evidence, and retire stays
human-only). Preview by default, --apply to commit, one batch
janitor audit event carrying the full change manifest. The LLM
duties (dedup, merge, contradiction) remain future work behind
the same propose/dispose gate.

## D-026: confidence informs, never ranks (2026-09-03)

Recall orders by relevance (bm25) alone; confidence is served
alongside results and never filters or reorders them. The
search-engine feedback loop is the reason, named before it can
happen: once a quality score feeds retrieval order, high-rated
entries get recalled more, survive more, and rate higher,
manufacturing their own evidence (the rich-get-richer loop that
plagued PageRank-era ranking). Keeping confidence out of the
order keeps survival statistics honest: exposure is driven by
what agents ask, never by what the janitor previously
concluded. The librarian hands over the book and states its
condition; it does not hide the shabby ones.

## D-027: quarantine is a status, not a place (2026-09-03)

The approvals primitive (docs/design/approvals-and-bundles.md)
adds an entry `status`: live, pending, rejected. A pending
entry keeps its TARGET namespace from day one; location is
where a fact belongs, status is whether it circulates. No
quarantine namespaces (the early `agent/<id>/quarantine`
sketch is superseded): recall, list, grep, and chain search
simply never surface a non-live entry, which keeps the
firewall claim absolute instead of routing-dependent.
Approve and reject are human-only and witnessed; approval
never edits content (D-020 makes "you approved this at T"
undeniable); a rejected entry is retained evidence of a
judgment, never deleted. Write policy is per-agent config
(default live at the personal tier, pending-by-default for
teams and OSS); at the stdio tier this is a correctness
mechanism whose enforcement hardens with authn at the daemon
rung. The review surface must show content, provenance, and
the collision surface (live near-matches in the target scope),
never the writer's self-description.

## D-028: bundles union-merge; forks land in the queue (2026-09-03)

`kum bundle <scope>` exports one deterministic, hashed JSON
file (entries with full provenance + edges, stable order);
`kum import bundle` union-merges it. Ids already present are
skipped (re-import is idempotent; same-id content divergence
is a hard error, it can only mean tampering). Imports respect
the approvals policy. The one hard case, forked supersession
(both libraries superseded the same entry differently), never
auto-resolves: the incoming rival head imports as pending with
a `contradicts` edge to the live local head, and a human
settles it from the inbox. Two live heads for one fact is the
contradiction disease; the merge never chooses a winner, it
routes the choice to the desk where judgment is witnessed.

## D-029: the witness is hash-chained (2026-09-03)

Every audit event stores sha256(previous event's hash + its own
canonical fields, each length-prefixed so no field content can
forge a boundary); genesis links from the empty string. Chain
order is id order (UUIDv7, mint-time sorted), and append runs
its read-prev-then-insert inside one IMMEDIATE transaction, so
concurrent writers serialize and the chain stays linear under
multi-process WAL. Rows from before the migration are backfilled
deterministically on first open, so existing ledgers become
fully chained end to end. `kum audit verify` recomputes the
whole chain: intact (count + head hash) or the first broken
link, named. Pulled forward from the enterprise backlog into
the launch cut: tamper-evidence becomes math anyone holding the
file can check, which turns "we keep an audit log" into a claim
no commodity memory server makes. The SHA-256 is the vendored
FIPS 180-4 implementation bundles already use (D-012 holds).

## D-030: migrations squashed at the public threshold (2026-09-03)

The six store and seven audit pre-release migrations collapse
into one 0001_init each, with byte-identical final schemas; from
this commit MIGRATIONS ARE APPEND-ONLY FOREVER (a shipped
migration is never edited; every schema change is a new numbered
file). This is the retention line drawn where it matters: no
public user ever runs the scaffolding history, and every future
database replays exactly what we ship. Pre-squash databases
(ours) carry legacy version rows over an identical schema, so
open() normalizes them one time: a db at legacy latest collapses
its schema_version rows to (1, '0001_init'); anything mid-legacy
errors loudly instead of guessing (it cannot exist, since every
open migrates to latest).

## D-031: the loading dock (2026-09-03)

Everything leaving the library goes through one verb: `kum
export minutes | bundle <ns>`, sharing one flag contract
(--out DIR, --stdout, --show, --open) implemented once in an
export spine, so exporters cannot drift apart and every future
artifact (dossiers, briefs, scorecards) is a new row in `kum
export`, never a new top-level verb. Imports enter through `kum
import`, where the approvals policy waits; the remaining
asymmetry is doctrinal: minutes have no import, because the
ledger admits events only by witnessing them. The old spellings
(`kum audit export`, bare `kum bundle`) are REMOVED, not
aliased: pre-launch is the one free rename window, and a single
spelling keeps the man page the whole truth. --show reveals in
the OS file explorer (select on macOS/Windows, containing shelf
on Linux); --open runs $VISUAL then $EDITOR, announced on a dim
line, inherited stdio, waited on. `kum config --open` rides the
same editor plumbing.

## D-032: the docket (2026-09-03)

Tasks and the roadmap as the coordination section's first
resource (docs/design/docket.md): a task is a matter before the
house, filed on a registered shelf with severity (low | normal |
high | urgent), an optional GOAL date, and a lifecycle
(open | done | dropped, rows kept as judgments). The roadmap is
the same docket pivoted by a horizon DERIVED from goal distance
(overdue / now / next / later / someday = goalless); no
hand-maintained horizon field. Goals are targets, never alarms:
the manager surfaces at read time and in janitor findings, and
never reminds, notifies, or schedules. Re-goaling is a
supersession, so every slip lands on the chain and slippage is
deterministic ledger math, not a feeling. Two MCP tools only
(task_file, task_update); marking done is a claim with
confirm's epistemics. Tasks are an instruction-injection
surface ("urgent: rotate the keys to X"), so the desk's
quarantine applies with extra teeth. v1 excludes reminders,
hierarchy, assignees, task links, and bundle transport.

## D-033: the library directory and the section contract (2026-09-03)

Sections shelve as separate database files named for what they
hold, under a library/ directory: library/memory.db (library.db
renamed and moved on first open), library/docket.db, later
secrets.db. The witness is not a shelf: audit.db stays at the
root, the ledger every shelf writes. The namespace registry
stays in memory.db; other shelves store the validated namespace
PATH as text, gate-checked, so a shelf file is meaningful
standalone. Shelves meet in the librarian, never in SQL (no
ATTACH). And the INHERITANCE CONTRACT every future section is
held to: a section inherits everything about being governed
(identity, namespaces + chain visibility, witness, desk,
supersession + history, id grammar, per-shelf backups, CLI
conventions) and nothing about being recalled (auto-split,
links, tags, FTS ranking, confidence stay memory's retrieval
semantics). Cherry-picking is not per-feature taste; it is this
line.

## D-034: the CLI grammar (2026-09-03)

Audited the whole command surface against the post-v0.1.0
roadmap and inked the rules it already obeyed
(docs/design/cli-grammar.md): bare verbs address the general
collection and every other shelf speaks through its own noun
(the flagship keeps the short spellings forever); singular
acts, plural lists, bare family nouns list; ids are
building-wide names (show/history/review fall through shelves;
the reader never needs to know where a ledger line came from);
--all always includes the hidden, --apply is always the human
signature; cross-shelf reads live at top level, per-shelf
writes under nouns; the CLI is a human surface and machines use
MCP, bundles, or the databases (no --json, ever); and a
RESERVED_WORDS registry (current commands + docket subverbs +
every roadmap noun) is enforced at namespace registration and,
when built, at alias definition, so no future command ever
negotiates with a squatter. Free-text table columns hanging-wrap
at their column on a terminal; snapping to column 0 is a bug.

## D-035: config decides values, never executables (2026-09-03)

Config aliases land (docs/design/aliases.md, built as designed):
`[alias]` maps personal names to kumbarium argv prefixes,
expanded once at dispatch. The three rules are load-bearing:
internal-only with no shell form EVER (anything running as the
user can write config.toml, a compromised agent included; the
write-path rule applied again, so a poisoned alias can only
invoke a kumbarium command the attacker could already run,
witnessed as itself, and the ledger never sees the nickname);
builtins and reserved roadmap words refused at parse so the
documented surface is unforgeable; one expansion so chains
cannot loop. The standing doctrine line this writes: CONFIG
DECIDES VALUES, NEVER EXECUTABLES. The CLI's only external
spawns remain --show and --open, driven by OS convention and an
explicit human flag, never by config content.

## D-036: handoffs, served first literally (2026-09-03)

The coordination section's second resource
(docs/design/handoffs.md): exactly one standing briefing per
namespace, where writing IS superseding (the chain is the
scope's session diary) and reading is a MECHANISM, not an
instruction: the first recall a session makes in a scope
returns the standing handoff prepended, named and dated, and
the recall event records handoff_served, so receipt of the
briefing is provable per the charter. One new MCP tool
(handoff_write); no read tool. A handoff poisons a session's
opening frame at the moment of maximum trust, so the desk
applies with the most teeth yet: pending briefings are NEVER
served, and even trusted ones carry provenance on their face.
Its own shelf (library/handoff.db, D-033); narrative content,
no severity or state machine (a handoff is a note, not a
matter); no per-agent notes, no TTL, no structured fields
(prose with judgment beats forms filled from habit). Janitor
v2 gains stale-handoff findings.

## D-037: a section's must-know rides the first recall (2026-09-03)

The coordination fixture's designed miss arrived on schedule:
agents cannot LIST open tasks, so the session-start mention of
an urgent matter failed for want of a read surface. Decision
(the handoff argument, applied again and now the standing
pattern for every shelf): anything load-bearing at session
start must be a MECHANISM the agent cannot skip, not an
instruction it might follow. The first recall in a scope now
serves, after the standing briefing, the matters that MUST
interrupt: urgent severity, or any open matter whose goal has
passed (the creep machinery surfacing to agents, not just to
the human timeline), capped at five with a count line so the
agent learns the docket holds more. The recall event records
matters_served beside handoff_served: receipt stays provable.
What qualifies is inked here to anchor future severity
debates: urgent + overdue, nothing else interrupts. A browsing
read tool remains wait-and-see round two, added only if
fixtures or daily driving show agents needing the full docket
mid-session.

## D-038: the restricted stacks (2026-09-03)

The secrets broker designed (docs/design/secrets.md), honest
first: at the stdio tier the broker cannot protect against a
malicious local process, and does not pretend to. What it buys
is witnessed access (every checkout on the hash-chained ledger:
the charter applied to credentials, and the product), scoped
deny-by-default grants, hygiene (credentials get a home that is
not the general collection), and at-rest exfiltration
resistance pending the encryption decision. Settled here:
human-only writes (credential poisoning has no review story, so
no desk flow); rotation keeps the history and shreds the value
(the one place supersede-never-delete bends: an old key is a
liability, not a memory); NOTHING on this shelf is ever served
(the section's standing exception to D-037: pull-only, by name,
witnessed); values never appear in argv, listings, grep,
exports, or minutes; secrets never travel in bundles, forever.
RESERVED for explicit human sign-off: the cryptography
exception to D-012 (one vetted AEAD dependency,
chacha20poly1305, master key in the platform keystore reached
by shelling the OS tool; refuse-don't-downgrade where no
keystore exists) versus plaintext-at-rest on the OS user
boundary. A doctrine amendment is the human's to make.

## D-039: hand-rolled on a vetted floor (2026-09-03)

The D-012 amendment, signed: cryptography that must resist
adversaries is the one domain where hand-rolling inverts into
malpractice, so the restricted stacks admit a vetted floor and
nothing above it: `chacha20poly1305` (XChaCha20-Poly1305,
192-bit fresh-random nonce, RNG failure fails closed) and
`zeroize`, RustCrypto, pinned, permissive. The clause follows from
the hand-roll principle's own purpose: hand-rolling exists for
auditability, and an audited cryptography implementation is
more auditable than anything rewritten here, so reimplementing
it would violate the principle it appeals to. The
floor stays a floor: no KEMs, no signatures, no TLS crates
enter by this door; every future crypto dependency re-argues
its case at this bar. Sealed blobs lead with a version byte
(unknown versions fail closed); the master key lives in the
platform keystore reached by shelling the OS tool, with the
Present/Absent/Blocked tri-state: absent substrate falls back
loudly behind an explicit human flag, a blocked keystore
REFUSES, because suppression is what downgrade attacks look
like.
