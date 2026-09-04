//! The human-facing CLI, split along the building's own map:
//! term plumbing, the painted usage surfaces, the loading dock,
//! the collection commands, the desk, and upkeep. `main.rs`
//! keeps only wiring (dispatch, open_stores, serve, backups).

pub(crate) mod admin;
pub(crate) mod brief;
pub(crate) mod desk;
pub(crate) mod dock;
pub(crate) mod docket;
pub(crate) mod dossier;
pub(crate) mod entries;
pub(crate) mod handoff;
pub(crate) mod secret;
pub(crate) mod term;
pub(crate) mod usage;
