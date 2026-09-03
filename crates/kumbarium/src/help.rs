//! Per-topic manual pages: syntax, the input grammar each
//! command speaks, and worked examples. Authored as markdown so
//! the body renders through the same highlighter memory bodies
//! use; plain when piped, colored on a terminal.

pub const TOPICS: &str = "list show history revert retire \
import namespace audit backup serve ids namespaces \
instructions status grep move janitor approvals export docket \
alias handoff";

pub fn page(topic: &str) -> Option<&'static str> {
  Some(match topic {
    "list" => PAGE_LIST,
    "show" => PAGE_SHOW,
    "history" => PAGE_HISTORY,
    "revert" => PAGE_REVERT,
    "retire" | "unretire" => PAGE_RETIRE,
    "import" => PAGE_IMPORT,
    "namespace" => PAGE_NAMESPACE,
    "audit" => PAGE_AUDIT,
    "backup" => PAGE_BACKUP,
    "serve" => PAGE_SERVE,
    "ids" | "id" => PAGE_IDS,
    "instructions" | "setup" => PAGE_INSTRUCTIONS,
    "status" => PAGE_STATUS,
    "grep" => PAGE_GREP,
    "move" => PAGE_MOVE,
    "namespaces" | "scopes" => PAGE_NAMESPACES,
    "janitor" => PAGE_JANITOR,
    "approvals" | "inbox" | "review" | "approve" | "reject" => PAGE_APPROVALS,
    "export" | "bundle" | "bundles" | "minutes" => PAGE_EXPORT,
    "docket" | "task" | "tasks" | "roadmap" => PAGE_DOCKET,
    "alias" | "aliases" => PAGE_ALIAS,
    "handoff" | "handoffs" => PAGE_HANDOFF,
    _ => return None,
  })
}

const PAGE_IDS: &str = "\
## ids: the id grammar

Every entry has a UUIDv7 id. Listings show its **short form**:
the last 8 hex chars (fronts are timestamps and collide within
a batch; tails are random).

Anywhere a command takes an id, you may give:

- the full id: `01a06550-ec55-7470-88d9-e009dfeb4d7c`
- the short form: `dfeb4d7c`
- ANY unique fragment of 4+ hex chars: `e009d`

Ambiguous fragments error and never guess:

```
kum show dfeb          # ok if unique
kum show 01a0          # AmbiguousId: matches many
```
";

const PAGE_NAMESPACES: &str = "\
## namespaces: the scope grammar

Slash paths, 1-3 segments of `[a-z0-9._-]`, registered by you
(agents cannot create them):

- `global`: cross-project facts
- `project/<name>`: one project's facts

Quarantine is NOT a namespace: an untrusted writer's entries
land in their target namespace with pending status instead
(D-027; see `kum help approvals`).

Recall searches a scope's CHAIN: itself, its ancestors, then
`global`; never a sibling. `project/web.app` searches
`project/web.app`, `project`, `global`.
";

const PAGE_LIST: &str = "\
## list: browse entries

```
kum list [namespace] [--all]
```

Newest first: short id, created day, kind, namespace, first
content line. Default hides superseded and retired entries;
`--all` shows them with `[superseded]` / `[retired]` markers.
The namespace filter is EXACT (browsing does not chain; recall
does).

```
kum list
kum list project/kumbarium
kum list global --all
```
";

const PAGE_SHOW: &str = "\
## show: one entry in full

```
kum show <id> [--full]
```

All fields, tags, links, and the supersession neighbors. On a
member of a split set, prints `set: part i of n`; `--full`
stitches every part of the set in order, markdown-rendered.
See `kum help ids` for what <id> accepts.

```
kum show dfeb4d7c
kum show dfeb4d7c --full
```
";

const PAGE_HISTORY: &str = "\
## history: a fact's versions

```
kum history <id> [--diff]
```

The supersession chain containing <id>, newest first, live
version marked. `--diff` adds line diffs between adjacent
versions. Version numbers (v1, v2, ...) are display ordinals
computed from chain position, oldest = v1; they can renumber if
the chain changes. Short ids are the stable names; commands
take ids, never vN.

