//! Shell completions, generated from the same command-word
//! list the did-you-mean machinery uses (one source, no
//! drift). Static scripts by design: completion must never
//! open the library or slow the shell down.

use std::process::ExitCode;

use super::term::fail;

/// Every top-level command word, shared with `nearest_command`.
pub(crate) const COMMAND_WORDS: &[&str] = &[
  "list",
  "show",
  "grep",
  "history",
  "confirm",
  "link",
  "move",
  "forget",
  "retire",
  "unretire",
  "revert",
  "janitor",
  "task",
  "tasks",
  "roadmap",
  "brief",
  "agents",
  "agent",
  "dossier",
  "leases",
  "lease",
  "handoff",
  "handoffs",
  "inbox",
  "review",
  "approve",
  "reject",
  "secret",
  "secrets",
  "audit",
  "export",
  "import",
  "namespace",
  "processes",
  "status",
  "backup",
  "doctor",
  "config",
  "paths",
  "serve",
  "instructions",
  "completions",
  "update",
  "version",
  "help",
];

/// `kum completions bash|zsh|fish`: print a completion script
/// for the caller's shell, covering both binary names.
pub(crate) fn completions_cmd(shell: &str) -> ExitCode {
  let words = COMMAND_WORDS.join(" ");
  match shell {
    "bash" => {
      println!(
        "_kumbarium() {{\n\
         \x20 local cur=\"${{COMP_WORDS[COMP_CWORD]}}\"\n\
         \x20 if [ \"$COMP_CWORD\" -eq 1 ]; then\n\
         \x20   COMPREPLY=($(compgen -W \"{words}\" -- \"$cur\"))\n\
         \x20 fi\n\
         }}\n\
         complete -F _kumbarium kumbarium kum"
      );
      ExitCode::SUCCESS
    }
    "zsh" => {
      println!(
        "#compdef kumbarium kum\n\
         local -a _kumbarium_cmds\n\
         _kumbarium_cmds=({words})\n\
         if (( CURRENT == 2 )); then\n\
         \x20 _describe 'command' _kumbarium_cmds\n\
         fi"
      );
      ExitCode::SUCCESS
    }
    "fish" => {
      for bin in ["kumbarium", "kum"] {
        println!(
          "complete -c {bin} -f -n '__fish_use_subcommand' \
           -a '{words}'"
        );
      }
      ExitCode::SUCCESS
    }
    other => fail(&format!(
      "unknown shell {other:?}; completions speak bash, zsh, \
       and fish"
    )),
  }
}
