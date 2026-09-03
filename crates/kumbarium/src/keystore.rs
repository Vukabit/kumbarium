//! The master key, held by the platform keystore and reached by
//! shelling the OS tool (zero dependencies for this part,
//! D-039). Tri-state presence model: PRESENT
//! serves the key (minting one on first use), genuinely ABSENT
//! substrate falls back loudly to the human's explicit
//! plaintext choice, and BLOCKED (present but failing or
//! suppressed) REFUSES, because a suppressed keystore is what a
//! downgrade attack looks like.

#[cfg(any(target_os = "macos", target_os = "linux"))]
use std::process::Command;

use kumbarium_secrets::KEY_LEN;

#[cfg(any(target_os = "macos", target_os = "linux"))]
const SERVICE: &str = "kumbarium-master";
#[cfg(any(target_os = "macos", target_os = "linux"))]
const ACCOUNT: &str = "kumbarium";

// On targets without keystore wiring yet, only Absent is ever
// constructed; the other variants are the cross-platform
// contract, not dead code.
#[cfg_attr(
  not(any(target_os = "macos", target_os = "linux")),
  allow(dead_code)
)]
pub enum Keystore {
  Present([u8; KEY_LEN]),
  Absent,
  Blocked(String),
}

/// Fetch (or mint on first use) the master key.
pub fn master_key() -> Keystore {
  #[cfg(target_os = "macos")]
  {
    macos()
  }
  #[cfg(target_os = "linux")]
  {
    linux()
  }
  #[cfg(not(any(target_os = "macos", target_os = "linux")))]
  {
    // Windows: DPAPI wiring is deferred until a Windows host
    // can validate the real error paths (classifying error
    // paths without the real host to observe is guessing).
    // Absent = the documented loud-fallback path, never a
    // silent downgrade.
    Keystore::Absent
  }
}

#[cfg(any(target_os = "macos", target_os = "linux"))]
fn decode_hex(hex: &str) -> Option<[u8; KEY_LEN]> {
  let hex = hex.trim();
  if hex.len() != KEY_LEN * 2 {
    return None;
  }
  let mut key = [0u8; KEY_LEN];
  for (i, chunk) in hex.as_bytes().chunks(2).enumerate() {
    let s = std::str::from_utf8(chunk).ok()?;
    key[i] = u8::from_str_radix(s, 16).ok()?;
  }
  Some(key)
}

#[cfg(any(target_os = "macos", target_os = "linux"))]
fn encode_hex(key: &[u8; KEY_LEN]) -> String {
  key.iter().map(|b| format!("{b:02x}")).collect()
}

#[cfg(any(target_os = "macos", target_os = "linux"))]
fn mint() -> Option<[u8; KEY_LEN]> {
  let mut key = [0u8; KEY_LEN];
  getrandom::getrandom(&mut key).ok()?;
  Some(key)
}

#[cfg(target_os = "macos")]
fn macos() -> Keystore {
  let find = Command::new("security")
    .args(["find-generic-password", "-s", SERVICE, "-a", ACCOUNT, "-w"])
    .output();
  let out = match find {
    Ok(out) => out,
    // The `security` binary itself missing is an absent
    // substrate, not a suppressed one.
    Err(_) => return Keystore::Absent,
  };
  if out.status.success() {
    let hex = String::from_utf8_lossy(&out.stdout);
    return match decode_hex(&hex) {
      Some(key) => Keystore::Present(key),
      None => {
        Keystore::Blocked("keystore item exists but is not a valid key".into())
      }
    };
  }
  // Exit code 44 = errSecItemNotFound: no key yet, mint one.
  if out.status.code() == Some(44) {
    let Some(key) = mint() else {
      return Keystore::Blocked("OS RNG unavailable".into());
    };
    let add = Command::new("security")
      .args([
        "add-generic-password",
        "-s",
        SERVICE,
        "-a",
        ACCOUNT,
        "-w",
        &encode_hex(&key),
        "-U",
      ])
      .output();
    return match add {
      Ok(o) if o.status.success() => Keystore::Present(key),
      _ => Keystore::Blocked("keychain refused the new key".into()),
    };
  }
  Keystore::Blocked(format!("keychain error (exit {:?})", out.status.code()))
}

#[cfg(target_os = "linux")]
fn linux() -> Keystore {
  let find = Command::new("secret-tool")
    .args(["lookup", "service", SERVICE, "account", ACCOUNT])
    .output();
  let out = match find {
    Ok(out) => out,
    Err(_) => return Keystore::Absent,
  };
  if out.status.success() && !out.stdout.is_empty() {
    let hex = String::from_utf8_lossy(&out.stdout);
    return match decode_hex(&hex) {
      Some(key) => Keystore::Present(key),
      None => {
        Keystore::Blocked("keystore item exists but is not a valid key".into())
      }
    };
  }
  // Not found: mint and store (secret-tool reads the value on
  // stdin).
  let Some(key) = mint() else {
    return Keystore::Blocked("OS RNG unavailable".into());
  };
  use std::io::Write;
  let child = Command::new("secret-tool")
    .args([
      "store",
      "--label",
      "kumbarium master key",
      "service",
      SERVICE,
      "account",
      ACCOUNT,
    ])
    .stdin(std::process::Stdio::piped())
    .spawn();
  match child {
    Ok(mut c) => {
      if let Some(stdin) = c.stdin.as_mut() {
        let _ = stdin.write_all(encode_hex(&key).as_bytes());
      }
      match c.wait() {
        Ok(status) if status.success() => Keystore::Present(key),
        _ => Keystore::Blocked("secret-tool refused the key".into()),
      }
    }
    Err(_) => Keystore::Absent,
  }
}
