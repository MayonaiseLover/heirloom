//! # heirloom-core
//!
//! Core storage and search engine for Heirloom — a local-first, MCP-native
//! personal memory layer for AI.
//!
//! This crate exposes a [`Store`] backed by SQLite (FTS5) for full-text search
//! across heterogeneous memories. Memories are typed by their [`Source`] (e.g.
//! `fs`, `browser`, `claude`) and stored with rich metadata for filtering.
//!
//! ## Example
//!
//! ```no_run
//! use heirloom_core::{Store, Memory};
//!
//! # fn run() -> anyhow::Result<()> {
//! let store = Store::open("./heirloom.db")?;
//! let memory = Memory::new("fs", "note", "Buy milk on Tuesday");
//! store.add(&memory)?;
//!
//! let results = store.search("milk", 5, None)?;
//! assert_eq!(results.len(), 1);
//! # Ok(())
//! # }
//! ```

mod error;
mod memory;
mod store;

pub use error::{Error, Result};
pub use memory::{Memory, SearchFilters, SearchResult};
pub use store::{Embedder, Store};
