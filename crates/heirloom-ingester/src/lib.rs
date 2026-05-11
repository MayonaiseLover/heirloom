//! # heirloom-ingester
//!
//! The contract every Heirloom ingester implements. An ingester is a small
//! adapter that knows how to extract memories from one external source —
//! a folder of markdown files, a browser's history database, a Slack export,
//! and so on.
//!
//! ## Implementing an ingester
//!
//! ```no_run
//! use async_trait::async_trait;
//! use heirloom_core::Memory;
//! use heirloom_ingester::{Ingester, IngestContext, IngestReport};
//!
//! pub struct MySource;
//!
//! #[async_trait]
//! impl Ingester for MySource {
//!     fn name(&self) -> &'static str { "mysource" }
//!     fn description(&self) -> &'static str { "Reads memories from My Source" }
//!
//!     async fn ingest(&self, ctx: &IngestContext) -> anyhow::Result<IngestReport> {
//!         let memory = Memory::new("mysource", "item", "hello from my source");
//!         let inserted = ctx.store.add(&memory)? as usize as u64;
//!         Ok(IngestReport { scanned: 1, inserted, skipped: 1 - inserted, errors: 0 })
//!     }
//! }
//! ```

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use heirloom_core::Store;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;

/// Context handed to an ingester during a run.
///
/// Holds the shared [`Store`], the cutoff timestamp for incremental ingestion,
/// and arbitrary string options pulled from the user's `config.toml`.
pub struct IngestContext {
    pub store: Arc<Store>,
    pub since: Option<DateTime<Utc>>,
    pub options: HashMap<String, String>,
}

impl IngestContext {
    /// Fetch an option as a string, falling back to the provided default.
    pub fn opt<'a>(&'a self, key: &str, default: &'a str) -> &'a str {
        self.options.get(key).map(|s| s.as_str()).unwrap_or(default)
    }
}

/// Summary of a single ingestion run.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct IngestReport {
    pub scanned: u64,
    pub inserted: u64,
    pub skipped: u64,
    pub errors: u64,
}

impl IngestReport {
    pub fn merge(&mut self, other: IngestReport) {
        self.scanned += other.scanned;
        self.inserted += other.inserted;
        self.skipped += other.skipped;
        self.errors += other.errors;
    }
}

/// The contract every ingester implements.
#[async_trait]
pub trait Ingester: Send + Sync {
    /// Short, lowercase, ASCII-only identifier. Used as the `source` tag on every memory.
    fn name(&self) -> &'static str;

    /// One-line, user-facing description.
    fn description(&self) -> &'static str;

    /// Run an ingestion pass. Should be idempotent — the [`Store`] dedupes
    /// by content hash so re-runs are safe.
    async fn ingest(&self, ctx: &IngestContext) -> anyhow::Result<IngestReport>;
}
