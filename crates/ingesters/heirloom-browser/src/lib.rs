//! # heirloom-browser
//!
//! Ingests browser history from Chromium-family browsers: Chrome, Brave, Arc,
//! Edge, and Vivaldi. Firefox lands in v0.3.
//!
//! ## Safety
//!
//! Browsers hold an exclusive lock on the live `History` SQLite file. Reading
//! it directly while the browser is open fails with `database is locked`.
//! This ingester **copies the file to a temp location and reads from the copy**,
//! so it works even while the browser is running.
//!
//! ## Privacy
//!
//! Only `url`, `title`, `visit_count`, and `last_visit_time` are read. No
//! cookies, no form data, no passwords. The temp copy is deleted at the end
//! of the ingestion run.

use async_trait::async_trait;
use chrono::{DateTime, TimeZone, Utc};
use directories::BaseDirs;
use heirloom_core::Memory;
use heirloom_ingester::{IngestContext, IngestReport, Ingester};
use rusqlite::Connection;
use std::path::{Path, PathBuf};
use tracing::{debug, warn};

/// Browser-history ingester.
pub struct BrowserIngester;

#[derive(Debug, Clone, Copy)]
enum Family {
    Chromium {
        name: &'static str,
        profile_path: &'static [&'static str],
    },
}

const PROFILES: &[Family] = &[
    Family::Chromium {
        name: "chrome",
        profile_path: &["Google", "Chrome", "Default"],
    },
    Family::Chromium {
        name: "brave",
        profile_path: &["BraveSoftware", "Brave-Browser", "Default"],
    },
    Family::Chromium {
        name: "arc",
        profile_path: &["Arc", "User Data", "Default"],
    },
    Family::Chromium {
        name: "edge",
        profile_path: &["Microsoft", "Edge", "Default"],
    },
    Family::Chromium {
        name: "vivaldi",
        profile_path: &["Vivaldi", "Default"],
    },
];

#[async_trait]
impl Ingester for BrowserIngester {
    fn name(&self) -> &'static str {
        "browser"
    }

    fn description(&self) -> &'static str {
        "Reads history from Chromium-family browsers (Chrome, Brave, Arc, Edge, Vivaldi)."
    }

    async fn ingest(&self, ctx: &IngestContext) -> anyhow::Result<IngestReport> {
        let store = ctx.store.clone();
        let since = ctx.since;
        let configured_paths: Vec<PathBuf> = ctx
            .opt("paths", "")
            .split(',')
            .filter(|s| !s.is_empty())
            .map(PathBuf::from)
            .collect();

        let report = tokio::task::spawn_blocking(move || -> anyhow::Result<IngestReport> {
            let mut report = IngestReport::default();

            let candidates: Vec<(String, PathBuf)> = if !configured_paths.is_empty() {
                configured_paths
                    .into_iter()
                    .map(|p| {
                        (
                            p.file_name()
                                .map(|s| s.to_string_lossy().to_string())
                                .unwrap_or_else(|| "browser".into()),
                            p,
                        )
                    })
                    .collect()
            } else {
                discover_history_files()
            };

            if candidates.is_empty() {
                warn!("no browser history files found — pass --path explicitly to override");
                return Ok(report);
            }

            for (name, path) in candidates {
                match ingest_one(&name, &path, since, store.as_ref()) {
                    Ok(r) => report.merge(r),
                    Err(e) => {
                        warn!("failed to ingest {} from {}: {}", name, path.display(), e);
                        report.errors += 1;
                    }
                }
            }

            Ok(report)
        })
        .await??;

        Ok(report)
    }
}

fn discover_history_files() -> Vec<(String, PathBuf)> {
    let mut out = Vec::new();
    let base = match BaseDirs::new() {
        Some(b) => b,
        None => return out,
    };
    // Chromium browsers store under different roots per OS.
    let roots: Vec<PathBuf> = if cfg!(target_os = "macos") {
        vec![base.data_dir().to_path_buf()] // ~/Library/Application Support
    } else if cfg!(target_os = "windows") {
        vec![base.data_local_dir().to_path_buf()] // %LocalAppData%
    } else {
        // Linux: under ~/.config
        vec![base.config_dir().to_path_buf()]
    };

    for family in PROFILES {
        match family {
            Family::Chromium { name, profile_path } => {
                for root in &roots {
                    let mut p = root.clone();
                    for seg in *profile_path {
                        p.push(seg);
                    }
                    p.push("History");
                    if p.exists() {
                        debug!("found {} history at {}", name, p.display());
                        out.push((name.to_string(), p));
                    }
                }
            }
        }
    }
    out
}

