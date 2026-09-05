# kum doctor: the mechanic (D-048)

The janitor judges FACTS (confidence, dormancy, circulation
misfires: epistemics). The doctor judges the BUILDING (files,
schemas, invariants, referential integrity: mechanics). A
finding about what an agent did belongs to the janitor; a
finding about a broken shelf belongs to the doctor. Neither
does the other's job, and the peer group agrees: git fsck,
restic check, and pg_amcheck all check plumbing, never
content.

This design follows a survey of the field (git fsck, restic
check, borg check, pg_amcheck, SQLite's integrity pragmas and
.recover, fsck(8), and the doctor-report grammar of brew /
flutter / npm doctor). Each stance below names where it came
from and whether we adopt, extend, or diverge.

## The surface

```
kum doctor                 examine; report; repair nothing
kum doctor --deep          the expensive tier (priced below)
kum doctor --apply         perform the preen-class repairs
kum doctor --json          machine findings, same structs
```

Preview-by-default with `--apply` sign-off is the house idiom
(revert, janitor), kept here DIVERGING from restic's
separate-repair-commands: their guard exists because repair
can destroy; our `--apply` set is preen-class only (below), so
the worst a flag typo can do is sweep debris.

Two affordances from the brew/npm convergence are DESIGNED but
not yet built: `--list-checks` (name every check) and
`<check-name>` (run one by name), cheap and good for support
conversations. `-v` verbosity is likewise reserved; today the
report already stays to one line per passing check, so the
healthy run is quiet without it.

## The check taxonomy

