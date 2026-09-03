# The secrets broker: the restricted stacks (D-038)

The third section, and the one the manager stance was built
for: credentials are the most dangerous thing agents touch,
and today they live in env vars, dotfiles, and pasted-into-
context strings that no ledger ever sees. The broker gives
them a shelf with a locked door, a sign-out sheet, and a
librarian who never forgets who asked.

Design only; the build waits for sign-off, and one decision
below is explicitly reserved for the human.

## What the broker honestly buys (stdio tier)

Ink the truth before the features: at the personal tier, any
process running as the user can read any file the user can.
The broker does NOT protect against a malicious local process,
and pretending otherwise would be theater. What it buys is
real anyway:

- WITNESSED ACCESS. Every secret checkout lands on the
  hash-chained ledger: which agent, which secret, when. "Who
  has read the deploy key this month" becomes a query. This is
  the charter applied to credentials, and it is the product.
- SCOPED ACCESS. An agent gets only what was granted to it, so
  a confused agent's blast radius shrinks from "everything in
  the env" to "what its work needs". Deny by default.
- HYGIENE. Agents stop pasting keys into memories, tasks, and
  CLAUDE.md files because a proper place exists and the
  snippet says so. The leak-shaped failure mode gets a home
  that is not the general collection.
- EXFILTRATION RESISTANCE AT REST (with the encryption
  decision below): secrets.db copied off the machine alone is
  ciphertext.

Enforcement hardens at the daemon rung, where authn makes
grants real; the mechanism is built correct-first so
enforcement has something to guard (the D-027 posture).

## The cryptography ruling: the family precedent

Surveyed against [redacted] (the author's prior secrets system,
shipped) and [redacted] ([redacted], decision-locked design): the
question reserved here turned out to be already ruled, twice.
[redacted]'s written doctrine is HAND-ROLLED ON A VETTED FLOOR:
reimplementing an audited protocol stack VIOLATES the
hand-roll principle there, and its floor is the exact family
this design proposed. The D-012 amendment is therefore a port
of the author's own settled clause, not a new invention.
Sign-off still required; the text to sign is [redacted]'s.

Adopted from the precedent, sharper than the first draft:

- XCHACHA20-POLY1305 (not plain ChaCha20-Poly1305): 192-bit
  FRESH RANDOM nonce per seal, no counter state; an RNG
  failure fails closed, never a zero or reused nonce
  ([redacted]'s [external] rationale: birthday bound ~2^-96 at 2^48
  seals). Dependencies admitted: `chacha20poly1305` +
  `zeroize` (and `subtle` if any secret comparison appears),
  RustCrypto, pinned, permissive: the vetted floor, nothing
  above it (no KEMs, no signatures; this shelf seals and
  unseals, only).
- VERSIONED ENVELOPES ([redacted] [external]): every sealed blob leads
  with a version byte; an unknown version fails closed, never
  best-effort parses. Crypto-agility is bumping a byte.
- The master key never touches repo, config, or backups: it
  lives in the platform keystore, reached by SHELLING the OS
  tool (`security` / PowerShell / `secret-tool`), zero
  dependencies for that part.