```
kum history dfeb4d7c
kum history dfeb4d7c --diff
```
";

const PAGE_REVERT: &str = "\
## revert: restore an old version (sign-off gated)

```
kum revert <id-of-ancestor> [--apply]
```

Supersedes the LIVE version with the content of an ancestor
version (find one via `kum history`). Without `--apply` it only
previews: the plan plus a full diff, nothing written. `--apply`
is the human signature. CLI-only by design: agents have no
revert tool. Reverting the live version itself is refused.
Oversized restored content auto-splits like any write.

```
kum history dfeb4d7c          # find the ancestor id
kum revert d0b96f52           # preview + diff
kum revert d0b96f52 --apply   # sign off
```
";

const PAGE_RETIRE: &str = "\
## retire / unretire: the suggestion surface

```
kum retire <id>
kum unretire <id>
```

Retired = still true, still kept, no longer SUGGESTED: hidden
from recall and default listings, but present in history,
version chains, sets, and `list --all`. Not a trust judgment
(confidence is separate). Immediate but fully reversible;
human-only (agents cannot retire). Use `forget` only for wrong
or sensitive content; use `supersede` when a replacement fact
exists; retire when something is simply no longer relevant.

```
kum retire 8d39dd36
kum list project/[redacted] --all   # shows [retired]
kum unretire 8d39dd36
```
";

const PAGE_IMPORT: &str = "\
## import claude: migrate Claude Code auto-memories

```
kum import claude [--dir <path>]... [--map name=ns]... [--apply]
```

Reads `~/.claude/projects/*/memory/*.md`, maps frontmatter to
entries, `[[wiki-links]]` to relates_to edges (dangling ones
become tags). DRY RUN by default: prints the full plan (target
namespace, kind, part count) and writes nothing without
`--apply`. Idempotent: already-imported files are skipped, so
re-running imports only the delta. Oversized memories split
automatically.

```
kum import claude
kum import claude --map kumbarium=project/kumbarium --apply
```
";

const PAGE_NAMESPACE: &str = "\
## namespace: the registry

```
kum namespace add <path> [description]
kum namespace list
```

Namespaces are registered-only: agents can never create one,
which is the firewall against taxonomy drift. Path grammar in
`kum help namespaces`.

```
kum namespace add project/my-app \"the new thing\"
kum namespace list
```
";

const PAGE_AUDIT: &str = "\
## audit: the witness

```
kum audit tail [n] [--scope <ns>]
kum audit verify
```

Minutes leave through the loading dock: `kum export minutes`
(see `kum help export`).

Every librarian transaction is an event: who (agent identity),
when, what, in which scope. `tail` shows the most recent n
(default 20) as prose. Storage is always strict ISO-8601 UTC;
rendering localizes.

The ledger is HASH-CHAINED (D-029): each event stores
sha256(previous hash + its own fields), so `verify` recomputes
the whole chain and either confirms it intact (event count +
head hash) or names the first broken link. Tamper-evidence is
math anyone holding the file can check, not a promise.

```
kum audit tail 50
kum audit verify
kum export minutes --stdout | less
```
";

const PAGE_BACKUP: &str = "\
## backup: snapshots

```
kum backup
```

Forces a snapshot of both databases now: VACUUM INTO a temp
file, integrity-checked, atomically renamed into backups/.
Normally you never run this: every `serve` startup snapshots
automatically when 12h have elapsed. Retention is tiered
(library: 2 recent + 7 dailies + 4 weeklies) and computed from
the flat timestamped filenames; unrecognized files are never
touched.
";

const PAGE_SERVE: &str = "\
## serve: the MCP server

```
kum serve
```

