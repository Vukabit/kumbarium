# The process rung: presence, reload, update (D-048)

The library has always known what agents DID (the ledger) and
what they are DOING (the reading room). It has never known who
is simply THERE: which serve processes are alive, on which
binary, spawned by whom. This rung adds that awareness and the
two verbs it unlocks: a hot reload that swaps binaries under
live sessions, and a self-update that knows when a session is
still speaking the old one, plus the session card that makes
minted ids addressable. The doctor (its own design,
doctor.md) reads the same registry.

## The presence registry

Every serve process (and any long-running command) drops a
presence record at `library/procs/<pid>`: pid, process
start-time, binary version, the claimed agent id, the minted
session id, and the parent client's process name. The record
is removed on clean exit and updated after `initialize` claims
an identity.

Three liveness rules, one per failure shape:

- DEAD CLIENT: stdio transport is the strongest signal. When
  the client dies or closes the session, serve's stdin hits
  EOF, the loop ends, the process exits, the record removes
  itself. Zero new machinery; the transport is the heartbeat.
- CRASHED KUMBARIUM: a record is trusted only if its pid is
  alive AND the process start-time matches what the record
  stored. Pid reuse fails the second check, so a recycled pid
  can never impersonate a session. Stale records are debris;
  every reader skips them and the doctor sweeps them.
- WEDGED CLIENT: alive as a process, hung as a peer. The
  config-gated idle watchdog covers it: after
  `serve.idle_ping_minutes` of silence (default 0, disabled),
  serve sends a spec `ping` (MCP is JSON-RPC both ways; ping
  is explicitly bidirectional). No response after a retry and
  a generous timeout means the patron is a ghost: serve shuts
  down cleanly, removing its record; its leases lapse on TTL
  as they always would. Conservative by default, because some
  clients answer server pings lazily or not at all.

The registry lives under `library/`, so it is per-home: a
second `KUMBARIUM_HOME` is a different building with its own
register.

## kum processes: the occupancy listing

A bare browse noun (bare nouns show data, never usage walls),
beside `kum agents` (identities across history) and
`kum leases` (work claims). This is the third axis: live
incarnations, right now.

```
pid    version  agent        session   client       since (local)     activity
4132   0.3.0    claude-code  9f21ab04  Claude Code  2026-09-05 14:02  2m ago
5901   0.2.1    gemini       77c0de19  gemini       2026-09-05 09:47  3h ago
```

- Every row shown is liveness-verified (pid + start-time); the
  listing never renders a ghost.
- VERSION is the load-bearing column: a row whose version
  predates the CLI's own paints yellow with the remedy inline
  (`kum serve reload <pid>`). After an update, "which sessions
  are still on the old binary" answers itself.
- ACTIVITY derives from the ledger by session id (the ledger
  is already the heartbeat; presence records are not touched
  per tool call). Same `active 2m ago` grammar as leases.
- SESSION is the 8-char short, pasteable into `kum show` and
  `kum dossier --session`.
- `--json` like every browse surface; one `kum status` line
  (`processes: 2 live (1 on an older binary)`); voiced empty
  state. `processes` joins the reserved namespace words.
- No verbs under it, by design: reload belongs to `serve`,
  killing belongs to the OS. There is deliberately NO
  kill-switch verb: the client owns its serve child (and
  respawns it), pid sweeps are how innocent processes die,
  and killing writers mid-transaction manufactures the
  corruption the doctor exists to find. The librarian governs
  the collection, never the patrons; where termination is
  truly wanted, the listing hands over the pid and `kill` is
  the OS's verb, one paste away.

## kum serve reload: the hot restart

After an update, live sessions keep speaking the old binary
until their clients restart, unless the serve process swaps
itself. `exec()` makes that a true hot restart: file
descriptors survive exec, so the process keeps its pid and its
stdio pipes, and the client never notices which incarnation is
answering.

```
kum serve reload [pid]     signal live serves to re-exec
```

