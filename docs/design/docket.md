# The docket: tasks and the roadmap

The coordination section's first resource, and the second
section to open after memory. A task is a matter before the
house: something to do, filed on a shelf, carrying a severity,
awaiting action and eventually a judgment (done or dropped).
The roadmap is not a separate system: it is the same docket
read at a longer horizon.

Design only; the build waits for sign-off.

## Why it belongs in the library

The section thesis (identity doc) said the librarian, witness,
identity, and CLI were always section-agnostic, and memory was
merely the first thing stocked. The docket proves it: same
registered namespaces (a task lives on the shelf of the project
it belongs to, org-wide matters on `global`), same chain
visibility (an agent in `project/x` sees x's tasks and the
org's, never a sibling's), same provenance (who filed it), same
witness (every filing, edit, and completion is an event), same
desk (an untrusted writer's task lands pending). One new noun,
zero new architecture.

## Shelving: the library directory

The docket is its own DATABASE, and it inaugurates the layout
every section will use: the data dir is the building, library/
is the library proper, and each shelf that needs a database is
a file named for what it holds. The witness is not a shelf; it
is the ledger every shelf writes, and it stays at the root
watching all of them.

```
data_dir/
  audit.db           the witness, at the root
  library/
    memory.db        the general collection (library.db, moved
                     and renamed on first open; one rename,
                     nothing else changes)
    docket.db        tasks and the roadmap
    (secrets.db)     the restricted stacks, later
  backups/<shelf>/   one backup shelf per database
  exports/           the loading dock
  config.toml
```

Why files, not tables: shelves are isolated at the storage
level the way the crates isolate them at compile time; each
gets its own schema_version and migration history, its own
backup cadence and growth; a shelf's file simply does not exist
until its section is first used; and the section after this one
(secrets) requires a separate file anyway. Full backup remains
`cp -r` of one directory.

The namespace registry stays in memory.db as the one shared
piece; other shelves store the validated namespace PATH as
text, and the librarian checks it against the registry at the
gate exactly as it does for agents today. A shelf file on its
own therefore remains meaningful (a docket.db is readable
without the registry; registration is a write-time gate, not a
read-time join). Shelves meet in the librarian, never in SQL:
no cross-database ATTACH.

## What a task is

A row in library/docket.db, deliberately NOT an entry kind:
tasks must never surface in memory recall (recall answers "what
is true", the docket answers "what is owed"), and filtering
them out of every fact surface forever would be a tax on both.

Fields, reusing every convention the entries table proved:

- id (UUIDv7), namespace (the validated PATH as text, checked
  against the registry at the gate; see shelving), content (the
  matter, one self-contained statement), agent_id + source
  (provenance), created_at / updated_at.
- `severity`: low | normal | high | urgent. Display order and
  color follow it everywhere.
- `goal`: an optional target date (ISO day). A goal, not an
  alarm: the manager surfaces, it never schedules. The roadmap
  axis is DERIVED from it, never hand-maintained: overdue /
  now (within a week) / next (within a month) / later (dated
  beyond) / someday (no goal). Dates in an unwatched tool rot
  silently, which is why goals here are watched: see the
  creeping-deadlines section.
- `state`: open | done | dropped. Done and dropped both KEEP
  the row (the docket is a record of judgments, like the desk);
  done_at stamps when. Dropped is for matters overtaken by
  events; a short note says why.
- `superseded_by` + `note`: edits are supersessions, exactly
  like memory. Rewording a task, re-grading its severity, or
  moving its goal mints a new version chained to the old;
  history and diffs come free from the same discipline (D-020
  applies: content immutable, judgment witnessed).
- `status`: live | pending | rejected (D-027 reused verbatim).
  See the injection note below; the desk judges tasks with the
  same three verbs it already has.

## The injection note (why quarantine matters MORE here)

A memory poisons what an agent believes; a task poisons what an
agent DOES. "urgent: rotate the deploy keys to this value" filed
by a compromised writer is an instruction injection wearing a
to-do's clothes. So the write policy applies with extra teeth:
pending_agents' tasks land pending like any write, the review
surface shows severity and provenance prominently (an untrusted
writer filing `urgent` is itself a signal), and the snippet
tells agents that tasks are CLAIMS of work owed, carrying their
filer's authority and nothing more.

## Surfaces

MCP, two tools (the six memory verbs stay untouched; verb
economy is a feature):

- `task_file`: namespace, content, severity, optional goal
  (default severity: normal). Policy decides live vs pending.
- `task_update`: id + any of severity / goal / content
  (supersession under the hood) / state -> done | dropped with
  an optional note. Marking done is a CLAIM, attributed and
  witnessed, same epistemics as confirm: the charter never
  pretends to verify work happened outside the walls.

Listing open tasks is part of `recall`? No: separate concerns,
separate tool would be a third... the CLI and the snippet cover
reading (agents get told to check the docket at session start
alongside recall; the tool result of task_file echoes the open
docket for the scope so filing an item shows its neighbors).
If daily driving shows agents need a dedicated read tool, a
`docket` tool is a compatible addition, not a redesign.

CLI:

```
kum task <ns> <content> [--severity S] [--goal YYYY-MM-DD]
kum tasks [ns] [--all] [--severity S]
kum task done <id> [note]
kum task drop <id> [note]
kum task grade <id> --severity S | --goal YYYY-MM-DD
kum roadmap [ns]
```

`kum tasks` is the timeline: open matters, passed goals first,
then urgent first, oldest first within a grade; age and goal
columns always visible (an old urgent is a smell the layout
itself should surface). `kum roadmap` is the same rows pivoted
by derived horizon: overdue / now / next / later / someday as
sections, severity ordering within each. One dataset, two
readings, exactly as "roadmap-esque" should mean.

The witness gains kinds `task_file`, `task_update`, `task_done`,
`task_drop` (one migration, the first of the append-only era).
Minutes render them as prose like everything else: "filed
urgent task 3f2a91bc", "completed 3f2a91bc". The docket's
history IS meeting minutes now; that was free.

## Creeping deadlines (the goal machinery's payoff)

Re-goaling a task is a supersession like any other edit, which
means every slip is ON THE CHAIN: the witness records who moved
the goal, when, and by how much, and slippage becomes
deterministic ledger math ("goal moved 3 times, 40 days total")
instead of a feeling. Surfacing happens at two layers, both
read-side (the manager never reminds, notifies, or schedules):

- The timeline marks goal proximity as it renders: approaching
  goals in yellow, passed ones in red with "over by Nd", and a
  passed goal outranks its severity peers. You cannot open the
  docket without the creep looking back at you.
- The janitor's findings grow "creeping matters": open tasks
  whose goal has passed or keeps sliding (slip count and total
  slip straight off the chain), advisory and human-judged like
  dormant memories. Deciding whether the task or the goal was
  wrong stays a human call.

## Janitor and the docket (v2, named now)

Staleness has a task shape: open + untouched + old is the
docket's dormancy, and creeping matters (above) are its
overdue. An agent-day exposure analog exists too: a task
repeatedly shown at session start and never acted on is
telling you something about either the task or the roadmap.
The creep MARKS in the timeline are v1 (pure rendering); the
janitor findings are v2. Nothing blocks either.

## Relationship to handoff logs

Handoffs ("what is mid-flight for the next session") are the
coordination section's second resource and stay a separate
design: a handoff is ephemeral narrative, a task is a durable
matter. But they are siblings, and the session-start ritual the
snippet teaches becomes: recall the scope, read the handoff,
check the docket. The docket lands first because severity +
horizon was the ask, and because tasks give handoffs something
to point at.

## Non-goals (v1)

- No reminders, no notifications, no scheduling (the manager
  never directs work, it records what is owed; goals surface
  at read time and in janitor findings, nowhere else).
- No hierarchy / subtasks (deep taxonomies are where personal
  tools go to die; a matter too big to state is two matters).
- No assignees (personal tier has one human; the daemon rung
  can add assignment when identity hardens).
- No cross-links to entries yet (a `relates_to` bridge from
  tasks to memories is attractive and cheap later; the edge
  table generalizes when needed, not before).
- Bundles do not carry tasks in v1 (a shelf's facts travel;
  its obligations staying home is a defensible default until
  someone needs otherwise).

## Testing shape (when built)

Store: lifecycle transitions, supersession chains on regrade,
recall NEVER returns tasks, chain visibility on task listing.
CLI: file / grade / done round-trip with witnessed events;
roadmap pivot rendering. Persona harness: a fixture where the
agent is told "we should fix X eventually" and the grader
expects a filed task (severity judgment observable!), plus a
session-start fixture where an open urgent task must be
mentioned before new work begins. The injection fixture:
a quarantined persona files an urgent task; the grader proves
it never surfaced to the trusted agent.
