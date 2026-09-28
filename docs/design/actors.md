# Actors: minted identity and the actor's trail (D-056)

Status: designed and built 2026-09-28; review answers folded in
the same day (the human actor, grants, cross-library scope
deferred). One change in the build: `kum agent add` registers a
name only (no `--workspace`); an auto-mint key needs the client's
claimed name too, and guessing it would be wrong. Pinning goes
through `--agent`, and adopting an auto-minted actor is a rename.

## The gap

Identity has two legs today. The agent NAME is claimed at
initialize from `clientInfo.name` (D-038's honesty: self-reported),
and the SESSION is minted per serve process (D-044), stamped on
every ledger event and hashed into the chain (D-045).

Neither leg is an actor. Every Claude Code session in every
repository claims the same name, so the roster reads:

```
agent          sess  events  live
claude-code      23     760   308
```

That is four workspaces (Ngomia, ambyte, kumbarium,
sudocode-challenges at the time of writing) and dozens of
conversations folded into one identity. The dossier can split
it by session, but a session is one process, not one agent: there
is no way to ask "what has the agent working in ambyte done
across the last month", and one Claude session correcting
another is invisible to the survival math, because "corrected by
OTHERS" compares claimed names and both sides are `claude-code`.

## The model: three legs

| leg      | minted or claimed | lifetime             | answers              |
|----------|-------------------|----------------------|----------------------|
| name     | claimed           | whatever the client says | what client is this |
| session  | minted (D-044)    | one serve process    | which incarnation    |
| ACTOR    | minted (new)      | until merged/retired | who, across sessions |

An actor is a UUIDv7 the librarian mints, with a human-readable
name. Events never carry the name: they carry the session, and
the session is bound to exactly one actor. Renames and merges
therefore never touch the ledger.

## Minting: automatic, per client and workspace

At `initialize`, the librarian resolves the session's actor:

1. EXPLICIT: `kum serve --agent <name>` (or `KUMBARIUM_AGENT=<name>`
   in the client's MCP config) names a registered actor. An
   unknown name does not stop the server (an agent's server
   must always start, the config-warning precedent): it warns on
   stderr, falls back to step 2, and the bind event records the
   refused request.
2. AUTOMATIC: the key is (claimed name, workspace). The first
   time the pair is seen, an actor is minted; afterwards the
   same pair binds to the same actor.

The WORKSPACE is the git toplevel of the serve process's working
directory, else the working directory itself, canonicalized.
Verified on the live machine: every Claude Code serve runs with
its project directory as cwd. When a client announces MCP
`roots`, the first root can take precedence in a later
refinement; cwd is the floor that works today.

The default name is `<claimed>@<workspace leaf>`, e.g.
`claude-code@kumbarium`, with `-2`, `-3` on collision. Names use
the namespace alphabet plus `@` and never a reserved agent word.

## Binding: tamper-evident without touching the recipe

Every session writes one `actor_bind` event, before anything
else it does:

```json
{"actor": "<uuid>", "name": "claude-code@kumbarium",
 "via": "auto", "claimed": "claude-code",
 "workspace": "~/Desktop/projects/kumbarium"}
```

The event's detail is hashed, and every later event in the
session carries the session id, which is hashed too. So "which
actor did this" is already tamper-evident through the chain:
alter a bind and the chain breaks; alter an event's session and
the chain breaks. No new hashed column, no recipe change, no
re-chain. D-045's closed door stays closed.

The alternative (an `actor_id` column on every event, in the
recipe) was rejected: it buys nothing the bind does not, and it
would spend the one-way re-chain door for it.

## The registry

A new table in the library (`actors`: id, name UNIQUE, claimed,
workspace, created_at, merged_into, retired_at, note), the way
namespaces are registered in the library. The ledger stays the
authority on who acted; the registry is the catalog of names.
Every registry change is witnessed: new event kinds
`actor_mint`, `actor_bind`, `actor_rename`, `actor_merge`,
`actor_retire`, `actor_unretire` (an audit migration that only
widens the kind list, the usual rebuild dance, hashes untouched).

## The human's verbs

The agent-lifecycle family the reserved words have been holding
(`tools::RESERVED_AGENT_WORDS`) finally ships:

```
kum agents [--all]                  the roster, by actor
kum agent <actor>                   the dossier (as today)
kum agent add <name> [--human]      register one ahead of time
kum agent rename <actor> <new>      display name only; the id stays
kum agent merge <from> <into>       fold one actor's trail into
                                    another (from.merged_into)
kum agent retire | unretire <actor> hide from the roster, keep
                                    every record
```

`<actor>` is a name, a short id, or (for pre-actor history) a
claimed name. `merge` and `unretire` join the reserved words in
the same change, before any client could claim them.

A merge is a registry fact, never a rewrite: the ledger still
shows each session bound to its original actor, and reads follow
`merged_into`. Two workspaces for one project (a clone, a
worktree) are the expected reason to merge.

The config key `agents.retired` keeps working as a display filter
and is superseded by `kum agent retire`; the doctor can offer the
move.

## What the reads gain

- The ROSTER lists actors: name, short id, claimed client,
  workspace, sessions, events, live writes, corrections by
  others, last seen.
- The DOSSIER resolves an actor and reads every session bound to
  it (and to anything merged into it). `--session` still narrows.
- ENTRIES gain a nullable `actor_id` column (library migration),
  stamped on remember and supersede, so "corrected by OTHERS"
  compares actors and one Claude session correcting another
  finally counts. The docket, diary, and leases need no column:
  their writes are ledger events, and the ledger attributes them.
- The OPENING FRAME tells the agent who it is, one line: `you are
  claude-code@kumbarium (a1b2c3d4)`, so briefings and matters it
  files are written by a self it can name.
- `kum doc` can later grow actor pages (the dossier as a site).

## Honesty, stated once

- It DISAMBIGUATES, it does not AUTHENTICATE (D-044's sentence,
  unchanged). A client can still claim any name, and can pass
  `--agent` for an actor it is not. Authenticated identity is the
  daemon rung's, and when it lands, the credential binds to this
  same actor id.
- SUBAGENTS share their parent's MCP connection and are
  indistinguishable from it at this tier.
- HISTORY is not guessed. Sessions from before this change carry
  no bind event and stay unbound; the roster shows them under
  their claimed name, marked unbound. No heuristic back-fill.
- Actor ids are LOCAL, like confidence (D-028): bundles keep
  carrying the claimed name, and the receiving library binds its
  own actors.

## The human at the terminal

The CLI is an actor too: the human. Today it claims
`kumbarium-cli` and binds to nothing. The human actor is minted
once per identity and bound to every CLI invocation's session,
with `kind = human` in the registry (agents are `kind = agent`),
so the roster and dossiers show who at the keyboard approved,
broke a lease, granted a secret, or forgot an entry.

Its identity source is configurable, defaulting to git:

```toml
[identity]
# git: user.name / user.email from `git config` as resolved in
# the working directory (repo config over global), falling back
# to os when git or the keys are absent.
# os: the OS login name.
# any other value: that fixed name.
human = "git"
```

The key is the email under `git` (so a repo that sets a work
address yields a distinct actor, which is the point) and the
login name under `os`; the display name is the slugged
`user.name` or the login. Running `git` is the fixed OS binary,
never a config value (CONFIG DECIDES VALUES, NEVER EXECUTABLES
holds). The bind event records the source (`via: "git"` or
`"os"`), and an identity source that resolves differently from
one command to the next simply binds a different actor: honest,
and mergeable.

## Secret grants follow actors

Grants move from claimed names to actors (review decision): the
grant table's grantee becomes an actor id, and `kum secret grant`
resolves `<actor>` the way every verb does (name, short id).
The consequence, chosen: a new workspace mints a new actor, and
a new actor holds NO credentials until granted. That is the
deny-by-default stance (D-038) applied honestly, and it closes a
real hole: today a grant to `claude-code` reaches every Claude
session on the machine, in every repository.

Existing name-keyed grants are not silently re-pointed (one
claimed name maps to many actors, and guessing which one earns
a credential is exactly the wrong place to guess). They keep
working as NAME-WIDE grants, listed distinctly (`grantee:
claude-code (name-wide, legacy)`), and the doctor reports each
one with the remedy: regrant to an actor, then revoke the
legacy row. New name-wide grants cannot be created. The grants
table gains a `grantee_kind` column (`actor` | `name`) in an
append-only migration.

## Settled out of scope: actors across libraries

Merging actors across libraries (bundles) was considered and
deferred in review. It would unify one agent's trail across machines and
attribute imported facts to named foreign actors, but actors do
not authenticate, so a bundle carrying actor ids could claim to
be a local actor and forge its record, and actor rows would
carry workspace paths off the machine. Bundles instead carry a
plain-text ORIGIN label on each entry (`claude-code@ambyte, via
bundle a1b2...`), shown in provenance and never trusted as
identity. Real cross-library identity waits for the daemon rung,
where an actor can prove itself.

## Build plan (built 2026-09-28)

1. Store: `actors` table (with `kind`) + `entries.actor_id`
   (append-only migration). Audit: the six new kinds. Secrets:
   `grantee_kind` on grants.
2. Serve: workspace resolution, `--agent` / `KUMBARIUM_AGENT`,
   resolve-or-mint at initialize, the bind event first,
   `ServerState.actor_id`, presence record gains the actor.
3. Writes: stamp `actor_id` on remember / supersede.
4. CLI: the human actor (`[identity] human`), the `kum agent`
   verbs, roster and dossier by actor, reserved words widened,
   grants to actors with legacy name-wide rows honored and
   reported by the doctor.
5. Frame: the "you are" line.
6. Tests: mint and rebind stability, explicit and refused
   `--agent`, merge-follows reads, bind-before-first-event
   ordering, chain verification across a bind, corrections by
   another actor with the same claimed name.
