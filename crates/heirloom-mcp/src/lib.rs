//! # heirloom-mcp
//!
//! A minimal Model Context Protocol (MCP) server over stdio. Implements the
//! handshake (`initialize`), tool discovery (`tools/list`), and tool execution
//! (`tools/call`) message types from the MCP specification.
//!
//! Exposed tools:
//!
//! | name              | description                                    |
//! |-------------------|------------------------------------------------|
//! | `search_memory`   | Full-text search over the user's memory store  |
//! | `recent_memories` | Newest memories, optionally filtered by source |
//! | `list_sources`    | Distinct sources present in the store          |
//! | `get_memory`      | Fetch a single memory by id                    |
//!
//! ## Protocol
//!
//! Each message is a single line of JSON-RPC 2.0 on stdout. Requests arrive
//! as single lines on stdin. The server reads, dispatches, writes, repeats.

use anyhow::Result;
use chrono::DateTime;
use heirloom_core::{SearchFilters, Store};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::sync::Arc;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tracing::{debug, error, info, warn};

pub const SERVER_NAME: &str = "heirloom";
pub const SERVER_VERSION: &str = env!("CARGO_PKG_VERSION");
pub const PROTOCOL_VERSION: &str = "2024-11-05";

/// Run the MCP server. Reads JSON-RPC frames from stdin and writes responses to stdout.
pub async fn serve_stdio(store: Arc<Store>) -> Result<()> {
    let stdin = tokio::io::stdin();
    let mut stdout = tokio::io::stdout();
    let mut reader = BufReader::new(stdin).lines();

    info!("heirloom MCP server starting on stdio");

    while let Some(line) = reader.next_line().await? {
        if line.trim().is_empty() {
            continue;
        }
        debug!(?line, "received");

        let response = match serde_json::from_str::<Request>(&line) {
            Ok(req) => handle_request(req, &store).await,
            Err(e) => {
                error!("parse error: {}", e);
                Some(Response::error(
                    Value::Null,
                    -32700,
                    format!("parse error: {}", e),
                ))
            }
        };

        if let Some(resp) = response {
            let out = serde_json::to_string(&resp)?;
            stdout.write_all(out.as_bytes()).await?;
            stdout.write_all(b"\n").await?;
            stdout.flush().await?;
        }
    }

    info!("heirloom MCP server stdin closed; exiting");
    Ok(())
}

#[derive(Debug, Deserialize)]
struct Request {
    #[allow(dead_code)]
    jsonrpc: String,
    #[serde(default)]
    id: Value,
    method: String,
    #[serde(default)]
    params: Value,
}

#[derive(Debug, Serialize)]
struct Response {
    jsonrpc: &'static str,
    id: Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    result: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<RpcError>,
}

#[derive(Debug, Serialize)]
struct RpcError {
    code: i32,
    message: String,
}

impl Response {
    fn ok(id: Value, result: Value) -> Self {
        Self {
            jsonrpc: "2.0",
            id,
            result: Some(result),
            error: None,
        }
    }
    fn error(id: Value, code: i32, message: String) -> Self {
        Self {
            jsonrpc: "2.0",
            id,
            result: None,
            error: Some(RpcError { code, message }),
        }
    }
}

async fn handle_request(req: Request, store: &Arc<Store>) -> Option<Response> {
    // Notifications (no id) get no response.
    let is_notification = req.id.is_null();

    let result = match req.method.as_str() {
        "initialize" => Ok(handle_initialize()),
        "initialized" | "notifications/initialized" => {
            // Client signaled it finished setup. No response.
            return None;
        }
        "ping" => Ok(json!({})),
        "tools/list" => Ok(handle_tools_list()),
        "tools/call" => handle_tools_call(req.params, store).await,
        "resources/list" => Ok(json!({ "resources": [] })),
        "prompts/list" => Ok(json!({ "prompts": [] })),
        other => Err(anyhow::anyhow!("unknown method: {}", other)),
    };

    if is_notification {
        return None;
    }

    Some(match result {
        Ok(value) => Response::ok(req.id, value),
        Err(e) => {
            warn!("request {} failed: {}", req.method, e);
            Response::error(req.id, -32603, e.to_string())
        }
    })
}

fn handle_initialize() -> Value {
    json!({
        "protocolVersion": PROTOCOL_VERSION,
        "serverInfo": {
            "name": SERVER_NAME,
            "version": SERVER_VERSION
        },
        "capabilities": {
            "tools": {}
        }
    })
}