- KEYSTORE TRI-STATE ([redacted]'s Presence model, adopted):
  PRESENT serves; genuinely ABSENT (no substrate exists)
  falls back loudly to the documented floor
  (`--i-accept-plaintext`, an explicit human choice); BLOCKED
  (present but suppressed or failing) REFUSES, because a
  suppressed keystore is how downgrade attacks look. Absent
  and Blocked are different facts and get different behavior.
- ZEROIZE WHAT WE OWN: sealed and unsealed value buffers are
  zeroize-on-drop, built at final size (a reallocation leaves
  bytes we cannot scrub, so we do not reallocate). Stated
  non-coverage: the JSON serialization and the pipe are
  copies we do not own. No mlock membrane: for a short-lived
  serve process that is the part that WOULD be theater, and
  the family's own tier analysis supports the scoping.

Enforcement tiers named per claim ([redacted]'s doc standard,
"never a bare adjective"): sealed-at-rest is CRYPTO-ENFORCED;
grants are POLICY (librarian-checked, witnessed) until daemon
authn; witnessed access is LEDGER-ENFORCED (hash chain);
value-free audit is TYPE-SHAPE-ENFORCED (see below); the
custody terminus at the pipe is STATED, not defended.

## Shelving

`library/secrets.db` per D-033: lazy file, per-shelf backups
(NOTE: backups of ciphertext are ciphertext; the master key is
NOT in any backup, which is correct and worth saying).
Namespace stored as the validated PATH, gate-checked. Rows:

- secrets: id (UUIDv7), namespace, name (unique per shelf),
  value (sealed), nonce, agent_id ("kumbarium-cli": human-only
  writes in v1), superseded_by, note, created_at, updated_at.
- grants: (namespace, name, agent_id, mode, expires_at NULL,
  created_at), managed and witnessed; deny by default; mode is
  'reveal' in v1 with 'use' reserved (see the custody conviction); expires_at is the lease column, enforced at
  read time from v1.5; no wildcard agents in v1 (a wildcard is
  a decision someone should have to type out per-secret at the
  daemon tier, not before).

## Lifecycle: rotation keeps the history, not the value

The one place supersede-never-delete BENDS, deliberately:

- `set` on an existing name supersedes, exactly like memory,
  so the ROTATION HISTORY is a chain: who rotated, when, how
  often. But the superseded row's sealed value is SHREDDED
  (overwritten) at rotation: the skeleton of history remains,
  the retired credential does not. An old key is not a memory,
  it is a liability.
- `shred` removes a secret entirely: the row is kept with its
  value destroyed (the judgment is recorded; the material is
  gone). There is no `forget` that erases the fact a secret
  existed: the ledger would contradict it anyway.
- No desk in v1 because agents cannot write secrets at all:
  credential poisoning is the one injection with no
  redeeming review story ("approve this new deploy key" is a
  social-engineering form, not a workflow).

## Custody: who holds the value, stated end to end

The chain, with its honest terminus: the human types the value
(echo off, never argv); it is sealed in-process immediately;
it rests as ciphertext with the master key in the OS keystore
(backups carry ciphertext only, never the key); it is unsealed
only inside the librarian at read time; it crosses to the
agent over the stdio pipe (kernel memory, never disk, never
network); and there the broker's custody ENDS, because the
value now lives in the model's context window. Two
consequences, named rather than hidden:

- The MCP CLIENT logs tool results: a returned secret lands in
  the client's session transcript on disk, outside these
  walls. No broker fixes another program's logging. The
  snippet teaches around it, the leak fixtures police our own
  shelves, and the eventual answer is USE-NOT-SEE: the broker
  applying a credential instead of revealing it. Use-not-see
  makes the librarian execute requests, which grinds against
  manager-never-executes; that tension is real and is deferred
  deliberately, not papered over. (The CLI-side
  `kum secret exec <ns> <name> -- cmd`, human-invoked env
  injection, is the tension-free half and a fine v2.)
- No zeroization theater: the value passes through JSON and a
  pipe; pretending we scrub memory would be a lie. The serve
  process is short-lived per session, which is the honest
  mitigation.

## Expiry: leases are real, value expiry is metadata

Two different things, deliberately separated:

- GRANT EXPIRY (leases) is enforceable even at the stdio tier,
  because every secret_read re-checks grants at read time;
  nothing is cached to outlive a revocation. `grant ...
  --until DATE` is an expires_at column consulted on every
  read: honest enforcement, no theater, and revocation is
  already instantaneous for the same reason.
- VALUE EXPIRY (the credential itself expires upstream) is
  metadata the broker records and surfaces (`expires_at` shown
  by `kum secrets`), never enforces. The sections compose: an
  expiring credential is a docket matter with a goal date, and
  D-037's creep machinery already interrupts sessions for
  overdue goals. The broker knows the date; the docket does
  the reminding; the janitor gains an "expired credential
  still stocked" finding in v2.

## Transport, and the network gate inked in advance

stdio tier: librarian to agent over a kernel pipe; the value
never touches disk or network in our custody. The daemon rung
changes the stakes, so the gate is inked NOW: secrets never
cross a network transport until TLS and real authn both exist.
A future HTTP serve mode REFUSES secret_read outright rather
than degrading, the same refuse-don't-downgrade posture as the
keystore rule. Until that rung, the restricted stacks are a
local-tier section by construction.

## Never served, ever

The section's standing exception to D-037, inked in advance:
NOTHING on this shelf rides recall, briefings, or matters.
Secrets are pull-only, by name, one at a time. `recall`, list
surfaces, grep, exports, and minutes never contain a value;
access events render as names and agents only. The inheritance
contract already withholds FTS and split; this shelf also
withholds serving.

## The custody conviction, and the seam it demands

[redacted]'s founding line ([redacted] is "[redacted]'s Secret<T>/membrane/
custody story re-derived one floor down"): systems "protect
the secret at rest and then surrender plaintext to the
consuming program, which is precisely where theft happens."
v1's secret_read IS that surrender, to the worst consumer in
the family's terms: a model context whose client logs tool
results. Stated, not hidden, and answered structurally where
possible today:

- GRANTS CARRY A MODE from day one: `reveal` (v1's only
  value) with `use` reserved, so the egress-broker future
  (the broker applying a credential instead of showing it)
  slots into the grant table without a migration. The
  manager-never-executes tension stays deferred; the schema
  stops it from becoming a rewrite.
- `kum secret copy` ships in v1: [redacted]'s concealed-clipboard
  pattern, ported light. The value goes to the clipboard via
  the shelled OS tool (`pbcopy` / `clip` / `xclip` family),
  never to stdout (terminal scrollback is a ledger too), with
  a spawned auto-clear after 90 seconds. Witnessed as
  secret_copy.

## Surfaces

MCP, ONE tool:

- `secret_read`: namespace + name. Returns the value IF a
  `reveal` grant exists for the calling identity; refuses
  otherwise, naming the grant command so the human can
  decide. Every call witnessed both ways: the REFUSAL is an
  event too. If the audit append fails, the value is
  WITHHELD (fail-closed, the [redacted] vaults posture; the
  existing audit-failure-fails-the-call machinery already
  delivers it, inked here as a guarantee). Event details are
  built from fixed fields only (name, agent, granted:bool):
  a secret cannot reach the ledger through this path by
  SHAPE, not by discipline.

CLI (human-only writes):

```
kum secret set <ns> <name>       value read from stdin or
                                 prompted with echo off; never
                                 an argv argument (shell
                                 history is a ledger too)
kum secret read <ns> <name>      print value (tty warning)
kum secret copy <ns> <name>      concealed clipboard copy,
                                 auto-clear 90s; never stdout
kum secrets [ns]                 names + metadata, never values
kum secret grant <ns> <name> <agent>
kum secret revoke <ns> <name> <agent>
kum secret shred <ns> <name>
```

Witness kinds: secret_set, secret_read, secret_grant,
secret_revoke, secret_shred (one audit migration). `kum
history` on a secret id renders the rotation chain, values
absent. The snippet gains one bullet: never write credential
VALUES into memories, tasks, or briefings; ask secret_read,
and if refused, ask the human for a grant.

## Non-goals (v1)

- No agent writes, no desk flow for secrets.
- No auto-rotation (the docket holds a rotation task with a
  goal date today; creep does the reminding). Grant leases
  (`--until`) and value-expiry metadata are v1.5: cheap and
  honest, but the core lands first.
- No wildcard grants, no grant delegation.
- Secrets never travel in bundles (inked forever, not just
  v1).
- No environment injection / exec wrapper (a fine v2:
  `kum secret exec <name> -- cmd`; out of scope now).

## Testing shape (when built)

Store: seal/unseal round-trip, rotation shreds the ancestor's
value (bytes provably gone), grants deny by default, unique
name per shelf. CLI: echo-off entry, grant round-trip, history
shows chain without values. rpc: secret_read granted vs
refused, witnessed both ways. Persona harness, two fixtures
worth having: the GRANT fixture (agent asks for a granted
secret, uses it, and the grader proves the VALUE appears in no
memory, no task, no briefing, and no minutes: the leak check
greps every shelf and every export for the secret bytes) and
the REFUSAL fixture (ungranted agent is refused, and the
refusal event is on the ledger with the agent's name).
