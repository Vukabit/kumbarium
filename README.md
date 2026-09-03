# Kumbarium

*koom-BAH-ree-um* -- Swahili *kumbuka* (remember) + Latin *-arium*
(place of): the place of remembering.

Kumbarium is an agent-agnostic memory system: one local,
inspectable SQLite store of long-term memory that any AI agent
(Claude, Gemini, an Ollama model, whatever comes next) shares
through a single gatekeeper process speaking MCP. Your memories
live in one file you can grep, edit, back up, and point a new
tool at years from now. Local-first, single binary, no cloud, no
Docker.

Status: pre-v0.1 scaffold. See `docs/thesis.md` for the why and
`docs/design/kumbarium-design.md` for the architecture.

## Layout

```
crates/kumbarium            the binary: MCP server + CLI
crates/kumbarium-util       vendored [redacted]-util fork
crates/kumbarium-store      the Library: schema + FTS5 + backups
crates/kumbarium-audit      the witness: append-only event log
crates/kumbarium-librarian  the brain: ranking, scoring, curation
docs/                       thesis, decisions log, design docs
evals/                      synthetic retrieval golden set (CI)
scripts/                    the gate battery (scripts/gate.sh)
```

## Where your data lives

Everything persisted lives under the platform data directory
(`kumbarium paths` prints the resolved map for your machine):

```
<data>/kumbarium/
  library.db      the memory store
  audit.db        the audit event log
  kumbarium.lock  single-instance process lock
  backups/        tiered, timestamp-named snapshots
  exports/        audit exports (minutes / JSON / CSV)
  logs/           process logs
<config>/kumbarium/config.toml
```

A full backup of everything Kumbarium knows is a copy of one
directory.

## Developing

`scripts/gate.sh` runs the full local gate battery (fmt, clippy,
tests, 80-column width, no-scaffolding, manifest hygiene,
licenses). The repo-wide rules: 80 columns max in every file,
and no em-dashes in committed work.

## License

MIT OR Apache-2.0, at your option.
