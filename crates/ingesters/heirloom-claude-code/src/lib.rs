//! # heirloom-claude-code
//!
//! Ingests Claude Code session transcripts from `~/.claude/projects/`. Claude Code
//! writes a JSONL file per session, one line per turn. We parse each turn into
//! a memory and attach the project path and session id as metadata.
//!
//! This is the direct equivalent of competing "claude-mem" projects — but where
//! they require an npm install, a worker process, and (often) external services
//! like Chroma or Supabase, this is just one Rust binary reading files you
//! already have.

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use directories::BaseDirs;
use heirloom_core::Memory;
use heirloom_ingester::{IngestContext, IngestReport, Ingester};
use serde::Deserialize;
use std::path::{Path, PathBuf};
use tracing::{debug, warn};
use walkdir::WalkDir;

pub struct ClaudeCodeIngester;

#[derive(Debug, Deserialize)]
struct Turn {
    #[serde(default, rename = "type")]
    kind: Option<String>,
    #[serde(default)]
    role: Option<String>,
    #[serde(default)]
    content: Option<serde_json::Value>,
    #[serde(default)]
    text: Option<String>,
    #[serde(default)]
    timestamp: Option<String>,
    #[serde(default)]
    session_id: Option<String>,
    #[serde(default)]
    uuid: Option<String>,
    #[serde(default, rename = "cwd")]
    cwd: Option<String>,
}

#[async_trait]
impl Ingester for ClaudeCodeIngester {
    fn name(&self) -> &'static str {
        "claude-code"
    }

    fn description(&self) -> &'static str {
        "Reads Claude Code session transcripts from ~/.claude/projects/."
    }

    async fn ingest(&self, ctx: &IngestContext) -> anyhow::Result<IngestReport> {
        let configured = ctx.opt("root", "");
        let root: PathBuf = if configured.is_empty() {
            match default_claude_root() {
                Some(p) => p,
                None => {
                    anyhow::bail!("could not find ~/.claude/projects — pass --path to override")
                }
            }
        } else {
            PathBuf::from(configured)
        };
        if !root.exists() {
            anyhow::bail!("claude-code root does not exist: {}", root.display());
        }
        let store = ctx.store.clone();
        let since = ctx.since;

        let report = tokio::task::spawn_blocking(move || -> anyhow::Result<IngestReport> {
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
                let ext = path.extension().and_then(|s| s.to_str()).unwrap_or("");
                if !matches!(ext, "jsonl" | "json") {
                    continue;
                }

                match parse_session_file(path, since, store.as_ref(), &mut batch, &mut report) {
                    Ok(()) => {}
                    Err(e) => {
                        debug!("failed to parse {}: {}", path.display(), e);
                        report.errors += 1;
                    }
                }

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
        })
        .await??;
        Ok(report)
    }
}

fn default_claude_root() -> Option<PathBuf> {
    let base = BaseDirs::new()?;
    let p = base.home_dir().join(".claude").join("projects");
    if p.exists() {
        Some(p)
    } else {
        None
    }
}

fn parse_session_file(
    path: &Path,
    since: Option<DateTime<Utc>>,
    _store: &heirloom_core::Store,
    batch: &mut Vec<Memory>,
    report: &mut IngestReport,
) -> anyhow::Result<()> {
    let raw = std::fs::read_to_string(path)?;
    let project_hint = project_hint_from_path(path);

    // Files can be either JSONL (one Turn per line) or a single JSON array.
    let lines: Vec<&str> = if raw.trim_start().starts_with('[') {
        // JSON array form — parse once, push items.
        let arr: serde_json::Value = serde_json::from_str(&raw)?;
        if let Some(items) = arr.as_array() {
            for item in items {
                if let Ok(turn) = serde_json::from_value::<Turn>(item.clone()) {
                    push_turn(turn, &project_hint, since, batch, report);
                }
            }
        }
        return Ok(());
    } else {
        raw.lines().collect()
    };

    for line in lines {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let turn: Turn = match serde_json::from_str(line) {
            Ok(t) => t,
            Err(_) => {
                report.errors += 1;
                continue;
            }
        };
        push_turn(turn, &project_hint, since, batch, report);
    }
    Ok(())
}

