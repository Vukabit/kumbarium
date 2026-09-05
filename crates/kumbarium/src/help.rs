//! Per-topic manual pages: syntax, the input grammar each
//! command speaks, and worked examples. Authored as markdown so
//! the body renders through the same highlighter memory bodies
//! use; plain when piped, colored on a terminal.

/// Topic names for `kum help <topic>`, in the same order as
/// MANUAL_ORDER: the list a user scans and the manual they
/// read tell one story.
pub const TOPICS: &str = "instructions serve environment ids \
namespaces namespace list show grep history revert retire \
move approvals docket handoff leases secrets brief agents \
dossier processes janitor audit export import status backup \
update doctor alias conventions";

/// The whole manual, in READING order: the building tour, not
/// the accident of declaration order. Setup first, then the
/// grammars, the collection, the lifecycle, the desk, the
/// sections as they open, the two renderings, the witness,
/// upkeep. `kum help --all` renders these in sequence; the
/// doc-drift test keeps this list and `page()` in lockstep.
pub const MANUAL_ORDER: &[&str] = &[
  "instructions",
  "serve",
  "environment",
  "ids",
  "namespaces",
  "namespace",
  "list",
  "show",
  "grep",
  "history",
  "revert",
  "retire",
  "move",
  "approvals",
  "docket",
  "handoff",
  "leases",
  "secrets",
  "brief",
  "agents",
  "dossier",
  "processes",
  "janitor",
  "audit",
  "export",
  "import",
  "status",
  "backup",
  "update",
  "doctor",
  "alias",
  "conventions",
];

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
    "secret" | "secrets" => PAGE_SECRETS,
    "brief" | "binder" => PAGE_BRIEF,
    "dossier" => PAGE_DOSSIER,
    "lease" | "leases" => PAGE_LEASES,
    "agent" | "agents" | "roster" => PAGE_AGENTS,
    "environment" | "env" => PAGE_ENVIRONMENT,
    "processes" | "process" | "procs" => PAGE_PROCESSES,
    "conventions" | "stances" | "completions" | "json" => PAGE_CONVENTIONS,
    "update" => PAGE_UPDATE,
    "doctor" | "repair" => PAGE_DOCTOR,
    _ => return None,
  })
}

const PAGE_DOCTOR: &str = "\
## doctor: the mechanic

```
kum doctor            examine; report; repair nothing
kum doctor --deep     the expensive tier (integrity_check,
                      backup coverage)
kum doctor --apply    perform the safe repairs, witnessed
kum doctor --json     machine findings
```

The janitor judges FACTS (confidence, circulation); the doctor
judges the BUILDING (files, schemas, invariants, integrity).
Neither does the other's job.

Checks, grouped by section: database integrity (a read-only
`quick_check`, `integrity_check` under `--deep`); the audit
chain (intact, or the first break named); referential drift
(matters and briefings on namespaces the registry no longer
knows); debris (interrupted-backup temp files, stale presence
records from dead processes); the keystore (a present-but-
failing one is the downgrade-attack shape); and, under
`--deep`, backup coverage.

Preview by default, `--apply` for repair, the same idiom as
janitor and revert. But `--apply` is PREEN-CLASS ONLY: it
sweeps debris, re-chains an unhashed ledger tail (pure
recomputation), and nothing more. It NEVER repairs evidence: a
hash mismatch, content divergence, or a failed integrity check
is reported with its remedy (restore a snapshot, investigate),
never rewritten, because repairing evidence destroys it. There
is no `--reverse`: `--apply` snapshots every section first, so
reversal is a real restore (`kum backup list`), stronger than
an undo flag.

Preview is lock-free (a read snapshot, safe beside live
sessions). `--apply` takes the maintenance lock and DEFERS
file surgery while any serve process is live, naming the pids
to close. One `doctor` event is witnessed per apply, carrying
the repair count; preview witnesses nothing. Exit 0 healthy,
1 on any finding (the uniform stance; branch on `--json`).
";

const PAGE_UPDATE: &str = "\
## update: the one networked verb

