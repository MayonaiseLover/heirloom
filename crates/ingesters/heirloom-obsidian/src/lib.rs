//! # heirloom-obsidian
//!
//! Walks an Obsidian vault and turns each note into a memory. Adds two
//! pieces of structure that the generic `fs` ingester does not:
//!
//! 1. **Frontmatter** (YAML between `---` fences) is parsed loosely and
//!    flattened into memory metadata.
//! 2. **Wikilinks** (`[[Note Name]]`) are extracted as a `links` metadata
//!    field, comma-separated.
//!
//! Hidden directories like `.obsidian` are skipped.

use async_trait::async_trait;
use heirloom_core::Memory;
use heirloom_ingester::{IngestContext, IngestReport, Ingester};
use std::path::PathBuf;
use tracing::warn;
use walkdir::WalkDir;

pub struct ObsidianIngester;

#[async_trait]
impl Ingester for ObsidianIngester {
    fn name(&self) -> &'static str {
        "obsidian"
    }

    fn description(&self) -> &'static str {
        "Walks an Obsidian vault and ingests notes with frontmatter and wikilink metadata."
    }

    async fn ingest(&self, ctx: &IngestContext) -> anyhow::Result<IngestReport> {
        let vault: PathBuf = ctx.opt("path", "").into();
        if vault.as_os_str().is_empty() || !vault.exists() {
            anyhow::bail!("obsidian ingester requires --path to a vault directory");
        }
        let store = ctx.store.clone();

        let report = tokio::task::spawn_blocking(move || -> anyhow::Result<IngestReport> {
            let mut report = IngestReport::default();
            let mut batch: Vec<Memory> = Vec::with_capacity(128);

            for entry in WalkDir::new(&vault)
                .follow_links(false)
                .into_iter()
                .filter_entry(|e| {
                    e.depth() == 0
                        || !e
                            .path()
                            .file_name()
                            .and_then(|n| n.to_str())
                            .map(|n| n.starts_with('.'))
                            .unwrap_or(false)
                })
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
                if entry.path().extension().and_then(|s| s.to_str()) != Some("md") {
                    continue;
                }
                let path = entry.path();
                let raw = match std::fs::read_to_string(path) {
                    Ok(s) => s,
                    Err(_) => {
                        report.errors += 1;
                        continue;
                    }
                };
                let body = raw.trim();
                if body.is_empty() {
                    report.skipped += 1;
                    continue;
                }
                report.scanned += 1;

                let (frontmatter, content) = split_frontmatter(body);
                let links = extract_wikilinks(content);

                let mut m = Memory::new("obsidian", "note", content);
                if let Some(name) = path.file_stem().and_then(|s| s.to_str()) {
                    m.metadata.insert("title".into(), name.to_string());
                }
                m.metadata.insert("path".into(), path.display().to_string());
                if !links.is_empty() {
                    m.metadata.insert("links".into(), links.join(", "));
                }
                for (k, v) in frontmatter {
                    m.metadata.insert(format!("fm:{}", k), v);
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

/// Loose YAML-frontmatter splitter — handles the common `key: value` shape.
/// Returns `(frontmatter_pairs, remaining_body)`. Falls back gracefully on
/// malformed frontmatter rather than failing the whole ingestion.
fn split_frontmatter(input: &str) -> (Vec<(String, String)>, &str) {
    if !input.starts_with("---") {
        return (Vec::new(), input);
    }
    let after_first = &input[3..];
    let end_idx = match after_first.find("\n---") {
        Some(i) => i,
        None => return (Vec::new(), input),
    };
    let block = &after_first[..end_idx];
    let mut pairs = Vec::new();
    for line in block.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        if let Some((k, v)) = line.split_once(':') {
            let key = k.trim().to_string();
            let val = v.trim().trim_matches(|c| c == '"' || c == '\'').to_string();
            if !key.is_empty() && !val.is_empty() {
                pairs.push((key, val));
            }
        }
    }
    // Skip past the closing fence + newline.
    let after_close = &after_first[end_idx + 4..];
    let body = after_close.trim_start_matches('\n');
    (pairs, body)
}

fn extract_wikilinks(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '[' && chars.peek() == Some(&'[') {
            chars.next();
            let mut link = String::new();
            for next in chars.by_ref() {
                if next == ']' {
                    break;
                }
                if link.len() > 200 {
                    break;
                }
                link.push(next);
            }
            // Strip alias (`Page|Alias`) and section (`Page#section`).
            let core: String = link
                .split('|')
                .next()
                .unwrap_or("")
                .split('#')
                .next()
                .unwrap_or("")
                .trim()
                .to_string();
            if !core.is_empty() && !out.contains(&core) {
                out.push(core);
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use heirloom_core::Store;
    use std::sync::Arc;

    #[tokio::test]
    async fn ingests_vault_with_frontmatter_and_links() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(
            tmp.path().join("Auth refactor.md"),
            "---\ntags: oauth, security\nstatus: in-progress\n---\n# Auth refactor\n\nReviewing with [[Sam Chen]] this week. See [[Q2 priorities]] for context.",
        )
        .unwrap();
        let hidden = tmp.path().join(".obsidian");
        std::fs::create_dir_all(&hidden).unwrap();
        std::fs::write(hidden.join("workspace.json"), "{}").unwrap();

        let store = Arc::new(Store::in_memory().unwrap());
        let mut opts = std::collections::HashMap::new();
        opts.insert("path".into(), tmp.path().display().to_string());
        let ctx = IngestContext {
            store: store.clone(),
            since: None,
            options: opts,
        };

        let report = ObsidianIngester.ingest(&ctx).await.unwrap();
        assert_eq!(report.inserted, 1, "{:?}", report);
        let hit = store
            .search("auth refactor", 5, None)
            .unwrap()
            .into_iter()
            .next()
            .unwrap();
        assert_eq!(
            hit.memory.metadata.get("fm:tags").unwrap(),
            "oauth, security"
        );
        let links = hit.memory.metadata.get("links").unwrap();
        assert!(links.contains("Sam Chen"));
        assert!(links.contains("Q2 priorities"));
    }

    #[test]
    fn frontmatter_split_handles_missing_fence() {
        let (fm, body) = split_frontmatter("# just a heading\n\ncontent");
        assert!(fm.is_empty());
        assert!(body.starts_with("# just a heading"));
    }

    #[test]
    fn wikilinks_strip_aliases_and_sections() {
        let links = extract_wikilinks("see [[Page|Alias]] and [[Other#section]]");
        assert_eq!(links, vec!["Page", "Other"]);
    }
}