Grouped by section, one line per passing check, expanded block
only on findings (flutter's marks, minus its always-verbose
habit; brew's prose stream is the anti-pattern). Verbosity:
healthy output fits one screen; `-v` expands everything.

- SECTION INTEGRITY: `PRAGMA quick_check` per section by
  default, `integrity_check` under `--deep` (SQLite documents
  the cost/coverage split: quick skips index-content and
  UNIQUE verification; the verdict names the tier it earned).
  schema_version ahead of the binary (the downgrade shape);
  `.tmp-*.db` debris; WAL sidecars beside no live process;
  a maintenance lock held by a dead process.
- CHAIN HEALTH: distinguishes an UNHASHED TAIL (old-binary
  writes; recomputable, preen-class) from a HASH MISMATCH
  (evidence; never repaired, see below). Adopts git fsck's
  central lesson by vocabulary: benign-by-design states carry
  different names than corruption, or users learn to ignore
  everything.
- REFERENTIAL DRIFT: docket / handoff / lease rows on
  namespaces the registry no longer knows; grants to
  identities the ledger never witnessed; dangling
  superseded_by pointers; a continues-set with a missing part;
  two live standing briefings on one shelf.
- PROCESSES: the presence registry (process-lifecycle.md):
  live rows verified, unlocked records flagged as debris.
- CONFIG & KEYSTORE: malformed config lines; retired-agents
  entries naming unknown identities; a PRESENT-but-failing
  keystore (the downgrade-attack shape).
- BACKUPS: a section with no snapshot at all; a newest
  snapshot that fails its own integrity check (restic's
  read-data lesson scoped to our scale: verifying backups is
  the deep tier's job, and the backup pipeline already
  integrity-checks at write time, so the doctor re-checks only
  the newest snapshot per section, under `--deep`).

Never certify absence (pg_amcheck's dictum: a check can prove
presence of corruption, never its absence): the clean verdict
states the tier and the non-coverage, e.g.
`ok (quick tier; index cross-checks and backup reads need
--deep)`.

## The repair-vs-report line

`--apply` is fsck(8)'s PREEN MODE, adopted as the whole of
repair: an enumerated whitelist of minor, provably-lossless
fixes, and a hard refusal to grow past it. The whitelist:

- sweep `.tmp-*.db` debris and unlocked presence records
- clear a maintenance lock whose holder is dead
- checkpoint an orphaned WAL (no live process, verified)
- re-chain an unhashed ledger tail (pure recomputation from
  stored fields; already the open-time behavior)
- prune a corrupt snapshot AFTER a fresh good one is taken

Everything else is REPORT-ONLY, remedy printed verbatim
(restic's "you must run X" grammar; flutter's
remedy-per-finding placement):

- a hash mismatch, content divergence, or unwitnessed grant is
  EVIDENCE. Repairing evidence is destroying it; the doctor
  names the finding and points at the forensics (`kum audit
  verify`, the dossier, `kum backup list` + the hand restore).
  This is borg's own warning ("repair sacrifices data for the
  repository") taken to its conclusion: we simply do not ship
  the sacrifice.
- integrity failures inside a section point at the newest good
  snapshot and the documented hand restore, never an in-place
  rewrite. Salvage-to-the-side is unanimous in the field (git
  --lost-found, fsck lost+found, SQLite .recover into a NEW
  database); our side-channel already exists and is called a
  backup.

The borg horror story is a design input: reflexively repairing
a TRANSIENT problem does the damage the check only suspected.
This is why the preen whitelist excludes everything that
touches live section data: the shipped repairs (debris sweep,
unhashed-tail re-chain) cannot damage a false positive,
because they operate only on garbage and on recomputation.
The confirm-on-re-read refinement (a second check pass before
reporting a corruption-class finding) is reserved for if the
repair set ever grows toward data.

No `--reverse`: the preen whitelist has no meaningful inverses
(swept debris was garbage, a cleared dead lock is recreated on
demand, re-chaining backwards would DELETE hashes). Real
reversibility is state-level: `--apply` snapshots every
section it will touch before touching it, and the footer names
the snapshots and the restore page. An undo flag would be
weaker than what we ship and would imply the repairs are risky
enough to need one.

## Concurrency

- Preview is lock-free by construction: WAL read snapshots
  (SQLite's documented behavior: a check inside a read
  transaction sees one consistent snapshot beside live
  writers). Advertised in the page, per the survey's
  state-your-contract principle.
- `--apply` takes the maintenance lock (D-015). Transactional
  repairs (re-chain) run under WAL like any write. FILE
  SURGERY (WAL checkpoint, lock clearing, snapshot pruning)
  additionally requires an empty presence registry: live
  processes defer those repairs, and the deferral is REPORTED
  with the pids and clients to close, never silently skipped.
  Lock strength scales with depth, and the report says which
  repairs were deferred and why (pg_amcheck's pattern).

## Exit codes and output

- Exit 0 clean, exit 1 on any actionable finding, the house's
  uniform-1 stance (kum help conventions), DIVERGING from
  pg_amcheck's 0/1/2 and fsck's bitmask with the reason
  already inked: scripts branch on `--json`, the ledger holds
  detail, and a taxonomy of exit codes is precision nobody
  reads. The contract is documented in the manual page from
  day one (git fsck's undocumented bitmask is the
  anti-pattern).
- The report is plain stdout, pipeable and byte-identical
  piped; `--json` emits the same check-result structs the
  human renderer consumes, so the two cannot drift (the
  machine-output gap flutter never closed).
- The expected-noise firewall (brew's central failure): a
  check may not warn about a condition the user cannot or need
  not act on. Every warn must change a decision; informational
  states render as `ok` with a note, or live behind `-v`.

## Witnessing

Preview writes nothing and witnesses nothing (diagnosis is
browsing). `--apply` witnesses ONE doctor event carrying the
repair manifest and the finding counts, the janitor's
batch-event pattern; the event kind joins the ledger the same
way handoff_drop did.

## What the doctor refuses to become

- Not a data-recovery tool: restore is the hand move
  (kum help backup), .recover-class salvage is the SQLite
  shell's job, and both are pointed at, not wrapped.
- Not a kill switch: live processes are reported with pids;
  termination belongs to their clients and the OS
  (process-lifecycle.md).
- Not the janitor: nothing here reads content, scores facts,
  or judges agents.
