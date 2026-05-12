//! # heirloom-slack
//!
//! Parses a Slack workspace export. Point it at the *unzipped* export
//! directory; each channel is a subdirectory of dated JSON files, and a
//! top-level `users.json` maps user IDs to display names.
//!
//! ## Example
//!
//! ```bash
//! unzip workspace-export.zip -d slack-export
//! heirloom ingest slack --path ./slack-export
//! ```

use async_trait::async_trait;
use chrono::{TimeZone, Utc};
use heirloom_core::Memory;
use heirloom_ingester::{IngestContext, IngestReport, Ingester};
use serde::Deserialize;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use tracing::warn;
use walkdir::WalkDir;

pub struct SlackIngester;

#[derive(Debug, Deserialize)]
struct UserRecord {
    id: String,
    #[serde(default)]
    real_name: Option<String>,
    #[serde(default)]
    name: Option<String>,
}

#[derive(Debug, Deserialize)]
struct Message {
    #[serde(default, rename = "type")]
    kind: Option<String>,
    #[serde(default)]
    user: Option<String>,
    #[serde(default)]
    text: Option<String>,
    #[serde(default)]
    ts: Option<String>,
    #[serde(default)]
    thread_ts: Option<String>,
    #[serde(default)]
    subtype: Option<String>,
}

#[async_trait]
impl Ingester for SlackIngester {
    fn name(&self) -> &'static str {
        "slack"
    }

    fn description(&self) -> &'static str {
        "Parses a Slack workspace export (point at the unzipped directory)."
    }

    async fn ingest(&self, ctx: &IngestContext) -> anyhow::Result<IngestReport> {
        let root: PathBuf = ctx.opt("path", "").into();
        if root.as_os_str().is_empty() || !root.exists() {
            anyhow::bail!("slack ingester requires --path to an unzipped export directory");
        }
        let store = ctx.store.clone();

        let report = tokio::task::spawn_blocking(move || -> anyhow::Result<IngestReport> {
            let users = load_users(&root).unwrap_or_default();
            let mut report = IngestReport::default();
            let mut batch: Vec<Memory> = Vec::with_capacity(256);

            for entry in WalkDir::new(&root).follow_links(false) {
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
                if path.extension().and_then(|s| s.to_str()) != Some("json") {
                    continue;
                }
                if path.file_name().and_then(|s| s.to_str()) == Some("users.json") {
                    continue;
                }
                if path.file_name().and_then(|s| s.to_str()) == Some("channels.json") {
                    continue;
                }
                let channel = path
                    .parent()
                    .and_then(|p| p.file_name())
                    .and_then(|s| s.to_str())
                    .unwrap_or("unknown")
                    .to_string();

                let raw = match std::fs::read_to_string(path) {
                    Ok(s) => s,
                    Err(_) => {
                        report.errors += 1;
                        continue;
                    }
                };
                let messages: Vec<Message> = match serde_json::from_str(&raw) {
                    Ok(v) => v,
                    Err(_) => {
                        report.errors += 1;
                        continue;
                    }
                };
                for m in messages {
                    report.scanned += 1;
                    if m.kind.as_deref() != Some("message") {
                        report.skipped += 1;
                        continue;
                    }
                    if m.subtype.as_deref() == Some("bot_message")
                        || m.subtype.as_deref() == Some("channel_join")
                        || m.subtype.as_deref() == Some("channel_leave")
                    {
                        report.skipped += 1;
                        continue;
                    }
                    let text = match m.text {
                        Some(t) if !t.trim().is_empty() => t,
                        _ => {
                            report.skipped += 1;
                            continue;
                        }
                    };
                    let author = m
                        .user
                        .as_deref()
                        .and_then(|uid| users.get(uid).cloned())
                        .unwrap_or_else(|| m.user.clone().unwrap_or_else(|| "unknown".into()));
                    let ts = m.ts.as_deref().and_then(parse_ts).unwrap_or_else(Utc::now);

                    let display = format!("{author} in #{channel}: {text}");
                    let mut mem = Memory::new("slack", "message", &display);
                    mem.created_at = ts;
                    mem.metadata.insert("channel".into(), channel.clone());
                    mem.metadata.insert("author".into(), author);
                    if let Some(thread) = m.thread_ts.clone() {
                        mem.metadata.insert("thread_ts".into(), thread);
                    }
                    batch.push(mem);

                    if batch.len() >= 256 {
                        let inserted = store.add_many(&batch)? as u64;
                        report.inserted += inserted;
                        report.skipped += batch.len() as u64 - inserted;
                        batch.clear();
                    }
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

fn load_users(root: &Path) -> Option<HashMap<String, String>> {
    let path = root.join("users.json");
    let raw = std::fs::read_to_string(path).ok()?;
    let records: Vec<UserRecord> = serde_json::from_str(&raw).ok()?;
    let mut map = HashMap::new();
    for r in records {
        let display = r.real_name.or(r.name).unwrap_or_else(|| r.id.clone());
        map.insert(r.id, display);
    }
    Some(map)
}

fn parse_ts(s: &str) -> Option<chrono::DateTime<Utc>> {
    // Slack timestamps look like "1700000000.000200" — Unix seconds.
    let secs = s.split('.').next()?.parse::<i64>().ok()?;
    Utc.timestamp_opt(secs, 0).single()
}

#[cfg(test)]
mod tests {
    use super::*;
    use heirloom_core::Store;
    use std::sync::Arc;

    #[tokio::test]
    async fn ingests_slack_export() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(
            tmp.path().join("users.json"),
            r#"[{"id":"U1","real_name":"Sam Chen"},{"id":"U2","name":"priya"}]"#,
        )
        .unwrap();
        let general = tmp.path().join("general");
        std::fs::create_dir_all(&general).unwrap();
        std::fs::write(
            general.join("2026-04-12.json"),
            r#"[
                {"type":"message","user":"U1","text":"shipping the dashboard Friday","ts":"1712923200.000100"},
                {"type":"message","user":"U2","text":"reviewing the OAuth PKCE refactor","ts":"1712923300.000100"},
                {"type":"message","user":"U1","text":"","ts":"1712923400.000100"}
            ]"#,
        )
        .unwrap();
        let store = Arc::new(Store::in_memory().unwrap());
        let mut opts = std::collections::HashMap::new();
        opts.insert("path".into(), tmp.path().display().to_string());
        let ctx = IngestContext {
            store: store.clone(),
            since: None,
            options: opts,
        };
        let report = SlackIngester.ingest(&ctx).await.unwrap();
        assert_eq!(report.inserted, 2, "{:?}", report);
        let hits = store.search("PKCE OAuth", 5, None).unwrap();
        let priya_hit = hits
            .iter()
            .find(|h| h.memory.metadata.get("author") == Some(&"priya".to_string()))
            .unwrap();
        assert_eq!(priya_hit.memory.metadata.get("channel").unwrap(), "general");
    }
}