Speaks MCP over stdio: newline-delimited JSON-RPC 2.0. Not for
humans; agents' clients spawn it (`claude mcp add kumbarium --
~/.cargo/bin/kumbarium serve`). Tools: remember, link, recall,
confirm, supersede, forget, task_file, task_update. Every call
is audited under the identity the client declared at
initialize. stdout carries protocol only; diagnostics go to
stderr.
";

/// The copy-paste block for an agent's root instruction file.
/// Deliberately agent-neutral wording; `kum instructions
/// --snippet` prints exactly this for appending to a file.
pub const SNIPPET: &str = "\
# Memory: Kumbarium

Long-term memory lives in the Kumbarium MCP tools, shared
across all agents and sessions.

- At the start of substantive work, `recall` with scope
  `project/<name>` for the current project (or `global`).
- `remember` durable new facts: preferences, decisions,
  standing constraints. Project facts go in the project
  namespace; cross-project facts in `global`. Send content
  whole; oversized memories are split and linked for you.
- When a recalled fact proves CORRECT in use, `confirm` it
  (evidence for the staleness and confidence signals).
- When a fact proves stale or wrong (a user correction counts),
  update it with `supersede`, never with a fresh `remember`:
  first `recall` the stale entry, then supersede the id it
  returned (add a short `note` like 'typo fix' for trivial
  changes). `forget` only wrong-or-sensitive content.
- Before ENDING substantive work, leave the briefing with
  `handoff_write`: what is mid-flight, decided-but-unfinished,
  sharp edges. The next session receives it automatically with
  its first recall; write it for them.
- The DOCKET is the shared task list. Urgent or overdue
  matters arrive automatically with your first recall in a
  scope: mention them before starting new work. File matters
  worth doing later
  with `task_file` (severity is your judgment; add a goal date
  only when one is real). Shelving follows the same rule as
  facts: a project's tasks go on the project's namespace,
  `global` only for genuinely cross-project matters.
  `task_update` marks them done when
  the work is complete, or regrades severity and goal as
  reality moves.
- Namespaces are registered by the user (`kumbarium namespace
  add`); never assume one exists, ask instead.
";

const PAGE_INSTRUCTIONS: &str = "\
## instructions: wiring agents up

```
kum instructions             this page
kum instructions --snippet   just the block, for appending
```

Two steps per agent: register the MCP server, then add the
standing instruction block so the agent uses it unprompted.

**1. Register the server**

- Claude Code:
  `claude mcp add --scope user kumbarium -- kumbarium serve`
- Gemini CLI: in `~/.gemini/settings.json` under
  `mcpServers`:
  `{ \"kumbarium\": { \"command\": \"kumbarium\",
  \"args\": [\"serve\"] } }`
- Any MCP client: stdio transport, command `kumbarium serve`.

**2. Add the instruction block** (append with
`kum instructions --snippet >> <file>`)

- Claude Code, all projects: `~/.claude/CLAUDE.md`
- Claude Code, one repo: `<repo>/CLAUDE.md`
- Gemini CLI: `~/.gemini/GEMINI.md` (or repo `GEMINI.md`)
- OpenAI Codex CLI: `~/.codex/AGENTS.md` (or repo `AGENTS.md`)
- Cursor / Windsurf / others: the repo `AGENTS.md` convention
  is increasingly honored; otherwise their rules file
  (`.cursor/rules/`, `.windsurfrules`).

Root-level files apply everywhere; repo files version with the
code. Keep durable law in files (they are injected every
session); keep evolving facts in Kumbarium (they are recalled).
";

const PAGE_STATUS: &str = "\
## status: library health at a glance

```
kum status
```

Entry counts (live / superseded / retired), split sets, live
entries per namespace, audit event count and latest, backup
ages, database sizes. The first command to run when wondering
what state things are in.
";

const PAGE_HANDOFF: &str = "\
## handoffs: the standing briefings

```
kum handoff <ns> <note...>    leave the briefing (supersedes)
kum handoff <ns>              read the standing briefing
kum handoffs                  every shelf's standing briefing
```

Exactly one standing briefing per shelf: what is mid-flight,
decided-but-unfinished, and sharp-edged, for the NEXT session.
Writing replaces the previous one; the chain is the scope's
session diary (`kum history <id>` on any briefing reads it).

