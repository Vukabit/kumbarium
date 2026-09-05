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

/// The completion script for one shell (covering both binary
/// names). None for an unknown shell.
fn script(shell: &str) -> Option<String> {
  let words = COMMAND_WORDS.join(" ");
  Some(match shell {
    "bash" => format!(
      "_kumbarium() {{\n\
       \x20 local cur=\"${{COMP_WORDS[COMP_CWORD]}}\"\n\
       \x20 if [ \"$COMP_CWORD\" -eq 1 ]; then\n\
       \x20   COMPREPLY=($(compgen -W \"{words}\" -- \"$cur\"))\n\
       \x20 fi\n\
       }}\n\
       complete -F _kumbarium kumbarium kum\n"
    ),
    "zsh" => format!(
      "#compdef kumbarium kum\n\
       local -a _kumbarium_cmds\n\
       _kumbarium_cmds=({words})\n\
       if (( CURRENT == 2 )); then\n\
       \x20 _describe 'command' _kumbarium_cmds\n\
       fi\n"
    ),
    "fish" => {
      let mut s = String::new();
      for bin in ["kumbarium", "kum"] {
        s.push_str(&format!(
          "complete -c {bin} -f -n '__fish_use_subcommand' \
           -a '{words}'\n"
        ));
      }
      s
    }
    _ => return None,
  })
}

/// Where `--install` writes each shell's script, relative to
/// $HOME: the conventional autoload path, plus any one manual
/// step the shell still needs. `zsh`/`bash` autoload only when
/// the path is on `$fpath` / bash-completion is active, so the
/// note names that; `fish` works the moment the file lands.
fn install_target(shell: &str, home: &str) -> Option<(String, String)> {
  Some(match shell {
    "bash" => (
      format!("{home}/.local/share/bash-completion/completions/kum"),
      "loads automatically if bash-completion is installed and \
       sourced from your shell rc"
        .into(),
    ),
    "zsh" => (
      format!("{home}/.zsh/completions/_kum"),
      "add this line to ~/.zshrc BEFORE `compinit`, then restart \
       the shell:\n  fpath=(~/.zsh/completions $fpath)"
        .into(),
    ),
    "fish" => (
      format!("{home}/.config/fish/completions/kum.fish"),
      "loads automatically on the next shell".into(),
    ),
    _ => return None,
  })
}

/// `kum completions bash|zsh|fish [--install]`: print the
/// completion script (stdout stays pure script, so it pipes to
/// a file), or `--install` writes it to the conventional path.
pub(crate) fn completions_cmd(shell: &str, install: bool) -> ExitCode {
  let Some(body) = script(shell) else {
    return fail(&format!(
      "unknown shell {shell:?}; completions speak bash, zsh, \
       and fish"
    ));
  };
  if !install {
    print!("{body}");
    // A hint on stderr when a human is watching: never on
    // stdout (the script must pipe cleanly) and never when
    // redirected (the reader is a file, not a person).
    use std::io::IsTerminal;
    if std::io::stdout().is_terminal()
      && let Ok(home) = std::env::var("HOME")
      && let Some((path, note)) = install_target(shell, &home)
    {
      eprintln!(
        "\nto install: kum completions {shell} --install \
         (writes {path})\n{note}"
      );
    }
    return ExitCode::SUCCESS;
  }
  let Ok(home) = std::env::var("HOME") else {
    return fail(
      "--install needs $HOME set; pipe the script to a path instead",
    );
  };
  let (path, note) =
    install_target(shell, &home).expect("shell already validated by script()");
  let p = std::path::Path::new(&path);
  if let Some(dir) = p.parent()
    && let Err(e) = std::fs::create_dir_all(dir)
  {
    return fail(&format!("creating {}: {e}", dir.display()));
  }
  if let Err(e) = std::fs::write(p, &body) {
    return fail(&format!("writing {path}: {e}"));
  }
  let sty = super::super::style::Style::detect();
  println!("installed {shell} completions to {path}");
  println!("{}", sty.dim(&note));
  ExitCode::SUCCESS
}
