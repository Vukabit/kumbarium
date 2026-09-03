# The CLI grammar (D-034)

The rules the command surface already obeys, written down so
every future section starts from them instead of re-deriving.
Audited 2026-09-03 against the full post-v0.1.0 roadmap
(handoffs, secrets, janitor v2, daemon, dashboard, aliases, the
manager's paperwork).

## The default referent

Bare verbs address the general collection: `list`, `show`,
`grep`, `history`, `revert`, `retire`, `confirm`, `move` mean
memory, the way git's bare verbs mean the working tree. Every
other shelf speaks through its own noun: `task ...` today,
`secret ...` and `handoff ...` when those sections open. The
flagship keeps the short spellings; sections scale to N without
ever renaming the daily drivers.

## Singular acts, plural lists

`kum task <ns> ...` files one matter; `kum tasks` is the
timeline. Future sections inherit the pair for free (`kum
secret add` / `kum secrets`). A verb that IS its own output
takes no sub-word: `kum tasks list` and `kum inbox list` do not
exist. Bare family nouns list (`kum namespace` = `kum namespace
list`).

## Ids are building-wide names

Any id the witness ever prints is inspectable by one command:
`show`, `history`, and the desk verbs resolve on the memory
shelf first and fall through to the docket (and to future
shelves as they open). UUIDv7 across shelves makes collision a
non-issue; the reader should never need to know which shelf a
ledger line came from.

## Flag law

- `--all` always means "include what the default hides"
  (superseded, retired, judged, non-live).
- `--apply` always means the human signature on a previewed
  destructive change; preview is always the default.
- Read verbs take the namespace as an optional positional
  filter; write verbs take it first, before content.
- Export flags (`--out`, `--stdout`, `--show`, `--open`) exist
  only on the loading dock and are implemented once (D-031).

## Reads at the top, writes under nouns

Cross-shelf reports live at top level: `status`, `inbox`,
`roadmap` today; `brief`, `incident`, `scorecard` when the
paperwork suite lands. Per-shelf writes live under the shelf's
noun. The desk's verbs (`review`, `approve`, `reject`) are top
level BECAUSE they are cross-shelf: one queue, every shelf.

## The CLI is a human surface

No `--json`, ever. Machines already have three doors: MCP, the
bundle format, and the SQLite files themselves. Piped CLI
output stays stable and grep-friendly (plain, single-line,
byte-identical to what the renderer wraps on a terminal), but
it is a courtesy, not a schema.

## Reserved words

`kumbarium_librarian::RESERVED_WORDS` holds every current
command word, every docket subverb, and the roadmap's future
nouns (mem, memory, secret(s), handoff(s), brief, incident,
scorecard(s), daemon, dashboard, alias(es), section(s),
shelf/shelves). Two gates enforce it: `namespace add` refuses a
reserved SINGLE-SEGMENT name (multi-segment `project/brief` is
always fine, the slash disambiguates), and the alias feature
checks the same list when it lands. This is the cheap insurance
that no future command ever has to negotiate with a squatter.

## Hanging wrap

Every table with a free-text final column wraps it at the
column start on a terminal, measured against real width
(`audit tail`, `export minutes --stdout`, `kum tasks`,
`kum roadmap`); piped output stays single-line for grep. A
table that snaps to column 0 is a bug.