Served first, literally: the first recall an agent session
makes in a scope receives the standing briefing prepended,
named and dated, and the recall event records handoff_served,
so receipt is provable. There is no read tool to forget to
call.

A briefing poisons a session's OPENING FRAME at maximum trust,
so the desk applies with the most teeth: a quarantined writer's
briefing lands pending and is NEVER served; approval makes it
THE standing note (superseding the live one, so one head
survives the desk). Agents write via `handoff_write` before
ending substantive work.
";

const PAGE_ALIAS: &str = "\
## alias: personal vocabulary

Defined in config (`kum config --open`), listed by
`kum config`:

```
[alias]
urgent = \"tasks --severity urgent\"
mins = \"export minutes --open\"
```

`kum urgent project/x` runs `kum tasks --severity urgent
project/x`. Three rules, all load-bearing (D-035):

1. INTERNAL-ONLY, never shell. An alias expands to kumbarium
   arguments and nothing else; there is no `!` form. Anything
   that can write files as you (a compromised agent included)
   can edit config.toml, so config decides VALUES, never
   executables: a poisoned alias can only invoke a kumbarium
   command the attacker could run directly, witnessed as
   itself.
2. BUILTINS ALWAYS WIN. A name that shadows a command or a
   reserved roadmap word is refused at config parse (warned,
   ignored), so the documented surface is unforgeable.
3. ONE EXPANSION. An alias never expands another alias.
";

const PAGE_DOCKET: &str = "\
## the docket: tasks and the roadmap

```
kum task <ns> <matter...> [--severity S] [--goal YYYY-MM-DD]
kum tasks [ns] [--all] [--severity S]
kum roadmap [ns]
kum task done <id> [note]    kum task drop <id> [note]
kum task grade <id> [--severity S] [--goal DATE] [note]
kum task history <id>
```

A task is a matter before the house: one self-contained
statement (detail belongs in memory), filed on a registered
shelf, carrying a severity (low | normal | high | urgent) and
an optional GOAL date. A goal is a target the library watches,
never an alarm: the timeline marks approaching goals and paints
passed ones red, and re-grading a goal is a supersession, so
every slip is on the chain (`task history` shows the creep).

`kum tasks` is the timeline: most-overdue first, then severity,
then age. `kum roadmap` pivots the same matters by derived
horizon: overdue / now (within a week) / next (within a month)
/ later / someday (no goal). Done and dropped keep the row (the
docket records judgments); agents file and update tasks over
MCP, and a quarantined writer's tasks wait at the desk like any
write: a task poisons what an agent DOES, so provenance
deserves a look before an urgent stranger jumps your queue.

Reserved first words after `kum task`: done, drop, grade,
history (a namespace with one of those names cannot be filed
to from the CLI; agents are unaffected).
";

const PAGE_EXPORT: &str = "\
## export: the loading dock

```
kum export                     list what can leave
kum export minutes [--raw]     audit minutes markdown
kum export bundle <namespace>  a shelf as one hashed JSON file
kum import bundle <FILE> [--pending]
```

Everything leaving the library goes through one verb with one
flag contract; imports enter through `kum import`, where the
approvals policy waits. Minutes have no import on purpose: the
ledger admits events only by witnessing them.

Shared flags on every exporter:

- `--out DIR`: export into DIR (created if missing, `~/` ok,
  trailing slash irrelevant) instead of the artifact's shelf
  under exports/. The sortable stamped name is not negotiable.
- `--stdout`: stream instead; nothing persisted.
- `--show`: reveal the file in the OS file explorer (macOS and
  Windows select it; Linux opens the containing shelf).
- `--open`: open the file in $VISUAL / $EDITOR (announced on a
  dim line first).

Minutes render local time by default; `--raw` keeps stored UTC
(machine-comparable across exporting machines).

