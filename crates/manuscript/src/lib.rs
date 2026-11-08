//! Manuscript projects (see `docs/manuscript-design.md`): a book, thesis or report kept as a
//! folder of ordinary documents. A `.wcproj` folder holds `project.toml` (the binder: a tree of
//! folders and sections, with titles, synopses, labels, status and word targets), one `.docx`
//! per section under `sections/` (named by the item's id, never its title, so each opens in any
//! word processor), `snapshots/` (saved versions of sections) and `research/`.
//!
//! Everything is a command: [`specs`] are the `project.*`, `binder.*` and `snapshot.*` commands,
//! and [`registry`] is the engine's commands plus these, for a [`wordcraft_engine::Session`].
//! Each command names the project folder in its parameters (`"project"`) and reads and writes
//! the folder itself, so they work the same from the CLI, the control channel and MCP. Files
//! are read and written through the engine's automation roots, and `project.toml` is hostile
//! input like any document: its size, item count and nesting are capped, and a file that can't
//! be read opens in recovery mode (every section listed) rather than failing.
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

pub mod binder;
mod cache;
mod commands;
pub mod project;
pub mod snapshot;

use std::sync::Arc;

use wordcraft_engine::{CommandSpec, Registry, Session};

pub use commands::specs;

/// The engine's commands and the manuscript commands.
pub fn registry() -> Registry {
    let mut all = wordcraft_engine::cmd::registry().all().to_vec();
    all.extend(specs());
    Registry::new(all)
}

/// Give `s` the manuscript commands as well as the engine's.
pub fn install(s: &mut Session) {
    s.registry = Arc::new(registry());
}

/// The specs [`specs`] adds, by id (for listings).
pub fn ids() -> Vec<&'static str> {
    specs().iter().map(|s: &CommandSpec| s.id).collect()
}

#[cfg(test)]
mod tests;
