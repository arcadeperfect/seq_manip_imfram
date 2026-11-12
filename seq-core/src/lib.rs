//! seq_manip's domain code, straight from `../sequence_manipulator/src-tauri/src`.
//!
//! Nothing is copied: `state` and `thumbs` are the live app's own files.
//! `state`, `thumbs` and `commands` are the live app's own files, compiled
//! unchanged; `commands.rs` names Tauri, and `../tauri-shim` gives it just
//! enough Tauri to build. Left out: `lib.rs` (the Tauri builder) and
//! `sentry_init.rs`. If seq_manip changes, this follows.
//!
//! `backend` is ours: the managed state and one plain call per command.

#![allow(dead_code, unused_imports)]

#[path = "../../../sequence_manipulator/src-tauri/src/state.rs"]
pub mod state;
#[path = "../../../sequence_manipulator/src-tauri/src/thumbs.rs"]
pub mod thumbs;
#[path = "../../../sequence_manipulator/src-tauri/src/commands.rs"]
pub mod commands;

pub mod backend;

pub use sequitur;