One shelf as one deterministic JSON file, SHA-256 hashed so a
review conversation can name it and the importer can verify
nothing changed in transit (an altered bundle is refused).
Entries travel with full provenance, tags, notes, and chain
pointers; pending and rejected material never travels, and
CONFIDENCE never travels either: evidence is local, the
receiving janitor re-earns the number from its own ledger.

Import is a union-merge, idempotent by id (re-import no-ops;
the same id with different content is refused as tampering).
A chain the bundle extends fast-forwards locally. A FORK (both
libraries superseded the same entry differently) never
auto-resolves: the rival head lands pending with a
`contradicts` edge to the live local head, and you settle it
from the inbox (D-028). `--pending` routes every imported head
through the desk, for bundles from hands you do not know.

```
kum export bundle project/my-app --show
kum export bundle project/my-app --out ~/Desktop/files
kum export bundle project/my-app --stdout | jq .content_hash
kum export minutes --open
kum import bundle contributed.bundle.json --pending
```
";

const PAGE_APPROVALS: &str = "\
## approvals: the circulation desk

```
kum inbox                    pending entries, oldest first
kum review <id>              one pending entry, judged view
kum approve <id>             promote to circulation
kum reject <id> [reason]     decline, kept on record
```

Quarantine is a STATUS, not a place (D-027): a pending entry
keeps its target namespace but never surfaces in recall, list,
grep, or chain search until a human approves it. Who lands in
quarantine is write policy in config:

```
[approvals]
default_mode = \"live\"       or \"pending\" (teams, OSS)
pending_agents = \"a, b\"     always-quarantined writers
```

`kum review` shows content, provenance, and the COLLISION
SURFACE: live near-matches already shelved in the target scope,
so approval happens with eyes open. Approve and reject are
human-only and witnessed (the ledger records who judged, when,
seeing what); a rejected entry is kept as evidence of the
judgment, never deleted (`forget` is the separate tool for
wrong-or-sensitive content). Blame lands where judgment
happened: provenance shows who submitted, the approval event
shows who promoted.
";

const PAGE_JANITOR: &str = "\
## janitor: the confidence pass

```
kum janitor           preview proposed changes
kum janitor --apply   sign off and write them
```

The janitor is the only mover of the confidence number. It is
pure ledger math (D-025), no LLM: every live entry is
recomputed from the full audit log, so reruns are idempotent.

- survival is the backbone: distinct agent-day exposures via
  recall, never corrected (asymptote 0.80).
- confirms are garnish: weighted evidence, self-confirms
  discounted to a quarter (ceiling 0.95; nothing inside the
  library can prove a fact was applied, so nothing reaches 1.0).
- no exposure keeps the 0.50 neutral prior. Entries older than
  `janitor.dormant_days` (config, default 45) that were never
  recalled are listed as dormant: retire candidates for YOUR
  judgment; the janitor never retires.

Confidence informs recall output, it never ranks or filters it
(D-026). Applying writes each entry's new number plus a stored
basis line (shown by recall and `kum show`), and witnesses one
batch janitor event carrying the full change manifest.
";

const PAGE_GREP: &str = "\
## grep: literal search, rg-flavored

```
kum grep <pattern> [namespace] [--all]
```

NOT recall: recall is ranked, stemmed, live-only (the agent
surface); grep is literal, exhaustive forensics. Smart-case
like ripgrep (all-lowercase pattern matches insensitively; any
uppercase makes it exact). `--all` includes superseded and
retired versions: 'where did I EVER say X'. On a terminal:
grouped by entry with highlighted matches; piped:
`id:line:text` for scripts.

```
kum grep porter
kum grep TIOCGWINSZ project/kumbarium
kum grep 'old convention' --all
```
";

const PAGE_MOVE: &str = "\
## move: relocate a memory

```
kum move <id> <namespace>
```

Moves an entry to another namespace AS A SUPERSESSION with an
auto-note ('moved from project/x'): nothing mutates in place,
the move is history like everything else. Target namespace must
be registered. Moving one part of a split set moves that part
only.

```
kum move 8d331758 project/[redacted]
```
";