fn push_turn(
    turn: Turn,
    project_hint: &str,
    since: Option<DateTime<Utc>>,
    batch: &mut Vec<Memory>,
    report: &mut IngestReport,
) {
    report.scanned += 1;
    let role = turn
        .role
        .clone()
        .or(turn.kind.clone())
        .unwrap_or_else(|| "turn".into());
    if matches!(
        role.as_str(),
        "tool_use" | "tool_result" | "system" | "meta"
    ) {
        report.skipped += 1;
        return;
    }
    let content = extract_text(&turn);
    if content.trim().is_empty() {
        report.skipped += 1;
        return;
    }
    let created = turn
        .timestamp
        .as_deref()
        .and_then(|s| DateTime::parse_from_rfc3339(s).ok())
        .map(|d| d.with_timezone(&Utc))
        .unwrap_or_else(Utc::now);
    if let Some(s) = since {
        if created < s {
            report.skipped += 1;
            return;
        }
    }

    let mut m = Memory::new("claude-code", role, &content);
    m.created_at = created;
    if let Some(sid) = turn.session_id {
        m.metadata.insert("session_id".into(), sid);
    }
    if let Some(uid) = turn.uuid {
        m.metadata.insert("turn_id".into(), uid);
    }
    if let Some(cwd) = turn.cwd {
        m.metadata.insert("cwd".into(), cwd);
    }
    if !project_hint.is_empty() {
        m.metadata
            .insert("project".into(), project_hint.to_string());
    }
    batch.push(m);
}

fn extract_text(turn: &Turn) -> String {
    if let Some(t) = &turn.text {
        if !t.is_empty() {
            return t.clone();
        }
    }
    if let Some(content) = &turn.content {
        if let Some(s) = content.as_str() {
            return s.to_string();
        }
        if let Some(arr) = content.as_array() {
            let mut out = String::new();
            for block in arr {
                if let Some(text) = block.get("text").and_then(|t| t.as_str()) {
                    if !out.is_empty() {
                        out.push('\n');
                    }
                    out.push_str(text);
                }
            }
            return out;
        }
    }
    String::new()
}

fn project_hint_from_path(path: &Path) -> String {
    // ~/.claude/projects/<encoded-project-path>/<sessionid>.jsonl
    path.parent()
        .and_then(|p| p.file_name())
        .and_then(|s| s.to_str())
        .map(|s| s.replace('-', "/"))
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use heirloom_core::Store;
    use std::sync::Arc;

    #[tokio::test]
    async fn ingests_jsonl_session() {
        let tmp = tempfile::tempdir().unwrap();
        let session_dir = tmp.path().join("-Users-me-project-x");
        std::fs::create_dir_all(&session_dir).unwrap();
        let session = session_dir.join("abc123.jsonl");
        std::fs::write(
            &session,
            r#"{"type":"user","role":"user","text":"refactor the auth flow","timestamp":"2026-05-01T10:00:00Z","session_id":"abc123","uuid":"t1","cwd":"/Users/me/project-x"}
{"type":"assistant","role":"assistant","content":[{"type":"text","text":"I'll split auth into a PKCE-aware module."}],"timestamp":"2026-05-01T10:00:30Z","session_id":"abc123","uuid":"t2"}
{"type":"tool_use","role":"tool_use","content":"...","timestamp":"2026-05-01T10:00:35Z"}
"#,
        )
        .unwrap();

        let store = Arc::new(Store::in_memory().unwrap());
        let mut opts = std::collections::HashMap::new();
        opts.insert("root".into(), tmp.path().display().to_string());
        let ctx = IngestContext {
            store: store.clone(),
            since: None,
            options: opts,
        };

        let report = ClaudeCodeIngester.ingest(&ctx).await.unwrap();
        assert_eq!(report.inserted, 2, "{:?}", report);
        let hits = store.search("PKCE auth", 5, None).unwrap();
        assert!(hits.iter().any(|h| h.memory.source == "claude-code"));
        let assistant = hits
            .iter()
            .find(|h| h.memory.source == "claude-code")
            .unwrap();
        assert_eq!(
            assistant.memory.metadata.get("session_id").unwrap(),
            "abc123"
        );
        assert_eq!(
            assistant.memory.metadata.get("project").unwrap(),
            "/Users/me/project/x"
        );
    }
}
