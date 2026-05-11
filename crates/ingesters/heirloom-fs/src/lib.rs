//! # heirloom-fs
//!
//! Walks a directory and ingests text-shaped files (`.md`, `.markdown`, `.txt`,
//! `.org`, `.rst`) as Heirloom memories. One memory per file. Hidden files and
//! common dependency/cache directories are skipped.
//!
//! ## Example
//!
//! ```no_run
//! use std::sync::Arc;
//! use std::collections::HashMap;
//! use heirloom_core::Store;
//! use heirloom_ingester::{Ingester, IngestContext};
//! use heirloom_fs::FsIngester;
//!
//! # async fn run() -> anyhow::Result<()> {
//! let store = Arc::new(Store::open("./heirloom.db")?);
//! let mut options = HashMap::new();
//! options.insert("path".to_string(), "/home/me/notes".to_string());
//! let ctx = IngestContext { store, since: None, options };
//! let report = FsIngester.ingest(&ctx).await?;
//! println!("inserted {} memories", report.inserted);
//! # Ok(())
//! # }
//! ```

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use heirloom_core::Memory;
use heirloom_ingester::{IngestContext, IngestReport, Ingester};
use std::path::Path;
use tracing::{debug, warn};
use walkdir::WalkDir;

/// Filesystem ingester.
pub struct FsIngester;

const TEXT_EXTENSIONS: &[&str] = &["md", "markdown", "txt", "org", "rst"];
const SKIP_DIRS: &[&str] = &[
    "node_modules",
    "target",
    ".git",
    ".venv",
    "venv",
    "dist",
    "build",
    "__pycache__",
];
const MAX_FILE_BYTES: u64 = 5 * 1024 * 1024; // 5 MiB cap per file

#[async_trait]
impl Ingester for FsIngester {
    fn name(&self) -> &'static str {
        "fs"
    }

    fn description(&self) -> &'static str {
        "Walks a directory and ingests text and markdown files."
    }

    async fn ingest(&self, ctx: &IngestContext) -> anyhow::Result<IngestReport> {
        let root = ctx.opt("path", ".").to_string();
        let root_path = Path::new(&root).to_path_buf();
        if !root_path.exists() {
            anyhow::bail!("path does not exist: {}", root);
        }

        let since = ctx.since;
        let store = ctx.store.clone();

        // Filesystem walks are blocking — push to a thread.
        let report = tokio::task::spawn_blocking(move || -> anyhow::Result<IngestReport> {
            let mut report = IngestReport::default();
            let mut batch: Vec<Memory> = Vec::with_capacity(128);

            for entry in WalkDir::new(&root_path)
                .follow_links(false)
                .into_iter()
                .filter_entry(|e| e.depth() == 0 || !is_skipped(e.path()))
            {
                let entry = match entry {
                    Ok(e) => e,
                    Err(e) => {
                        warn!("walk error: {}", e);
                        report.errors += 1;
                        continue;
                    }
                };
                if !entry.file_type().is_file() {
                    continue;
                }
                let path = entry.path();
                if !is_text_file(path) {
                    continue;
                }

                let meta = match entry.metadata() {
                    Ok(m) => m,
                    Err(e) => {
                        warn!("metadata error for {}: {}", path.display(), e);
                        report.errors += 1;
                        continue;
                    }
                };
                if meta.len() > MAX_FILE_BYTES {
                    debug!("skipping oversized file {}", path.display());
                    report.skipped += 1;
                    continue;
                }

                let modified: Option<DateTime<Utc>> = meta
                    .modified()
                    .ok()
                    .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                    .map(|d| {
                        DateTime::<Utc>::from_timestamp(d.as_secs() as i64, 0)
                            .unwrap_or_else(Utc::now)
                    });

                if let (Some(since), Some(modified)) = (since, modified) {
                    if modified < since {
                        report.skipped += 1;
                        continue;
                    }
                }

                let content = match std::fs::read_to_string(path) {
                    Ok(c) => c,
                    Err(_) => {
                        // Likely a binary file we misclassified. Skip silently.
                        report.skipped += 1;
                        continue;
                    }
                };
                let content = content.trim();
                if content.is_empty() {
                    report.skipped += 1;
                    continue;
                }

                report.scanned += 1;

                let kind = path
                    .extension()
                    .and_then(|s| s.to_str())
                    .unwrap_or("text")
                    .to_string();

                let mut m = Memory::new("fs", kind, content);
                m.metadata.insert("path".into(), path.display().to_string());
                if let Some(name) = path.file_stem().and_then(|s| s.to_str()) {
                    m.metadata.insert("title".into(), name.to_string());
                }
                if let Some(t) = modified {
                    m.created_at = t;
                    m.metadata.insert("modified_at".into(), t.to_rfc3339());
                }
                batch.push(m);

                if batch.len() >= 128 {
                    let inserted = store.add_many(&batch)? as u64;
                    report.inserted += inserted;
                    report.skipped += batch.len() as u64 - inserted;
                    batch.clear();
                }
            }

            if !batch.is_empty() {
                let inserted = store.add_many(&batch)? as u64;
                report.inserted += inserted;
                report.skipped += batch.len() as u64 - inserted;
            }

            Ok(report)
        })
        .await??;

        Ok(report)
    }
}

fn is_text_file(path: &Path) -> bool {
    path.extension()
        .and_then(|s| s.to_str())
        .map(|e| TEXT_EXTENSIONS.iter().any(|t| t.eq_ignore_ascii_case(e)))
        .unwrap_or(false)
}

fn is_skipped(path: &Path) -> bool {
    if let Some(name) = path.file_name().and_then(|s| s.to_str()) {
        if name.starts_with('.') && name != "." {
            return true;
        }
        if SKIP_DIRS.contains(&name) {
            return true;
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;
    use heirloom_core::Store;
    use std::sync::Arc;
    use tempfile::TempDir;

    #[tokio::test]
    async fn ingests_markdown_files() {
        let tmp = TempDir::new().unwrap();
        std::fs::write(tmp.path().join("a.md"), "hello world from a markdown file").unwrap();
        std::fs::write(
            tmp.path().join("b.txt"),
            "and another note about the project",
        )
        .unwrap();
        std::fs::write(tmp.path().join("ignore.png"), [0u8, 1, 2, 3]).unwrap();

        let store = Arc::new(Store::in_memory().unwrap());
        let mut opts = std::collections::HashMap::new();
        opts.insert("path".into(), tmp.path().display().to_string());
        let ctx = IngestContext {
            store: store.clone(),
            since: None,
            options: opts,
        };

        let report = FsIngester.ingest(&ctx).await.unwrap();
        assert_eq!(report.inserted, 2, "{:?}", report);
        assert_eq!(store.count().unwrap(), 2);
    }

    #[tokio::test]
    async fn second_run_is_idempotent() {
        let tmp = TempDir::new().unwrap();
        std::fs::write(tmp.path().join("a.md"), "same content twice").unwrap();
        let store = Arc::new(Store::in_memory().unwrap());
        let mut opts = std::collections::HashMap::new();
        opts.insert("path".into(), tmp.path().display().to_string());
        let ctx = IngestContext {
            store: store.clone(),
            since: None,
            options: opts,
        };
        FsIngester.ingest(&ctx).await.unwrap();
        let report2 = FsIngester.ingest(&ctx).await.unwrap();
        assert_eq!(report2.inserted, 0);
        assert_eq!(store.count().unwrap(), 1);
    }
}
