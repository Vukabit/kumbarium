//! `kum`: the short alias for the kumbarium CLI. Same code,
//! second name, so every install channel ships both.

#[path = "../main.rs"]
mod app;

fn main() -> std::process::ExitCode {
  app::run()
}