Runnable by the human or by an agent through its shell tool.
Mechanics, in order:

1. Read the presence registry; pick live serves (or the one
   named pid).
2. Signal each (SIGUSR1). The serve process finishes the
   request in flight, serializes its small session state (the
   claimed agent, the minted session id, the negotiated
   protocol version, the served-handoffs set so the opening
   frame does not replay), and re-execs the kumbarium binary
   BY ITS INSTALL PATH (resolving the path fresh picks up the
   replaced file); `/proc/self/exe` would re-run the old image.
3. The reborn process restores the state and sends
   `notifications/tools/list_changed`, so the client refetches
   the tool list and sees new tools mid-conversation.

The SESSION ID CARRIES ACROSS the exec: the session models the
conversation, not the binary incarnation (D-044 unchanged), so
leases keep renewing and attribution stays continuous. The
re-exec is announced on stderr and the presence record's
version field updates, which is also how `kum processes` shows
the reload took.

Windows is a stated divergence: no exec(), and proxying live
pipes across a process swap is corruption-shaped complexity.
There the command reports honestly: hot reload is unavailable;
restart the client session.

## kum update: the one networked verb

The librarian never phones home. No background version checks,
no nag lines in other commands: the network happens only
inside an explicit `kum update`, and that stance is the
decision, not the mechanics.

```
kum update            check; if newer, show the changelog
                      section and prompt [y/N]
kum update --check    report only; exit 1 if an update exists
kum update --yes      the script form
```

- CHANNEL OWNERSHIP first: a binary under `~/.cargo/bin`
  belongs to cargo, a Homebrew Cellar path to brew. The
  command defers to the owner (`cargo install kumbarium
  --force`, `brew upgrade`) and self-replaces ONLY the
  standalone-tarball install. Fighting a package manager is
  how updaters lose.
- VERIFY BEFORE REPLACE: download the platform tarball, check
  it against the release's published SHA-256 sums
  (release.yml grows a checksums asset as a prerequisite).
- ATOMIC SWAP, BOTH BINARIES: `kum` and `kumbarium` replace
  together or not at all, via the rename-aside dance (required
  on Windows anyway), old binaries kept one generation as
  `.bak`.
- FORWARD-ONLY HONESTY: sections migrate forward on first
  open and do not migrate back; the prompt says so and points
  at `kum backup list`.
- The success footer closes the loop:
  `2 live sessions on the old binary: kum serve reload`.
- Not witnessed: the ledger records what happens to the
  collection; toolchain management happens outside the
  library and never opens it.

## The session card: kum show <session-fragment>

Minted session ids print everywhere (leases, the dossier, the
recall frame, `kum processes`) but until now only
`kum dossier <agent> --session` accepted one, and it demands
the agent name you may not have. Building-wide ids are the
point, so sessions join `kum show`'s fall-through chain as the
FIFTH and last resolver: entries, tasks, briefings, secrets,
then distinct session ids from the ledger (plus the leases
table and the presence registry, for sessions too young to
have witnessed anything).

The card is a rendering (nothing written, not witnessed): the
claiming agent, alive-or-not with the pid, first and last
activity, event counts by kind, scopes touched, writes and how
they fared, leases held, what its first recall was served, and
the pointer to the full story
(`kum dossier <agent> --session <short>`). A zero-event
session degrades honestly to "alive, nothing witnessed yet".
The show error's teaching line grows one word: entry, task,
handoff, secret, or session.

## What this rung deliberately does not do

- No kill-switch (above). No `--reverse` anywhere: real
  reversibility is snapshots plus the documented hand restore,
  not an undo flag that implies the repairs were risky.
- No daemon: presence records describe processes clients
  spawn; the daemon rung, when it comes, drops a record on the
  same registry and appears in the same listing with zero new
  surface.
- No automatic anything networked. `--check` exists so a
  human's cron can ask; kumbarium itself never asks.
