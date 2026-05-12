//! # heirloom-firefox
//!
//! Reads Firefox's `places.sqlite` history database. Like the Chromium
//! ingester, copies to a temp file first so the live profile lock doesn't
//! block ingestion.
//!
//! Firefox stores `last_visit_date` as **microseconds since Unix epoch**
//! (not WebKit time like Chromium).

use async_trait::async_trait;
use chrono::{TimeZone, Utc};
use directories::BaseDirs;
use heirloom_core::Memory;
use heirloom_ingester::{IngestContext, IngestReport, Ingester};
use rusqlite::Connection;
use std::path::{Path, PathBuf};
use tracing::warn;

pub struct FirefoxIngester;

#[async_trait]
impl Ingester for FirefoxIngester {
    fn name(&self) -> &'static str {
        "firefox"
    }

    fn description(&self) -> &'static str {
        "Reads Firefox places.sqlite history."
    }

    async fn ingest(&self, ctx: &IngestContext) -> anyhow::Result<IngestReport> {
        let store = ctx.store.clone();
        let configured: Vec<PathBuf> = ctx
            .opt("paths", "")
            .split(',')
            .filter(|s| !s.is_empty())
            .map(PathBuf::from)
            .collect();

        let report = tokio::task::spawn_blocking(move || -> anyhow::Result<IngestReport> {
            let candidates = if configured.is_empty() {
                discover_places_files()
            } else {
                configured
            };
            if candidates.is_empty() {
                warn!("no Firefox profile found — pass --path to override");
                return Ok(IngestReport::default());
            }
            let mut report = IngestReport::default();
            for path in candidates {
                match ingest_one(&path, store.as_ref()) {
                    Ok(r) => report.merge(r),
                    Err(e) => {
                        warn!("failed to ingest {}: {}", path.display(), e);
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

fn discover_places_files() -> Vec<PathBuf> {
    let mut out = Vec::new();
    let Some(base) = BaseDirs::new() else {
        return out;
    };
    // Firefox profile roots vary by OS.
    let roots: Vec<PathBuf> = if cfg!(target_os = "macos") {
        vec![base.data_dir().join("Firefox").join("Profiles")]
    } else if cfg!(target_os = "windows") {
        vec![base
            .config_dir()
            .join("Mozilla")
            .join("Firefox")
            .join("Profiles")]
    } else {
        vec![
            base.home_dir().join(".mozilla").join("firefox"),
            base.home_dir()
                .join("snap")
                .join("firefox")
                .join("common")
                .join(".mozilla")
                .join("firefox"),
        ]
    };
    for root in roots {
        let Ok(entries) = std::fs::read_dir(&root) else {
            continue;
        };
        for entry in entries.flatten() {
            let p = entry.path().join("places.sqlite");
            if p.exists() {
                out.push(p);
            }
        }
    }
    out
}

fn ingest_one(places: &Path, store: &heirloom_core::Store) -> anyhow::Result<IngestReport> {
    let tmp_root = std::env::temp_dir().join(format!("heirloom-firefox-{}", uuid_short()));
    std::fs::create_dir_all(&tmp_root)?;
    let _guard = TempGuard(tmp_root.clone());
    let copy_path = tmp_root.join("places.sqlite");
    std::fs::copy(places, &copy_path)?;

    let conn = Connection::open_with_flags(&copy_path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)?;
    let mut stmt = conn.prepare(
        "SELECT url, COALESCE(title, ''), visit_count, last_visit_date
         FROM moz_places
         WHERE last_visit_date IS NOT NULL AND title IS NOT NULL AND title != ''
         ORDER BY last_visit_date DESC",
    )?;

    let mut report = IngestReport::default();
    let mut batch = Vec::with_capacity(256);
    let rows = stmt.query_map([], |row| {
        let url: String = row.get(0)?;
        let title: String = row.get(1)?;
        let visits: i64 = row.get(2)?;
        let last_us: i64 = row.get(3)?;
        Ok((url, title, visits, last_us))
    })?;
    for r in rows {
        let (url, title, visits, last_us) = match r {
            Ok(v) => v,
            Err(_) => {
                report.errors += 1;
                continue;
            }
        };
        report.scanned += 1;
        let secs = last_us / 1_000_000;
        let ts = Utc.timestamp_opt(secs, 0).single().unwrap_or_else(Utc::now);
        let mut m = Memory::new("firefox", "page", format!("{}\n{}", title, url));
        m.created_at = ts;
        m.metadata.insert("url".into(), url);
        m.metadata.insert("title".into(), title);
        m.metadata.insert("visit_count".into(), visits.to_string());
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

struct TempGuard(PathBuf);
impl Drop for TempGuard {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn uuid_short() -> String {
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

    fn fake_places(path: &Path) {
        let conn = Connection::open(path).unwrap();
        conn.execute_batch(
            "CREATE TABLE moz_places (
                id INTEGER PRIMARY KEY,
                url TEXT NOT NULL,
                title TEXT,
                visit_count INTEGER DEFAULT 0,
                last_visit_date INTEGER
            );",
        )
        .unwrap();
        let now_us = Utc::now().timestamp_micros();
        conn.execute(
            "INSERT INTO moz_places (url, title, visit_count, last_visit_date) VALUES
             ('https://example.com/rust', 'Rust async traits', 3, ?1),
             ('https://example.com/pg',   'Postgres tuning',   2, ?1),
             ('https://example.com/none', '',                  1, ?1)",
            [now_us],
        )
        .unwrap();
    }

    #[tokio::test]
    async fn reads_firefox_history_format() {
        let tmp = tempfile::tempdir().unwrap();
        let places = tmp.path().join("places.sqlite");
        fake_places(&places);

        let store = Arc::new(Store::in_memory().unwrap());
        let mut opts = std::collections::HashMap::new();
        opts.insert("paths".into(), places.display().to_string());
        let ctx = IngestContext {
            store: store.clone(),
            since: None,
            options: opts,
        };
        let report = FirefoxIngester.ingest(&ctx).await.unwrap();
        assert_eq!(report.inserted, 2, "{:?}", report);

        let hits = store.search("postgres tuning", 5, None).unwrap();
        assert!(hits.iter().any(|h| h.memory.source == "firefox"));
    }
}
