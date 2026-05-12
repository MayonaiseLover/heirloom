//! # heirloom-claude
//!
//! Parses Anthropic Claude conversation exports — the `conversations.json`
//! file you can download from `claude.ai → Settings → Export data`.
//!
//! Each *message* (one turn) becomes a separate memory. The conversation id
//! and title are attached as metadata so you can group by conversation later.

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use heirloom_core::Memory;
use heirloom_ingester::{IngestContext, IngestReport, Ingester};
use serde::Deserialize;
use std::path::PathBuf;


pub struct ClaudeIngester;

#[derive(Debug, Deserialize)]
struct Conversation {
    #[serde(default)]
    uuid: Option<String>,
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    created_at: Option<String>,
    #[serde(default, rename = "chat_messages")]
    messages: Vec<Message>,
}

#[derive(Debug, Deserialize)]
struct Message {
    #[serde(default)]
    uuid: Option<String>,
    #[serde(default)]
    sender: Option<String>,
    #[serde(default)]
    text: Option<String>,
    /// Newer exports include a structured `content` array.
    #[serde(default)]
    content: Option<serde_json::Value>,
    #[serde(default)]
    created_at: Option<String>,
}

#[async_trait]
impl Ingester for ClaudeIngester {
    fn name(&self) -> &'static str {
        "claude"
    }

    fn description(&self) -> &'static str {
        "Parses Claude conversation exports (conversations.json)."
    }

    async fn ingest(&self, ctx: &IngestContext) -> anyhow::Result<IngestReport> {
        let path: PathBuf = ctx.opt("path", "").into();
        if path.as_os_str().is_empty() {
            anyhow::bail!("claude ingester requires --path to a conversations.json export");
        }
        if !path.exists() {
            anyhow::bail!("file does not exist: {}", path.display());
        }
        let store = ctx.store.clone();
        let since = ctx.since;

        let report = tokio::task::spawn_blocking(move || -> anyhow::Result<IngestReport> {
            let raw = std::fs::read_to_string(&path)?;
            let conversations: Vec<Conversation> = serde_json::from_str(&raw)?;
            let mut report = IngestReport::default();
            let mut batch: Vec<Memory> = Vec::with_capacity(256);

            for conv in conversations {
                let conv_title = conv.name.clone().unwrap_or_default();
                let conv_id = conv.uuid.clone().unwrap_or_default();

                for msg in conv.messages {
                    report.scanned += 1;
                    let content = extract_text(&msg);
                    if content.trim().is_empty() {
                        report.skipped += 1;
                        continue;
                    }

                    let created = parse_dt(msg.created_at.as_deref())
                        .or_else(|| parse_dt(conv.created_at.as_deref()))
                        .unwrap_or_else(Utc::now);

                    if let Some(s) = since {
                        if created < s {
                            report.skipped += 1;
                            continue;
                        }
                    }

                    let kind = msg.sender.clone().unwrap_or_else(|| "message".into());
                    let mut m = Memory::new("claude", kind, &content);
                    m.created_at = created;
                    if !conv_id.is_empty() {
                        m.metadata.insert("conversation_id".into(), conv_id.clone());
                    }
                    if !conv_title.is_empty() {
                        m.metadata.insert("title".into(), conv_title.clone());
                    }
                    if let Some(id) = &msg.uuid {
                        m.metadata.insert("message_id".into(), id.clone());
                    }
                    batch.push(m);

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

fn extract_text(msg: &Message) -> String {
    if let Some(t) = &msg.text {
        if !t.is_empty() {
            return t.clone();
        }
    }
    if let Some(content) = &msg.content {
        return walk_content(content);
    }
    String::new()
}

/// Walk newer Claude export shape: `content` is an array of `{type, text}` blocks.
fn walk_content(v: &serde_json::Value) -> String {
    let mut out = String::new();
    if let Some(arr) = v.as_array() {
        for block in arr {
            if let Some(text) = block.get("text").and_then(|t| t.as_str()) {
                if !out.is_empty() {
                    out.push('\n');
                }
                out.push_str(text);
            }
        }
    } else if let Some(s) = v.as_str() {
        out.push_str(s);
    }
    out
}

fn parse_dt(s: Option<&str>) -> Option<DateTime<Utc>> {
    let s = s?;
    DateTime::parse_from_rfc3339(s)
        .map(|d| d.with_timezone(&Utc))
        .ok()
        .or_else(|| {
            // Some exports use ISO without timezone.
            chrono::NaiveDateTime::parse_from_str(s, "%Y-%m-%dT%H:%M:%S%.f")
                .ok()
                .map(|n| Utc.from_utc_datetime(&n))
        })
}

use chrono::TimeZone;

#[cfg(test)]
mod tests {
    use super::*;
    use heirloom_core::Store;
    use std::sync::Arc;

    const SAMPLE: &str = r#"[
        {
            "uuid": "conv-1",
            "name": "Architecture review",
            "created_at": "2026-04-12T10:00:00Z",
            "chat_messages": [
                {
                    "uuid": "msg-1",
                    "sender": "human",
                    "text": "I'm building with Rust, Postgres, and Tauri. What's a good error handling pattern?",
                    "created_at": "2026-04-12T10:01:00Z"
                },
                {
                    "uuid": "msg-2",
                    "sender": "assistant",
                    "content": [
                        {"type": "text", "text": "Use thiserror in libraries and anyhow in binaries."},
                        {"type": "text", "text": "This separates intentional error types from incidental ones."}
                    ],
                    "created_at": "2026-04-12T10:01:30Z"
                }
            ]
        }
    ]"#;

    #[tokio::test]
    async fn ingests_sample_export() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("conversations.json");
        std::fs::write(&path, SAMPLE).unwrap();

        let store = Arc::new(Store::in_memory().unwrap());
        let mut opts = std::collections::HashMap::new();
        opts.insert("path".into(), path.display().to_string());
        let ctx = IngestContext {
            store: store.clone(),
            since: None,
            options: opts,
        };

        let report = ClaudeIngester.ingest(&ctx).await.unwrap();
        assert_eq!(report.inserted, 2, "{:?}", report);
        let hits = store.search("thiserror anyhow", 5, None).unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].memory.source, "claude");
        assert_eq!(
            hits[0].memory.metadata.get("title").unwrap(),
            "Architecture review"
        );
    }
}