fn handle_tools_list() -> Value {
    json!({
        "tools": [
            {
                "name": "search_memory",
                "description": "Search the user's personal memory using full-text search. Returns matching memories ranked by relevance. Use this whenever the user refers to something from their past — projects, conversations, files, decisions.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "query": {
                            "type": "string",
                            "description": "Free-text search query. Supports natural language."
                        },
                        "k": {
                            "type": "integer",
                            "description": "Max number of results to return. Default 10.",
                            "default": 10,
                            "minimum": 1,
                            "maximum": 50
                        },
                        "sources": {
                            "type": "array",
                            "items": { "type": "string" },
                            "description": "Optional filter — restrict to these source tags (e.g. 'fs', 'browser', 'claude')."
                        },
                        "since": {
                            "type": "string",
                            "description": "Optional RFC3339 timestamp. Only memories at or after this time."
                        },
                        "until": {
                            "type": "string",
                            "description": "Optional RFC3339 timestamp. Only memories at or before this time."
                        }
                    },
                    "required": ["query"]
                }
            },
            {
                "name": "recent_memories",
                "description": "List the user's most recent memories, optionally filtered by source.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "source": { "type": "string", "description": "Optional source tag filter." },
                        "limit": { "type": "integer", "default": 20, "minimum": 1, "maximum": 100 }
                    }
                }
            },
            {
                "name": "list_sources",
                "description": "List all source tags currently present in the user's memory store, with counts.",
                "inputSchema": { "type": "object", "properties": {} }
            },
            {
                "name": "get_memory",
                "description": "Fetch a single memory by id.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "id": { "type": "string", "description": "Memory id (UUID)." }
                    },
                    "required": ["id"]
                }
            },
            {
                "name": "add_memory",
                "description": "Record a new memory in the user's local store. Use this when the user shares a preference, decision, fact, or anything they want remembered across conversations. Be conservative — only record things the user would actually want persisted. Always set `source` to 'agent' so memories you write are clearly distinguishable from ingested ones.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "content": {
                            "type": "string",
                            "description": "The text to remember. Write it as a third-person statement (e.g. 'User prefers dark mode in their editor')."
                        },
                        "source": {
                            "type": "string",
                            "description": "Source tag — should be 'agent' for memories written by an AI.",
                            "default": "agent"
                        },
                        "kind": {
                            "type": "string",
                            "description": "Subtype (e.g. 'preference', 'decision', 'fact').",
                            "default": "note"
                        }
                    },
                    "required": ["content"]
                }
            }
        ]
    })
}

async fn handle_tools_call(params: Value, store: &Arc<Store>) -> Result<Value> {
    let name = params
        .get("name")
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow::anyhow!("missing tool name"))?;
    let args = params
        .get("arguments")
        .cloned()
        .unwrap_or_else(|| json!({}));

    let payload = match name {
        "search_memory" => tool_search_memory(args, store)?,
        "recent_memories" => tool_recent(args, store)?,
        "list_sources" => tool_list_sources(store)?,
        "get_memory" => tool_get_memory(args, store)?,
        "add_memory" => tool_add_memory(args, store)?,
        other => anyhow::bail!("unknown tool: {}", other),
    };

    Ok(json!({
        "content": [
            { "type": "text", "text": serde_json::to_string_pretty(&payload)? }
        ]
    }))
}

fn tool_search_memory(args: Value, store: &Arc<Store>) -> Result<Value> {
    let query = args
        .get("query")
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow::anyhow!("missing query"))?;
    let k = args.get("k").and_then(|v| v.as_u64()).unwrap_or(10) as usize;

    let mut filters = SearchFilters::default();
    if let Some(arr) = args.get("sources").and_then(|v| v.as_array()) {
        let v: Vec<String> = arr
            .iter()
            .filter_map(|x| x.as_str().map(String::from))
            .collect();
        if !v.is_empty() {
            filters.sources = Some(v);
        }
    }
    if let Some(s) = args.get("since").and_then(|v| v.as_str()) {
        filters.since = DateTime::parse_from_rfc3339(s)
            .ok()
            .map(|d| d.with_timezone(&chrono::Utc));
    }
    if let Some(s) = args.get("until").and_then(|v| v.as_str()) {
        filters.until = DateTime::parse_from_rfc3339(s)
            .ok()
            .map(|d| d.with_timezone(&chrono::Utc));
    }

    let results = store.search(query, k, Some(filters))?;
    Ok(json!({
        "query": query,
        "count": results.len(),
        "results": results.iter().map(|r| json!({
            "id": r.memory.id,
            "source": r.memory.source,
            "kind": r.memory.kind,
            "snippet": r.snippet,
            "content": truncate(&r.memory.content, 600),
            "metadata": r.memory.metadata,
            "created_at": r.memory.created_at.to_rfc3339(),
            "score": r.score,
        })).collect::<Vec<_>>()
    }))
}

fn tool_recent(args: Value, store: &Arc<Store>) -> Result<Value> {
    let source = args.get("source").and_then(|v| v.as_str());
    let limit = args.get("limit").and_then(|v| v.as_u64()).unwrap_or(20) as usize;
    let memories = store.recent(source, limit)?;
    Ok(json!({
        "count": memories.len(),
        "memories": memories.iter().map(|m| json!({
            "id": m.id,
            "source": m.source,
            "kind": m.kind,
            "content": truncate(&m.content, 400),
            "metadata": m.metadata,
            "created_at": m.created_at.to_rfc3339(),
        })).collect::<Vec<_>>()
    }))
}

