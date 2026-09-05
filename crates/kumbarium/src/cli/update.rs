//! `kum update` (D-048): the one networked verb. The librarian
//! never phones home; the network happens ONLY inside this
//! command, and it reaches the network through curl (the
//! tooling door, the way SQLite is storage's) rather than a
//! compiled-in HTTP stack that would weigh on every build for
//! a feature most installs never run.
//!
//! Package-manager installs defer to their manager; only the
//! standalone-tarball install self-replaces, and only after
//! verifying the download against its published SHA-256.

use std::process::ExitCode;

use serde_json::Value;

use super::super::style;
use super::term::*;

const REPO: &str = "Vukabit/kumbarium";
const TARGET: &str = env!("KUMBARIUM_TARGET");

pub(crate) fn update_cmd(check_only: bool, yes: bool) -> ExitCode {
  let sty = style::Style::detect();
  let current = env!("CARGO_PKG_VERSION");

  // What the network says is newest.
  let latest = match latest_release() {
    Ok(v) => v,
    Err(e) => return fail(&e),
  };
  let newer = version_gt(&latest.tag, current);
  if !newer {
    println!("kumbarium {current} is current (latest is {}).", latest.tag);
    return ExitCode::SUCCESS;
  }
  println!(
    "{}",
    sty.bold(&format!(
      "kumbarium {} is available (you have {current})",
      latest.tag
    ))
  );
  if let Some(body) = &latest.notes {
    // The changelog section is what changes; show it, trimmed.
    for line in body.lines().take(20) {
      println!("  {line}");
    }
  }
  if check_only {
    // Gate-able: exit 1 signals "an update exists", the same
    // way leakscan signals an exposure.
    return ExitCode::FAILURE;
  }

  // Channel ownership: defer only where overwriting the binary
  // would desync a package DATABASE. Homebrew is that case (the
  // cautionary tale the peers warn about), so it defers. Cargo
  // is NOT: `cargo install` drops a binary and a bookkeeping
  // file that goes cosmetically stale, never broken, if the
  // binary is replaced (rustup self-updates its own ~/.cargo/bin
  // binary; cargo-binstall replaces cargo binaries as its whole
  // job). So a cargo install self-replaces, with a note.
  let mut cargo_note = false;
  match install_channel() {
    Channel::Homebrew => {
      println!(
        "\nthis kumbarium was installed by Homebrew; update with:\n  \
         brew upgrade kumbarium"
      );
      return ExitCode::SUCCESS;
    }
    Channel::Unknown => {
      println!(
        "\ncannot tell how this kumbarium was installed; if a \
         package manager owns it, update through that instead \
         of here"
      );
      return ExitCode::SUCCESS;
    }
    Channel::Cargo => cargo_note = true,
    Channel::Standalone => {}
  }

  println!(
    "\n{}",
    sty.dim(
      "sections migrate forward on first open and do not \
       migrate back; downgrading afterward is unsupported (kum \
       backup list)"
    )
  );
  if !yes && !confirm(&format!("update to {}?", latest.tag)) {
    println!("left unchanged.");
    return ExitCode::SUCCESS;
  }
  match perform_update(&latest.tag) {
    Ok(()) => {
      println!(
        "updated to {}. Live sessions still speak the old \
         binary: kum serve reload",
        latest.tag
      );
      if cargo_note {
        // Honest about the one cosmetic side effect: cargo's
        // own record still names the version it installed.
        println!(
          "{}",
          sty.dim(
            "(this replaced a cargo-installed binary; cargo \
             install --list may show the old version until you \
             reinstall through cargo)"
          )
        );
      }
      ExitCode::SUCCESS
    }
    Err(e) => fail(&e),
  }
}

struct Release {
  tag: String,
  notes: Option<String>,
}

fn latest_release() -> Result<Release, String> {
  let url = format!("https://api.github.com/repos/{REPO}/releases/latest");
  let body = curl_get(&url)?;
  let v: Value = serde_json::from_str(&body)
    .map_err(|e| format!("GitHub returned unparseable JSON: {e}"))?;
  let tag = v
    .get("tag_name")
    .and_then(Value::as_str)
    .ok_or("no release found (the repo may have none yet)")?
    .to_string();
  let notes = v
    .get("body")
    .and_then(Value::as_str)
    .filter(|s| !s.is_empty())
    .map(str::to_string);
  Ok(Release { tag, notes })
}

enum Channel {
  Cargo,
  Homebrew,
  Standalone,
  Unknown,
}

fn install_channel() -> Channel {
  let Ok(exe) = std::env::current_exe() else {
    return Channel::Unknown;
  };
  let path = exe.to_string_lossy();
  if path.contains("/.cargo/") || path.contains("\\.cargo\\") {
    Channel::Cargo
  } else if path.contains("/Cellar/") || path.contains("/homebrew/") {
    Channel::Homebrew
  } else {
    // A path we do not recognize as package-manager-owned is
    // treated as standalone (self-replaceable); the migration
    // and confirmation gates make that safe.
    Channel::Standalone
  }
}