fn ingest_one(
    browser_name: &str,
    history_path: &Path,
    since: Option<DateTime<Utc>>,
    store: &heirloom_core::Store,
) -> anyhow::Result<IngestReport> {
    // Copy to a temp file so we don't fight the browser for the lock.
    let tmp_dir = tempdir()?;
    let copy_path = tmp_dir.path().join("History");
    std::fs::copy(history_path, &copy_path)?;

    let conn = Connection::open_with_flags(&copy_path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)?;
    // Chromium WebKit time = microseconds since 1601-01-01 UTC.
    // We convert to Unix seconds.
    let since_webkit: i64 = since.map(|d| webkit_from_unix(d.timestamp())).unwrap_or(0);

    let mut stmt = conn.prepare(
        "SELECT url, title, visit_count, last_visit_time
         FROM urls
         WHERE last_visit_time >= ?1 AND title IS NOT NULL AND title != ''
         ORDER BY last_visit_time DESC",
    )?;

    let mut report = IngestReport::default();
    let mut batch: Vec<Memory> = Vec::with_capacity(256);

    let rows = stmt.query_map([since_webkit], |row| {
        let url: String = row.get(0)?;
        let title: String = row.get(1)?;
        let visits: i64 = row.get(2)?;
        let last_visit_webkit: i64 = row.get(3)?;
        Ok((url, title, visits, last_visit_webkit))
    })?;

    for r in rows {
        let (url, title, visits, last_webkit) = match r {
            Ok(v) => v,
            Err(_) => {
                report.errors += 1;
                continue;
            }
        };
        report.scanned += 1;

        let last_unix = unix_from_webkit(last_webkit);
        let last_visit = Utc
            .timestamp_opt(last_unix, 0)
            .single()
            .unwrap_or_else(Utc::now);

        let content = format!("{}\n{}", title, url);
        let mut m = Memory::new("browser", "page", &content);
        m.metadata.insert("url".into(), url);
        m.metadata.insert("title".into(), title);
        m.metadata
            .insert("browser".into(), browser_name.to_string());
        m.metadata.insert("visit_count".into(), visits.to_string());
        m.created_at = last_visit;
        batch.push(m);

        if batch.len() >= 256 {
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
}

// Chromium WebKit epoch: 1601-01-01 UTC, in microseconds.
const WEBKIT_UNIX_DELTA_SECS: i64 = 11_644_473_600;

fn webkit_from_unix(unix_secs: i64) -> i64 {
    (unix_secs + WEBKIT_UNIX_DELTA_SECS) * 1_000_000
}

fn unix_from_webkit(webkit_us: i64) -> i64 {
    (webkit_us / 1_000_000) - WEBKIT_UNIX_DELTA_SECS
}

fn tempdir() -> anyhow::Result<TempDirHandle> {
    let mut path = std::env::temp_dir();
    let unique = format!("heirloom-browser-{}", uuid_v4_short());
    path.push(unique);
    std::fs::create_dir_all(&path)?;
    Ok(TempDirHandle(path))
}

struct TempDirHandle(PathBuf);
impl TempDirHandle {
    fn path(&self) -> &Path {
        &self.0
    }
}
impl Drop for TempDirHandle {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn uuid_v4_short() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    format!("{:x}", nanos)
}

#[cfg(test)]
mod tests {
    use super::*;
    use heirloom_core::Store;
    use std::sync::Arc;

    fn fake_history_db(path: &Path) {
        let conn = Connection::open(path).unwrap();
        conn.execute_batch(
            "CREATE TABLE urls (
                id INTEGER PRIMARY KEY,
                url TEXT NOT NULL,
                title TEXT,
                visit_count INTEGER DEFAULT 0,
                last_visit_time INTEGER DEFAULT 0
            );",
        )
        .unwrap();
        let now_webkit = webkit_from_unix(Utc::now().timestamp());
        conn.execute(
            "INSERT INTO urls (url, title, visit_count, last_visit_time)
             VALUES
             ('https://example.com/post', 'Postgres tuning notes', 4, ?1),
             ('https://example.com/rust', 'Rust async traits stable', 2, ?1),
             ('https://example.com/empty', '', 1, ?1)",
            [now_webkit],
        )
        .unwrap();
    }

    #[tokio::test]
    async fn reads_chromium_history_format() {
        let tmp = tempfile::tempdir().unwrap();
        let history = tmp.path().join("History");
        fake_history_db(&history);

        let store = Arc::new(Store::in_memory().unwrap());
        let mut options = std::collections::HashMap::new();
        options.insert("paths".into(), history.display().to_string());
        let ctx = IngestContext {
            store: store.clone(),
            since: None,
            options,
        };

        let report = BrowserIngester.ingest(&ctx).await.unwrap();
        // Two rows have titles; one has empty title and is filtered.
        assert_eq!(report.inserted, 2, "{:?}", report);
        let hits = store.search("postgres tuning", 5, None).unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].memory.source, "browser");
        assert!(hits[0]
            .memory
            .metadata
            .get("url")
            .unwrap()
            .contains("example.com/post"));
    }

    #[test]
    fn webkit_conversion_roundtrip() {
        let now = 1_700_000_000i64;
        let webkit = webkit_from_unix(now);
        let back = unix_from_webkit(webkit);
        assert_eq!(now, back);
    }
}