fn tool_list_sources(store: &Arc<Store>) -> Result<Value> {
    let sources = store.sources()?;
    Ok(json!({
        "sources": sources.iter().map(|(s, n)| json!({ "name": s, "count": n })).collect::<Vec<_>>(),
        "total": store.count()?
    }))
}

fn tool_get_memory(args: Value, store: &Arc<Store>) -> Result<Value> {
    let id = args
        .get("id")
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow::anyhow!("missing id"))?;
    match store.get(id)? {
        Some(m) => Ok(json!({
            "id": m.id,
            "source": m.source,
            "kind": m.kind,
            "content": m.content,
            "metadata": m.metadata,
            "created_at": m.created_at.to_rfc3339(),
        })),
        None => Ok(json!({ "error": "not found", "id": id })),
    }
}

fn tool_add_memory(args: Value, store: &Arc<Store>) -> Result<Value> {
    let content = args
        .get("content")
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow::anyhow!("missing content"))?;
    let content = content.trim();
    if content.is_empty() {
        anyhow::bail!("empty content");
    }
    let source = args
        .get("source")
        .and_then(|v| v.as_str())
        .unwrap_or("agent");
    let kind = args.get("kind").and_then(|v| v.as_str()).unwrap_or("note");
    let memory = heirloom_core::Memory::new(source, kind, content);
    let inserted = store.add(&memory)?;
    Ok(json!({
        "id": memory.id,
        "inserted": inserted,
        "source": memory.source,
        "kind": memory.kind,
        "note": if inserted { "memory stored" } else { "duplicate — already in store" }
    }))
}

fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_string()
    } else {
        let mut out: String = s.chars().take(max).collect();
        out.push('…');
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use heirloom_core::Memory;

    fn store_with_seed() -> Arc<Store> {
        let store = Arc::new(Store::in_memory().unwrap());
        store
            .add(&Memory::new(
                "fs",
                "note",
                "buy oat milk and ginger tea tomorrow",
            ))
            .unwrap();
        store
            .add(&Memory::new(
                "browser",
                "page",
                "rust async traits stable release notes",
            ))
            .unwrap();
        store
    }

    #[tokio::test]
    async fn initialize_returns_capabilities() {
        let store = store_with_seed();
        let req = Request {
            jsonrpc: "2.0".into(),
            id: json!(1),
            method: "initialize".into(),
            params: json!({}),
        };
        let resp = handle_request(req, &store).await.unwrap();
        let v = serde_json::to_value(resp).unwrap();
        assert_eq!(v["result"]["serverInfo"]["name"], "heirloom");
    }

    #[tokio::test]
    async fn tools_list_includes_search_memory() {
        let store = store_with_seed();
        let req = Request {
            jsonrpc: "2.0".into(),
            id: json!(2),
            method: "tools/list".into(),
            params: json!({}),
        };
        let resp = handle_request(req, &store).await.unwrap();
        let v = serde_json::to_value(resp).unwrap();
        let names: Vec<String> = v["result"]["tools"]
            .as_array()
            .unwrap()
            .iter()
            .map(|t| t["name"].as_str().unwrap().to_string())
            .collect();
        assert!(names.contains(&"search_memory".to_string()));
        assert!(names.contains(&"recent_memories".to_string()));
    }

    #[tokio::test]
    async fn search_memory_tool_returns_results() {
        let store = store_with_seed();
        let req = Request {
            jsonrpc: "2.0".into(),
            id: json!(3),
            method: "tools/call".into(),
            params: json!({
                "name": "search_memory",
                "arguments": { "query": "ginger", "k": 5 }
            }),
        };
        let resp = handle_request(req, &store).await.unwrap();
        let v = serde_json::to_value(resp).unwrap();
        let text = v["result"]["content"][0]["text"].as_str().unwrap();
        let parsed: Value = serde_json::from_str(text).unwrap();
        assert_eq!(parsed["count"], 1);
    }

    #[tokio::test]
    async fn unknown_tool_returns_error() {
        let store = store_with_seed();
        let req = Request {
            jsonrpc: "2.0".into(),
            id: json!(4),
            method: "tools/call".into(),
            params: json!({ "name": "no_such_tool", "arguments": {} }),
        };
        let resp = handle_request(req, &store).await.unwrap();
        let v = serde_json::to_value(resp).unwrap();
        assert!(v["error"].is_object());
    }

    #[tokio::test]
    async fn notification_returns_no_response() {
        let store = store_with_seed();
        let req = Request {
            jsonrpc: "2.0".into(),
            id: Value::Null,
            method: "notifications/initialized".into(),
            params: json!({}),
        };
        let resp = handle_request(req, &store).await;
        assert!(resp.is_none());
    }
}
