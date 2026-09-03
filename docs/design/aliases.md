# Config aliases (BUILT 2026-09-03, D-035)

Personal command vocabulary via config, git-style. Designed and
built 2026-09-03; this doc is the standing reference.

```toml
[alias]
urgent = "tasks --severity urgent"
mins = "export minutes --open"
```

Dispatch preprocessing: if argv[0] is no builtin, look it up in
the alias table, splice the expansion in front of the remaining
args, dispatch once.

## The three rules (load-bearing, not preferences)

1. INTERNAL-ONLY, never shell. An alias expands to a kumbarium
   argv prefix and nothing else; there is no `!` form. This is
   the write-path rule applied again: config.toml is writable
   by anything running as the user, a compromised agent
   included, and the moment config can reach the shell, every
   writer of that file becomes a code-execution principal. With
   internal-only expansion a poisoned alias can only invoke a
   kumbarium command the attacker could run directly, and it
   executes inside the gatekeeper, witnessed as itself (the
   ledger never sees the nickname, so nothing launders).
2. BUILTINS ALWAYS WIN. An alias can never shadow a documented
   command (shadowing attempts warn and are ignored), so the
   shipped surface is unforgeable and the man-page-is-the-
   whole-truth principle governs everything that is promised;
   aliases only add names in the free space.
3. ONE EXPANSION, no recursion. The expansion must begin with a
   builtin; alias-to-alias cannot loop because expansion never
   re-enters.

The standing line this feature writes into doctrine when built:
CONFIG DECIDES VALUES, NEVER EXECUTABLES. The CLI's only
external spawns stay --show (OS opener) and --open
($VISUAL/$EDITOR), both driven by environment convention and an
explicit human flag, never by config content.

## Implementation notes

- Parser: `[alias]` is the one section accepting ARBITRARY keys
  (every other section warns on unknowns); section-scoped
  exception, commented.
- `kum config` lists effective aliases; `kum help alias` states
  the three rules; unknown-command errors say "no command or
  alias {word:?}".
- MCP surface untouched: aliases are human vocabulary only.
- D-035.
