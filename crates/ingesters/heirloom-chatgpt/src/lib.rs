//! # heirloom-chatgpt
//!
//! Parses OpenAI ChatGPT conversation exports — the `conversations.json` file
//! in the export ZIP you can request from `chatgpt.com → Settings → Data controls
//! → Export data`.
//!
//! ChatGPT exports are tree-shaped (each message points to its parent), but for
//! ingestion we flatten them to chronological turns. The tree's `current_node`
//! is the leaf of the active branch — we walk back up from it.

use async_trait::async_trait;
use chrono::{TimeZone, Utc};
use heirloom_core::Memory;
use heirloom_ingester::{IngestContext, IngestReport, Ingester};
use serde::Deserialize;
use std::collections::HashMap;
use std::path::PathBuf;

pub struct ChatGPTIngester;

#[derive(Debug, Deserialize)]
struct Conversation {
    #[serde(default)]
    id: Option<String>,
    #[serde(default)]
    title: Option<String>,
    #[serde(default)]
    create_time: Option<f64>,
    mapping: HashMap<String, Node>,
    #[serde(default)]
    current_node: Option<String>,
}

#[derive(Debug, Deserialize)]
struct Node {
    #[serde(default)]
    parent: Option<String>,
    #[serde(default)]
    message: Option<MessageBody>,
}

#[derive(Debug, Deserialize)]
struct MessageBody {
    #[serde(default)]
    id: Option<String>,
    #[serde(default)]
    create_time: Option<f64>,
    #[serde(default)]
    author: Option<Author>,
    #[serde(default)]
    content: Option<Content>,
}

#[derive(Debug, Deserialize)]
struct Author {
    #[serde(default)]
    role: Option<String>,
}

#[derive(Debug, Deserialize)]
struct Content {
    #[allow(dead_code)]
    #[serde(default)]
    content_type: Option<String>,
    #[serde(default)]
    parts: Option<Vec<serde_json::Value>>,
}

#[async_trait]
impl Ingester for ChatGPTIngester {
    fn name(&self) -> &'static str {
        "chatgpt"
    }

    fn description(&self) -> &'static str {
        "Parses ChatGPT conversation exports (conversations.json from the export ZIP)."
    }

    async fn ingest(&self, ctx: &IngestContext) -> anyhow::Result<IngestReport> {
        let path: PathBuf = ctx.opt("path", "").into();
        if path.as_os_str().is_empty() {
            anyhow::bail!("chatgpt ingester requires --path to conversations.json");
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
                let conv_id = conv.id.clone().unwrap_or_default();
                let conv_title = conv.title.clone().unwrap_or_default();
                let ordered_ids = order_messages(&conv);

                for node_id in ordered_ids {
                    let node = match conv.mapping.get(&node_id) {
                        Some(n) => n,
                        None => continue,
                    };
                    let body = match &node.message {
                        Some(b) => b,
                        None => continue,
                    };
                    report.scanned += 1;

                    let role = body
                        .author
                        .as_ref()
                        .and_then(|a| a.role.clone())
                        .unwrap_or_else(|| "message".into());
                    if role == "system" || role == "tool" {
                        // These don't carry user-relevant memory.
                        report.skipped += 1;
                        continue;
                    }

                    let text = extract_text(body.content.as_ref());
                    if text.trim().is_empty() {
                        report.skipped += 1;
                        continue;
                    }

                    let created = body
                        .create_time
                        .or(conv.create_time)
                        .and_then(|ts| Utc.timestamp_opt(ts as i64, 0).single())
                        .unwrap_or_else(Utc::now);

                    if let Some(s) = since {
                        if created < s {
                            report.skipped += 1;
                            continue;
                        }
                    }

                    let mut m = Memory::new("chatgpt", role, &text);
                    m.created_at = created;
                    if !conv_id.is_empty() {
                        m.metadata.insert("conversation_id".into(), conv_id.clone());
                    }
                    if !conv_title.is_empty() {
                        m.metadata.insert("title".into(), conv_title.clone());
                    }
                    if let Some(id) = &body.id {
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

/// Walk back from `current_node` through parents to produce a root→leaf ordering.
fn order_messages(conv: &Conversation) -> Vec<String> {
    let mut chain = Vec::new();
    let Some(start) = conv.current_node.clone() else {
        // Fall back to mapping iteration order — better than nothing.
        return conv.mapping.keys().cloned().collect();
    };
    let mut cursor = Some(start);
    while let Some(id) = cursor {
        chain.push(id.clone());
        cursor = conv.mapping.get(&id).and_then(|n| n.parent.clone());
    }
    chain.reverse();
    chain
}

fn extract_text(content: Option<&Content>) -> String {
    let Some(c) = content else {
        return String::new();
    };
    let parts = match &c.parts {
        Some(p) => p,
        None => return String::new(),
    };
    let mut out = String::new();
    for part in parts {
        if let Some(s) = part.as_str() {
            if !out.is_empty() {
                out.push('\n');
            }
            out.push_str(s);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use heirloom_core::Store;
    use std::sync::Arc;

    const SAMPLE: &str = r#"[
        {
            "id": "conv-1",
            "title": "Project bootstrap",
            "create_time": 1700000000.0,
            "current_node": "n3",
            "mapping": {
                "n1": {
                    "parent": null,
                    "message": {
                        "id": "m1",
                        "create_time": 1700000001.0,
                        "author": {"role": "user"},
                        "content": {"content_type": "text", "parts": ["What's a good architecture for a Tauri + Rust + Postgres app?"]}
                    }
                },
                "n2": {
                    "parent": "n1",
                    "message": {
                        "id": "m2",
                        "create_time": 1700000002.0,
                        "author": {"role": "assistant"},
                        "content": {"content_type": "text", "parts": ["Split it into core, service, and frontend crates with clear API boundaries."]}
                    }
                },
                "n3": {
                    "parent": "n2",
                    "message": {
                        "id": "m3",
                        "create_time": 1700000003.0,
                        "author": {"role": "system"},
                        "content": {"content_type": "text", "parts": ["You are a helpful assistant."]}
                    }
                }
            }
        }
    ]"#;

    #[tokio::test]
    async fn ingests_chatgpt_export() {
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

        let report = ChatGPTIngester.ingest(&ctx).await.unwrap();
        // user + assistant; system filtered.
        assert_eq!(report.inserted, 2, "{:?}", report);

        let hits = store.search("crates Tauri", 5, None).unwrap();
        assert!(hits.iter().any(|h| h.memory.source == "chatgpt"));
    }

    #[test]
    fn ordering_walks_back_from_current_node() {
        let convs: Vec<Conversation> = serde_json::from_str(SAMPLE).unwrap();
        let conv = &convs[0];
        let order = order_messages(conv);
        assert_eq!(order, vec!["n1", "n2", "n3"]);
    }
}