```
kum update            check; if newer, show what changes,
                      prompt, then swap
kum update --check    report only; exit 1 if an update exists
kum update --yes      no prompt (for scripts)
```

The librarian never phones home: no background checks, no
version nags in other commands. The network happens ONLY
inside this command, and it reaches the network through
`curl` (the tooling door, the way sqlite3 is storage's), so a
build carries no HTTP stack for a feature most installs never
run.

Package managers own their installs: a cargo or Homebrew
kumbarium is told the right upgrade command
(`cargo install kumbarium --force`, `brew upgrade kumbarium`)
and nothing is swapped. Only a standalone-tarball install
self-replaces, and only after the download's SHA-256 matches
the release's published sum (no checksum to check is a failure,
never a silent pass). Both binaries (`kum` and `kumbarium`)
swap together, the old kept one generation as `.bak`.

Migrations run forward only: after an update, sections do not
migrate back, so downgrading is unsupported; snapshots are the
way back (`kum backup list`). Live serve sessions keep the old
binary until `kum serve reload` swaps them (`kum processes`
shows who is still on it).
";

const PAGE_PROCESSES: &str = "\
## processes: the occupancy listing

```
kum processes [--json]
```

The third axis beside the roster and the reading room:
`kum agents` is identities across history, `kum leases` is
work claims, this is live incarnations, right now. Each serve
process registers a presence record under `library/procs/`,
held trustworthy by an OS file lock that dies with the
process: a row shown here is liveness-verified, never a pid a
recycled number could fool.

Columns: pid, binary version, the claimed agent, the minted
session (8-char short, pasteable into `kum dossier
--session`), the client that spawned it, since, and last
witnessed activity (the ledger is the heartbeat). A row on an
OLDER binary than this CLI paints yellow with the remedy
inline: `kum serve reload <pid>` swaps the binary under the
live session without restarting the client.

Deliberately absent: a kill verb. Clients own their serve
children (and respawn them), and killing writers
mid-transaction manufactures corruption; where termination is
truly wanted, the pid is right there and `kill` is the OS's
verb. The registry is per-home: a second KUMBARIUM_HOME is a
different building with its own register.
";

const PAGE_CONVENTIONS: &str = "\
## conventions: deliberate stances

Where kumbarium's CLI diverges from the conventions of the
tools its command surface takes after (git, cargo, gh,
docker), the divergence is a decision, never an accident:

- NO AUTO-PAGING. Output streams; SIGPIPE is handled cleanly,
  so `kum list | less` composes exactly as the shell intends,
  and scripts never fight a surprise pager.
- UNIFORM EXIT CODES. Success is 0, every failure is 1.
  Scripts branch on output and the ledger holds the detail;
  a taxonomy of exit codes would be precision nobody reads.
- FLAT VERBS + NOUN FAMILIES. The everyday verbs are flat
  (list, show, grep, forget); each section arrives as a noun
  family (secret, task, namespace, lease). Bare nouns BROWSE
  (`kum tasks`, `kum secrets`, `kum handoffs` show data, never
  usage walls); the grammar lives in `kum help <topic>`.
- MACHINE OUTPUT is `--json` on the browse surfaces: list,
  status, tasks, agents, secrets, leases. Values never appear
  in any of them; secrets emit metadata only, structurally.
- COMPLETIONS: `kum completions bash|zsh|fish` prints the
  script (static, never opens the library); `--install` writes
  it to the shell's conventional path, and the plain form
  prints where that is. Completes the command word, from the
  same list as the did-you-mean hints.
- ONE DOOR FOR AGENTS. Agents speak MCP and nothing else. The
  SQLite files are a TOOLING door: yours to open (sqlite3,
  backups, forensics), never a channel a governed agent may
  use. Nothing that bypasses the librarian is witnessed, and
  an unwitnessed write is what tampering looks like; the
  janitor's unwitnessed-grant finding exists for exactly that
  shape.
";

const PAGE_ENVIRONMENT: &str = "\
## environment: the variables the librarian honors

- `KUMBARIUM_HOME`: where the library lives. Set, it holds
  everything (data, config, backups, exports) under one
  directory: for test harnesses, portable installs, throwaway
  libraries. Unset, the platform's standard data and config
  directories apply.
