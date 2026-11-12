//! seq_manip's domain code, straight from `../sequence_manipulator/src-tauri/src`.
//!
//! Nothing is copied: `state` and `thumbs` are the live app's own files.
//! Only `commands.rs` (the Tauri IPC layer), `lib.rs` (the Tauri builder)
//! and `sentry_init.rs` are left out. If seq_manip changes, this follows.
//!
//! `session` is ours: what `commands.rs` does for the webview, as plain
//! calls (open a folder, stage, revert, commit), with no IPC or JSON.

#![allow(dead_code, unused_imports)]

#[path = "../../../sequence_manipulator/src-tauri/src/state.rs"]
pub mod state;
#[path = "../../../sequence_manipulator/src-tauri/src/thumbs.rs"]
pub mod thumbs;

pub mod session;

pub use sequitur;
