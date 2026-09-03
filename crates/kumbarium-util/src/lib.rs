//! Foundational shared helpers: a time-ordered id (UUIDv7),
//! strict ISO-8601 timestamps, a crash-safe atomic byte writer, a
//! size-capped symlink-refusing reader, and a process lock. The
//! base of the dependency graph; external deps only, no domain
//! logic, no dependency on any other Kumbarium module.

#![forbid(unsafe_code)]

mod fs;
mod id;
mod lock;
mod time;

pub use fs::{ReadError, WriteError, read_bytes, write_atomically};
pub use id::{generate_id, is_valid_id};
pub use lock::{LockError, ProcessLock};
pub use time::{format_iso8601_ms, now_iso8601, now_ms, parse_iso8601_ms};