- `NO_COLOR` (any value), `CLICOLOR=0`, or `TERM=dumb`:
  suppress color.
- `CLICOLOR_FORCE=1`: force color even into a pipe (scripts
  that relay to a terminal). Force beats suppression.
- `$VISUAL`, then `$EDITOR`: what `--open` launches on
  exports.

Without any of these set, color follows the terminal: on when
stdout is a real terminal, plain when piped. The MCP server
(`kum serve`) never colors its protocol stream.
";

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

A namespace is a shelf of the library; SCOPE names the same
thing from recall's side (the namespace a search starts from).
Slash paths, 1-3 segments of `[a-z0-9._-]`, registered by you
(agents cannot create them):

- `global`: cross-project facts
- `project/<name>`: one project's facts

Quarantine is NOT a namespace: an untrusted writer's entries
land in their target namespace with pending status instead
(see `kum help approvals`).

Recall searches a scope's CHAIN: itself, its ancestors, then
`global`; never a sibling. `project/web.app` searches
`project/web.app`, `project`, `global`.
";

const PAGE_LIST: &str = "\
## list: browse entries

```
kum list [namespace] [--all]
```

An ENTRY is one stored fact (the word memory names the same
row from the agent's side). Newest first: short id, created
day, kind, namespace, first content line. Default hides
superseded and retired entries; `--all` shows them with
`[superseded]` / `[retired]` markers.
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

Links are drawn by agents (the MCP link tool) or by you:
`kum link <from> <rel> <to>` with rel one of continues,
relates_to, duplicates, contradicts. Typed edges are curation:
`contradicts` flags a fact for human review, and link votes
feed the janitor's confidence math.

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
kum list project/my-app --all   # shows [retired]
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
kum namespace describe <path> <description>
kum namespace list
```

Namespaces are registered-only: agents can never create one,
which is the firewall against taxonomy drift. Path grammar in
`kum help namespaces`. The description is the shelf's charter
line (the binder opens with it); `describe` rewrites it in
place, so a typo'd charter is not forever.

```
kum namespace add project/my-app \"the new thing\"
kum namespace describe project/my-app \"the shipped thing\"
kum namespace list
```
";

const PAGE_AUDIT: &str = "\
## audit: the witness

```
kum audit tail [n] [--scope <ns>]
kum audit follow [n] [--scope <ns>]
kum audit verify
```

Minutes leave through the loading dock: `kum export minutes`
(see `kum help export`).

Every librarian transaction is an event: who (agent identity),
when, what, in which scope. `tail` shows the most recent n
(default 20) as prose. `follow` prints a short backlog and then
STREAMS new events as they are witnessed, oldest-first, until
Ctrl-C: the real-time view of what agents are doing right now
(what `watch` cannot give, since a growing log is not a re-run
command). Storage is always strict ISO-8601 UTC; rendering
localizes.

The ledger is HASH-CHAINED: each event stores
sha256(previous hash + its own fields, the minted session id
included), so `verify` recomputes the whole chain and
either confirms it intact (event count + head hash) or names
the first broken link. Tamper-evidence is
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
kum backup          snapshot every section now
kum backup list     every section's snapshots, newest first
```

Forces a snapshot of every SECTION that exists (memory, audit,
docket, handoff, secrets, leases; each to its own backups/
subdirectory): VACUUM INTO a temp file, integrity-checked,
atomically renamed in. Secrets back up as ciphertext; the
master key is never in any snapshot. Normally you never run
this: every `serve` startup snapshots automatically when 12h
have elapsed. Retention is tiered (2 recent + 7 dailies + 4
weeklies) and computed from the flat timestamped filenames;
unrecognized files are never touched.

RESTORE is a hand move, by design: stop every kumbarium
process, copy the chosen snapshot over the section's file
under library/ (`kum paths` names it), and start again. No
restore verb exists because a command that silently swaps a
section out from under live connections would be a corruption
machine; the deliberate copy is the safety interlock.
";

const PAGE_SERVE: &str = "\
## serve: the MCP server

```
kum serve
```

Speaks MCP over stdio: newline-delimited JSON-RPC 2.0. Not for
humans; agents' clients spawn it (the setup commands live in
`kum help instructions`). Tools: remember, link, recall, get,
task_list, confirm, supersede, task_file, task_update,
handoff_write, lease_take, lease_release, secret_read
(deletion is the human's verb: kum forget). Every
call is audited under the agent identity the client declared
at initialize, alongside a librarian-MINTED session id:
agents are claimed, sessions are minted. stdout
carries protocol only; diagnostics go to stderr.

`kum serve reload [pid|--all]` hot-swaps live serve processes
onto the current binary: the process re-execs in place (same
pid, same pipes, session carried over), then tells its client
the tool list changed. A pid reloads one; `--all` (or the bare
form) reloads every live session. Run it after an update
instead of restarting client sessions; `kum processes` shows
who needs it. Unix only; on Windows restart the client session. The
`serve.idle_ping_minutes` config (default 0, off) makes an
idle serve ping its client and exit cleanly if nobody
answers: the wedged-client watchdog.
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
- When two facts belong together (or clash), `link` them:
  relate or contradict facts you did not write. A
  `contradicts` edge flags the dispute for human review, and
  cross-agent links are evidence the confidence math counts.
- When a fact proves stale or wrong (a user correction
  counts), update it with `supersede`, never with a fresh
  `remember`: first `recall` the stale entry, then supersede
  the id it returned (add a short `note` like 'typo fix' for
  trivial changes). Deletion is the human's verb: for
  wrong-or-sensitive content, `link` it `contradicts` and ask
  them to run `kumbarium forget`.
- Before ENDING substantive work, leave the briefing with
  `handoff_write`: what is mid-flight, decided-but-unfinished,
  sharp edges. The next session receives it automatically with
  its first recall; write it for them.
- The DOCKET is the shared task list. Urgent or overdue
  matters arrive automatically with your first recall in a
  scope: mention them before starting new work. File matters
  worth doing later with `task_file` (severity is your
  judgment; add a goal date only when one is real). Shelving
  follows the same rule as facts: a project's tasks go on the
  project's namespace, `global` only for genuinely
  cross-project matters. `task_update` marks them done when
  the work is complete, or regrades severity and goal as
  reality moves.
- Starting substantive work on a distinct area? Take a
  reading-room lease (`lease_take`: scope + a short resource
  label). If the room warns that another agent holds it,
  coordinate instead of colliding. Leases lapse on idle;
  releasing is a courtesy.
- Credential VALUES never belong in memories, tasks, or
  briefings. Need one? Call `secret_read`; if refused, ask the
  human to run the `kumbarium secret grant` command the
  refusal names. Never store what it returns.
- Namespaces are registered by the user (`kumbarium namespace
  add`); never assume one exists, ask instead.
";

const PAGE_INSTRUCTIONS: &str = "\
## instructions: wiring agents up

```
kum instructions             this page
kum instructions --snippet   just the block, for appending
```

Three steps: register the MCP server, add the standing
instruction block so the agent uses it unprompted, and
register the namespaces agents will write to.

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

**3. Register the namespaces** (agents can never create one)

```
kum namespace add global \"cross-project facts\"
kum namespace add project/<name> \"what this project is\"
```

Then verify the wiring: after the agent's first `remember`,
`kum status` shows the entry under its namespace and
`kum agents` shows the identity on the roster.
";

const PAGE_STATUS: &str = "\
## status: library health at a glance

```
kum status
```

Entry counts (live / superseded / retired), split sets, the
desk's pending queue, docket and secrets counts, active
reading-room leases, live entries per namespace, audit event
count and latest, backup ages per section, database sizes. The
first command to run when wondering what state things are
in.
";

const PAGE_HANDOFF: &str = "\
## handoffs: the standing briefings

```
kum handoff <ns> <note...>    leave the briefing (supersedes)
kum handoff <ns>              read the standing briefing
kum handoff drop <ns>         take it out of circulation
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

`drop` retires a shelf's standing briefing without replacing
it: the row stays in the diary (witnessed, addressable), but a
dead project stops serving its stale briefing to every future
session. Human-only, like all deletion-shaped verbs.

Similarly named, different thing: `kum brief <ns>` is the
day-one binder, a page rendered FOR humans that includes the
standing briefing (see `kum help brief`).
";

const PAGE_SECRETS: &str = "\
## secrets: the restricted stacks

```
kum secret set <ns> <name>      stock or rotate; value from
     [--i-accept-plaintext]     stdin or an echo-off prompt,
     [--expires DATE]           never argv
kum secret read <ns> <name>     print the value
kum secret copy <ns> <name>     concealed clipboard copy,
                                auto-clear in 90 seconds
kum secret grant <ns> <name> <agent> [--until DATE]
                                allow secret_read (leased)
kum secret revoke <ns> <name> <agent>   withdraw, effective now
kum secret shred <ns> <name>    destroy the value, keep record
kum secret exec <ns> <name> [--as VAR] -- cmd args...
                                run with the value injected
kum secret leakscan [ns]        sweep shelves for exposures
kum secrets [ns]                names + grants, never values
```

Day one, three lines:

```
kum secret set global my-token           # prompts, echo off
kum secret grant global my-token claude
kum secret exec global my-token -- ./deploy.sh
```

These are the custody tools of the BROKER, the arm of the
library that holds credential values so that memories, tasks,
and briefings never have to. `exec` puts the value in the COMMAND'S
environment (never argv, never your scrollback, never a model
context) and streams the command's output back through a
redactor: a failing curl that echoes its token prints
`[kumbarium:redacted ...]` instead. The variable name derives
from the secret's name (`crates-io-token` injects as
`CRATES_IO_TOKEN`; override with `--as VAR`), and the exit
code passes through. `leakscan` is the other half, detection:
it unseals every live secret in-process and sweeps memories,
tasks, briefings, and ledger details for the bytes, reporting
row ids only, never content. Exit 1 on any exposure, so it
can gate.

Two expiries, deliberately different. A GRANT LEASE
(`--until DATE`) is enforced: every read re-checks, so the
lease ends at read time with nothing cached to outlive it, and
the grant dies at the end of that day (UTC). VALUE EXPIRY
(`--expires DATE`) is metadata: the credential expires
UPSTREAM, the broker records and surfaces the date (the
listing marks EXPIRED), and never blocks a read. Setting it
files a rotation matter on the docket automatically (one per
secret; a moved expiry re-grades its goal), and the docket's
goal-watching does the reminding. Completing the matter stays
your call: the broker never closes it.

Witnessed access is the product: every checkout, refusal, and
grant lands on the hash-chained ledger, so \"who has read the
deploy key this month\" is a query. Values are sealed at rest
(XChaCha20-Poly1305; the master key lives in the platform
keystore, never in any file or backup) and structurally absent
from every listing, export, and audit event.

Writes are human-only: agents cannot stock or grant, only ask.
An agent's `secret_read` is deny-by-default; the refusal names
the grant command so you can decide. Revocation is instant
because every read re-checks.

Rotation (`set` on an existing name) supersedes like memory,
but the retired value is SHREDDED: the history keeps who
rotated and when, never the old credential. `kum history <id>`
on a secret renders that chain. Secrets ride no recall, no
briefing, no bundle, ever.

Where no OS keystore exists, the first `set` refuses unless
told `--i-accept-plaintext`: a loud, sticky, standing choice.
A PRESENT-but-failing keystore refuses outright, because a
suppressed keystore is what a downgrade attack looks like.
";

const PAGE_LEASES: &str = "\
## leases: the reading room

```
kum leases [ns]         the register: active + stale cards
kum lease break <id>    clear a stuck card (witnessed)
```

The coordination section's third resource: what agents are
DOING, right now. An agent takes a lease (`lease_take`:
namespace + a short resource label) when it starts substantive
work on an area, and the room rides the FIRST recall any other
session makes in that scope, so occupancy is learned without a
tool to forget.

The stances, briefly: a
collision WARNS, never blocks (identity is self-reported at
this tier, so blocking would be theater, and a crashed agent
must never padlock the library); a lease lives
`leases.ttl_minutes` (config, default 120) past its holder's
last witnessed activity, so the ledger is the heartbeat and
releasing is a courtesy; expiry is computed at read time,
never stored or fired. Expired-but-unreleased cards (the
crashed-agent shape) show under `kum leases` and in the
janitor's findings; `kum lease break` clears one, witnessed
with the holder named.

A holder is (agent, SESSION): the librarian mints a session id
per serve process, so two sessions of the same agent
name warn each other ([ANOTHER SESSION OF YOU]) instead of
silently sharing one card. Minted ids disambiguate; they do
not authenticate.
";

const PAGE_BRIEF: &str = "\
## brief: the day-one binder

```
kum brief <ns>
```

One page before touching anything: the shelf's charter (its
registered description), the top standing facts ranked by what
survived circulation (confidence first, recency as tiebreak,
ten shown), the standing briefing the last session left, the
open matters that will not wait (urgency, then nearest goal,
eight shown), and what the restricted stacks hold in scope
(names and expiry only, structurally never values).

The binder is a RENDERING, not a record: every ingredient
already lives on a shelf, nothing is written, and reading it
is not witnessed (browsing is not circulation; recall is).
Agents keep their own channel: the first recall in a scope
already carries the briefing and urgent matters (see
`kum help handoff`); this page is the same state of the world
shaped for a person, or for pasting into a fresh context.
";

const PAGE_AGENTS: &str = "\
## agents: the roster

```
kum agents [--all]      every witnessed identity, at a glance
kum agent <name>        the deep story (alias for dossier)
```

Cleanup is CURATION, never erasure: the ledger keeps every
identity's history forever, so a stale identity is RETIRED in
config, not deleted:

```
[agents]
retired = \"demo, smoke, session-one\"
```

Retired identities leave the default roster and return under
`--all`, marked. Reversible by editing config; the dossier and
the ledger never hide anyone.

Every identity that ever touched the library, derived from the
ledger and the shelves: last seen, minted sessions, event
count, the estate (live writes, and how many were corrected by
OTHERS), grants held, active reading-room leases. Identities
seen only in imported entries show as pre-ledger.

Counts, never scores: the roster states what happened and
holds no opinion; ranking agents by number is precisely the
trap it refuses to build. Judgment stays yours, and
`kum dossier <agent>` is the evidence behind any row.
";

const PAGE_DOSSIER: &str = "\
## dossier: one agent's witnessed story

```
kum dossier <agent> [--since YYYY-MM-DD] [--until YYYY-MM-DD]
                    [--session <fragment>]
```

The deterministic postmortem, and the binder's sibling on the
other axis: the binder reads a SCOPE, the dossier reads an
AGENT. From the ledger and the shelves it renders what the
agent was served (recalls, briefings, matters), what it wrote
and how those writes fared (live, pending, rejected, revised
by itself, corrected by OTHERS: the survival fact), what the
desk judged, every credential it read or was REFUSED, and the
chronological record itself.

The hash chain is verified first and the verdict printed at
the top: a dossier states its own trustworthiness before
stating anything else. Events carry the librarian-MINTED
session id, hashed like every field, so `--session`
narrows the story to one incarnation of the agent and the
attribution cannot be quietly reassigned; the page lists the
sessions it saw. The estate figures deliberately outlive
the window (a write from last month corrected yesterday is
exactly what a postmortem wants to see); the record respects
it.

Like the binder it is a rendering, not a record: nothing
written, not witnessed. This is the seed of the compliance
packet: who acted, under what authority, with the math to
prove nobody edited the story afterward.
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
project/x`. Three rules, all load-bearing:

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
kum task reword <id> <matter...>
kum task history <id>
```

A task is a matter before the house: one self-contained
statement (detail belongs in memory), filed on a registered
shelf, carrying a severity (low | normal | high | urgent) and
an optional GOAL date. A goal is a target the library watches,
never an alarm: the timeline marks approaching goals and paints
passed ones red, and re-grading a goal is a supersession, so
every slip is on the chain (`task history` shows the CREEP:
each date the goal moved later).

`kum tasks` is the timeline: most-overdue first, then severity,
then age. `kum roadmap` pivots the same matters by derived
horizon: overdue / now (within a week) / next (within a month)
/ later / someday (no goal). Done and dropped keep the row (the
docket records judgments); agents file and update tasks over
MCP, and a quarantined writer's tasks wait at the desk like any
write: a task poisons what an agent DOES, so provenance
deserves a look before an urgent stranger jumps your queue.

`reword` restates a matter's wording as a supersession (the
clumsy first phrasing chains forward like a regrade), so
fixing a typo never means drop-and-refile.

Reserved first words after `kum task`: done, drop, grade,
reword, history (a namespace with one of those names cannot be
filed to from the CLI; agents are unaffected).
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
  trailing slash irrelevant) instead of the artifact's home
  under exports/. The sortable stamped name is not negotiable.
- `--stdout`: stream instead; nothing persisted.
- `--show`: reveal the file in the OS file explorer (macOS and
  Windows select it; Linux opens the containing folder).
- `--open`: open the file in $VISUAL / $EDITOR (announced on a
  dim line first).

Minutes render local time by default; `--raw` keeps stored UTC
(machine-comparable across exporting machines).

One shelf as one deterministic JSON file, SHA-256 hashed so a
review conversation can name it and the importer can verify
nothing changed in transit (an altered bundle is refused).
The FULL shelf travels, as typed sections: entries and their
edges, the docket's matters, the diary's briefings. Everything
carries full provenance and chain pointers; pending and
rejected material never travels, and CONFIDENCE never travels
either: evidence is local, the receiving janitor re-earns the
number from its own ledger.

Import is a union-merge, idempotent by id (re-import no-ops;
the same id with different content is refused as tampering).
A memory chain the bundle extends fast-forwards locally. A
FORK (both libraries superseded the same entry differently)
never auto-resolves: the rival head lands pending with a
`contradicts` edge to the live local head, and you settle it
from the inbox. Matters and briefings union by id, and a
visiting standing briefing never displaces yours unjudged: it
parks at the desk, where approval supersedes (one head
survives either way). `--pending` routes every imported head
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

Quarantine is a STATUS, not a place: a pending entry
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
## janitor: the confidence pass and the watchdog

```
kum janitor           preview proposed changes + findings
kum janitor --apply   sign off and write the changes
```

The janitor is the only mover of the confidence number. It is
pure ledger math, no LLM: every live entry is recomputed from
the full audit log, so reruns are idempotent.

- SURVIVAL is the backbone: every day a DIFFERENT agent was
  served a fact and did not correct it counts in the fact's
  favor. Survival alone can carry a fact to 0.80.
- Confirms add a little on top; confirming your own write
  counts a quarter of confirming someone else's.
- Links vote for the entry they point at: another agent's
  link counts in full, linking to your own entry a tenth.
  Nothing reaches 1.0 (the ceiling is 0.95): nothing inside
  the library can prove a fact was applied in the world.
- An entry never served keeps the neutral 0.50 it started
  with. Never-recalled entries older than their KIND'S window
  go DORMANT: retire candidates for YOUR judgment, the
  janitor never retires. The window scales from
  `janitor.dormant_days` (default 45): project_state ages out
  at half of it, decisions at 1x, references 2x, preferences
  4x.

The watchdog findings, advisory and write-free:

- UNWITNESSED GRANTS (first, in red): a secrets grant with no
  ledger event arrived around the librarian; treat as
  tampering until explained. Revoke (witnessed), then rotate.
- expired credentials still stocked: rotation owed.
- abandoned reading-room leases: expired, never released (the
  crashed-agent shape); kum lease break clears one.
- creeping matters: a goal that moved later twice or more.
- served-then-corrected: a fact superseded within 48h of a
  DIFFERENT agent's recall that served it (same-agent is the
  correction ritual); circulation misfired there.

Confidence informs recall output, it never ranks or filters
it. Applying writes each entry's new number plus a stored
basis line (shown by recall and `kum show`), and witnesses one
batch janitor event carrying the full change manifest and the
finding counts.
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
kum move 8d331758 project/other-app
```
";
