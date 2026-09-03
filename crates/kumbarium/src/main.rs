//! Kumbarium: the librarian process. CLI today; the MCP server
//! and wiring land as v0.1 completes.

mod paths;

use std::process::ExitCode;

const VERSION: &str = env!("CARGO_PKG_VERSION");

fn main() -> ExitCode {
  let args: Vec<String> = std::env::args().skip(1).collect();
  match args.first().map(String::as_str) {
    Some("version") => {
      println!("kumbarium {VERSION}");
      ExitCode::SUCCESS
    }
    Some("paths") => match paths::resolve() {
      Ok(p) => {
        println!("{p}");
        ExitCode::SUCCESS
      }
      Err(e) => {
        eprintln!("kumbarium: {e}");
        ExitCode::FAILURE
      }
    },
    Some(other) => {
      eprintln!("kumbarium: unknown command {other:?}");
      eprintln!("{USAGE}");
      ExitCode::FAILURE
    }
    None => {
      println!("{USAGE}");
      ExitCode::SUCCESS
    }
  }
}

const USAGE: &str = "\
kumbarium: the place of remembering

Usage:
  kumbarium version   print the version
  kumbarium paths     print where persisted data lives";