fn perform_update(tag: &str) -> Result<(), String> {
  let base = format!("https://github.com/{REPO}/releases/download/{tag}");
  let name = format!("kumbarium-{tag}-{TARGET}");
  let tar_url = format!("{base}/{name}.tar.gz");
  let sum_url = format!("{tar_url}.sha256");

  let tmp = std::env::temp_dir().join(format!(
    "kumbarium-update-{}-{}",
    std::process::id(),
    tag
  ));
  std::fs::create_dir_all(&tmp).map_err(|e| e.to_string())?;
  let tar_path = tmp.join("bundle.tar.gz");
  curl_download(&tar_url, &tar_path)?;

  // Verify before anything is swapped: "nothing to verify" is
  // a failure, never a silent pass.
  let published = curl_get(&sum_url)
    .map_err(|e| format!("no checksum to verify against: {e}"))?;
  let published = published
    .split_whitespace()
    .next()
    .unwrap_or("")
    .to_lowercase();
  let bytes = std::fs::read(&tar_path).map_err(|e| e.to_string())?;
  let actual = kumbarium_util::sha256_hex(&bytes);
  if published.is_empty() || actual != published {
    let _ = std::fs::remove_dir_all(&tmp);
    return Err(format!(
      "checksum mismatch (published {published}, downloaded \
       {actual}); refusing to install"
    ));
  }

  // Unpack via tar (the archive door), then swap both binaries
  // atomically-ish: each is renamed aside, the new one moved
  // into place, the old kept as .bak for one generation.
  let unpacked = tmp.join("unpacked");
  std::fs::create_dir_all(&unpacked).map_err(|e| e.to_string())?;
  run(
    "tar",
    &[
      "-xzf",
      &tar_path.to_string_lossy(),
      "-C",
      &unpacked.to_string_lossy(),
    ],
  )?;
  let payload = unpacked.join(&name);
  let exe = std::env::current_exe().map_err(|e| e.to_string())?;
  let bindir = exe
    .parent()
    .ok_or("cannot find the install directory")?
    .to_path_buf();
  for bin in ["kumbarium", "kum"] {
    let src = payload.join(bin);
    if !src.exists() {
      let _ = std::fs::remove_dir_all(&tmp);
      return Err(format!("the bundle is missing {bin}"));
    }
    let dst = bindir.join(bin);
    let bak = bindir.join(format!("{bin}.bak"));
    // Rename-aside (a running exe can be renamed but not
    // overwritten on Windows; harmless on unix), then move in.
    let _ = std::fs::rename(&dst, &bak);
    if let Err(e) = std::fs::copy(&src, &dst) {
      // Roll back this binary from its .bak.
      let _ = std::fs::rename(&bak, &dst);
      let _ = std::fs::remove_dir_all(&tmp);
      return Err(format!("installing {bin}: {e}"));
    }
    #[cfg(unix)]
    {
      use std::os::unix::fs::PermissionsExt;
      let _ =
        std::fs::set_permissions(&dst, std::fs::Permissions::from_mode(0o755));
    }
  }
  let _ = std::fs::remove_dir_all(&tmp);
  Ok(())
}

/// GET a URL's body as a string via curl (fail on HTTP error,
/// follow redirects, send a UA GitHub requires).
fn curl_get(url: &str) -> Result<String, String> {
  let out = std::process::Command::new("curl")
    .args([
      "-sSfL",
      "-H",
      "User-Agent: kumbarium-update",
      "-H",
      "Accept: application/vnd.github+json",
      url,
    ])
    .output()
    .map_err(|e| format!("curl is required for kum update: {e}"))?;
  if !out.status.success() {
    return Err(format!(
      "fetching {url} failed: {}",
      String::from_utf8_lossy(&out.stderr).trim()
    ));
  }
  String::from_utf8(out.stdout).map_err(|e| e.to_string())
}

fn curl_download(url: &str, to: &std::path::Path) -> Result<(), String> {
  run(
    "curl",
    &[
      "-sSfL",
      "-H",
      "User-Agent: kumbarium-update",
      "-o",
      &to.to_string_lossy(),
      url,
    ],
  )
  .map_err(|e| format!("downloading {url}: {e}"))
}

fn run(cmd: &str, args: &[&str]) -> Result<(), String> {
  let out = std::process::Command::new(cmd)
    .args(args)
    .output()
    .map_err(|e| format!("{cmd}: {e}"))?;
  if out.status.success() {
    Ok(())
  } else {
    Err(String::from_utf8_lossy(&out.stderr).trim().to_string())
  }
}

/// vX.Y.Z (or X.Y.Z) semantic-ish comparison, numeric per
/// component. A malformed remote tag never reads as newer.
fn version_gt(candidate: &str, current: &str) -> bool {
  fn parts(s: &str) -> Vec<u64> {
    s.trim_start_matches('v')
      .split('.')
      .map(|p| p.chars().take_while(|c| c.is_ascii_digit()).collect())
      .map(|d: String| d.parse().unwrap_or(0))
      .collect()
  }
  let (a, b) = (parts(candidate), parts(current));
  for i in 0..a.len().max(b.len()) {
    let x = a.get(i).copied().unwrap_or(0);
    let y = b.get(i).copied().unwrap_or(0);
    if x != y {
      return x > y;
    }
  }
  false
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn version_comparison() {
    assert!(version_gt("v0.4.0", "0.3.0"));
    assert!(version_gt("0.3.1", "0.3.0"));
    assert!(version_gt("v1.0.0", "0.9.9"));
    assert!(!version_gt("v0.3.0", "0.3.0"));
    assert!(!version_gt("v0.2.9", "0.3.0"));
    // A garbage tag is never newer.
    assert!(!version_gt("nightly", "0.3.0"));
  }
}
