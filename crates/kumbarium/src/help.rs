//! Per-topic manual pages: syntax, the input grammar each
//! command speaks, and worked examples. Authored as markdown so
//! the body renders through the same highlighter memory bodies
//! use; plain when piped, colored on a terminal.

pub const TOPICS: &str = "list show history revert retire \
import namespace audit backup serve ids namespaces \
instructions";

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
    "namespaces" | "scopes" => PAGE_NAMESPACES,
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
- `agent/<id>/quarantine`: an untrusted writer's pen

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
kum audit tail [n]
kum audit export [--stdout] [--raw]
```

Every librarian transaction is an event: who (agent identity),
when, what, in which scope. `tail` shows the most recent n
(default 20) as prose. `export` renders deterministic
meeting-minutes markdown to exports/ and prints the path
(quoted on a terminal, bare into pipes); `--stdout` streams the
markdown instead, for piping. Times render local by default;
`--raw` keeps the stored UTC form (machine-comparable across
exporting machines). Storage is always strict ISO-8601 UTC.

```
kum audit tail 50
kum audit export --stdout | less
command cat \"$(kum audit export)\"
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
supersede, forget. Every call is audited under the identity the
client declared at initialize. stdout carries protocol only;
diagnostics go to stderr.
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
- When a recalled fact proves stale or wrong, `supersede` it
  (add a short `note` like 'typo fix' for trivial changes);
  `forget` only wrong-or-sensitive content.
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
