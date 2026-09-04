# Kumbarium

*koom-BAH-ree-um*, Swahili *kumbuka* "remember" plus Latin
*-arium* "place of". The place of remembering.

**One memory, every agent, every checkout on the record.**

Young and validated: v0.1 is personal-scale, correct at the
core, and pre-1.0 surfaces may still move. The decisions log
says what is settled and why.

Kumbarium is a local-first, agent-agnostic memory library for
AI agents. Claude, Gemini, and local models share one store
over MCP; a deterministic Rust librarian is the sole
gatekeeper; and a separate hash-chained ledger witnesses every
transaction. One static binary, two SQLite files, no cloud, no
daemon required.

## The charter

It is a library in every sense:

- We know they checked the book out. Every recall is logged
  with the exact entries returned.
- We know the condition it came back in. Every alteration is
  an attributed, immutable-history write.
- We know silence. A fact recalled again and again and never
  corrected is a fact surviving use.
- We honestly do not know what they did with it outside the
  walls. Access is provable; application never is.

That last line is load-bearing. Kumbarium promises
**auditability, not obedience**: it cannot force an agent to
use what it was told, so instead it makes every exchange
provable. The ledger is a hash chain: rewrite or remove any
event and `kum audit verify` names the first broken link.
Tamper-evidence is math anyone holding the file can check, not
a promise.

## What agents get

Six MCP tools, learned from the schemas alone:

| verb | what it does |
|---|---|
| `remember` | store a durable fact (oversized content splits itself) |
| `recall` | ranked search over the scope's chain, never a sibling's |
| `confirm` | volunteer evidence a recalled fact proved correct in use |
| `supersede` | replace a stale fact; the old version chains forward |
| `link` | typed edges: relates_to, continues, duplicates, contradicts |
| `forget` | hard-remove wrong or sensitive content |

## What you get

The human runs the porcelain: `kum list / show / grep /
history / revert` (preview plus `--apply` sign-off), `retire`,
namespaces (registered by you, never invented by an agent),
timestamped backups, deterministic meeting-minutes export, and
three pieces agents cannot touch:

- **The janitor** (`kum janitor`): the only mover of the
  confidence number, and pure ledger math. Survival is the
  backbone: recalled across agents and days, never corrected.
  Confirms are garnish; self-confirms discounted; ceiling
  0.95, because nothing inside the walls can prove
  application. Confidence informs recall output; it never
  ranks or filters it.
- **The circulation desk** (`kum inbox / review / approve /
  reject`): untrusted writers' facts land pending, invisible
  to every recall, until a human judges them, seeing content,
  provenance, and the live near-matches already on the shelf.
  Blame lands where judgment happened, provably.
- **Bundles** (`kum export bundle`, `kum import bundle`): a
  shelf as one hashed JSON file. Union-merge, idempotent,
  tamper-refusing. When two libraries superseded the same fact
  differently, the merge never picks a winner: the rival lands
  in the inbox with a `contradicts` edge, and you decide.

## Ninety seconds of witness

Three agents, three projects, one brain, from a real run:

```
time      kind      agent  scope             detail
07:17:05  recall    alpha  project/alphaapp  nothing for "gremvaux"
07:17:06  remember  alpha  global            remembered bec9b7f5
07:17:32  recall    beta   project/betasvc   1 memory: bec9b7f5
```

Agent alpha judged an org-wide fact onto the global shelf;
agent beta, a different model in a different project, found it
in a fresh session. The firewall held: across every validation
run ever, no recall has returned a sibling project's entry or
a non-live one.

## Quickstart

Needs a Rust toolchain (`rustup.rs`). Linux, macOS, and
Windows are all CI-gated.

```
cargo install --git https://github.com/Vukabit/kumbarium
# installs both names: kumbarium, and kum as the short alias
kum namespace add project/my-app "what my-app knows"
kum instructions      # per-agent wiring, e.g. for Claude Code:
claude mcp add --scope user kumbarium -- kumbarium serve
kum instructions --snippet >> ~/.claude/CLAUDE.md
```

Everything lives in one directory (`kum paths` shows where); a
full backup is `cp -r` of it.

## How it earned its claims

The repo ships its own examiners, and their reports:

- a **systems harness** that seeds, churns, and storms a
  throwaway library: 60/60 retrieval precision, four
  concurrent server processes, zero failures.
- a **persona harness** where real LLMs, given only the tool
  schemas and the standard snippet, run multi-session arcs
  while the witness grades the exam deterministically. An 8B
  local model passes every check. When every tier failed one
  behavior (superseding on correction), the fix was one
  documentation sentence, and the harness verified it at both
  ends of the ladder. Misuse is documentation feedback here.

All of it in `docs/reports/`, and every design decision with
its reasoning in `docs/decisions.md` (forty and
counting).

## Design stance

Rust 2024, nine crates, one binary. SQLite (bundled, WAL, FTS5)
for both the library and the ledger. No async runtime, no MCP
framework, no diff library, no hash library: the JSON-RPC
loop, LCS diff, SHA-256, TOML subset, and terminal rendering
are a few hundred lines each and vendored in full. Shipped
dependencies are the ones you would keep under oath: rusqlite,
serde, thiserror, uuid, libc, directories. Supply chains are
attack surface; this one is short enough to read.

## Contributing

Run the gate battery before a PR: `cargo +nightly fmt`,
clippy, tests, and the Python gates (see
`.github/workflows/ci.yml` for the exact set). Design changes
start in `docs/decisions.md`; that log is the project's memory
and every entry carries its reasoning. Memory-content
contributions travel as bundles and go through the circulation
desk, same as any untrusted write.

## What it deliberately is not

No daemon, no dashboard, no embeddings, no cloud sync, no
orchestration; some not yet, some not ever. Kumbarium is a
manager, not an orchestrator: agents check things out from it
and answer to it; it never schedules or directs their work.
The boundaries are documented next to the features, with
reasons.

## Security

Kumbarium holds private data by design. Report
vulnerabilities privately through GitHub's security advisories
(the Security tab's "Report a vulnerability"), never in a
public issue.

## License

MIT or Apache-2.0, at your option.
