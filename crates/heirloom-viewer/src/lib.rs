//! # heirloom-viewer
//!
//! A tiny self-contained HTTP server that exposes a local web viewer for
//! browsing and searching your Heirloom memory. Binds to `127.0.0.1` by
//! default — the viewer is for local use only.
//!
//! No web framework dependency. The HTTP parser is just enough to handle
//! the few routes we serve. Single-file HTML/CSS/JS embedded at compile time.

use anyhow::Result;
use heirloom_core::{SearchFilters, Store};
use serde_json::json;
use std::net::SocketAddr;
use std::sync::Arc;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tracing::{debug, info, warn};

const INDEX_HTML: &str = include_str!("index.html");

pub async fn serve(store: Arc<Store>, addr: SocketAddr) -> Result<()> {
    let listener = TcpListener::bind(addr).await?;
    info!("heirloom viewer listening on http://{}", addr);
    println!(
        "\n  Heirloom viewer running at \x1b[1;36mhttp://{}\x1b[0m\n  Press Ctrl-C to stop.\n",
        addr
    );
    loop {
        let (stream, peer) = match listener.accept().await {
            Ok(v) => v,
            Err(e) => {
                warn!("accept error: {}", e);
                continue;
            }
        };
        let store = store.clone();
        tokio::spawn(async move {
            if let Err(e) = handle(stream, store).await {
                debug!(?peer, "handler error: {}", e);
            }
        });
    }
}

async fn handle(mut stream: TcpStream, store: Arc<Store>) -> Result<()> {
    let mut buf = vec![0u8; 8192];
    let n = stream.read(&mut buf).await?;
    if n == 0 {
        return Ok(());
    }
    let req = String::from_utf8_lossy(&buf[..n]);
    let (method, path) = parse_request_line(&req);

    let response = if method == "GET" && path == "/" {
        ok_html(INDEX_HTML)
    } else if method == "GET" && path.starts_with("/api/search") {
        let query = extract_query(path, "q").unwrap_or_default();
        let k: usize = extract_query(path, "k")
            .and_then(|s| s.parse().ok())
            .unwrap_or(20);
        let source = extract_query(path, "source");
        let filters = source.map(|s| SearchFilters {
            sources: Some(vec![s]),
            ..Default::default()
        });
        match store.search(&query, k, filters) {
            Ok(results) => {
                let body = json!({
                    "query": query,
                    "count": results.len(),
                    "results": results.iter().map(|r| json!({
                        "id": r.memory.id,
                        "source": r.memory.source,
                        "kind": r.memory.kind,
                        "content": r.memory.content,
                        "snippet": r.snippet,
                        "metadata": r.memory.metadata,
                        "created_at": r.memory.created_at.to_rfc3339(),
                        "score": r.score,
                    })).collect::<Vec<_>>(),
                });
                ok_json(&body.to_string())
            }
            Err(e) => err_json(400, &format!("search failed: {}", e)),
        }
    } else if method == "GET" && path == "/api/sources" {
        match store.sources() {
            Ok(sources) => {
                let body = json!({
                    "sources": sources.iter().map(|(n, c)| json!({"name": n, "count": c})).collect::<Vec<_>>(),
                    "total": store.count().unwrap_or(0),
                });
                ok_json(&body.to_string())
            }
            Err(e) => err_json(500, &e.to_string()),
        }
    } else if method == "GET" && path.starts_with("/api/recent") {
        let limit: usize = extract_query(path, "limit")
            .and_then(|s| s.parse().ok())
            .unwrap_or(50);
        let source = extract_query(path, "source");
        match store.recent(source.as_deref(), limit) {
            Ok(memories) => {
                let body = json!({
                    "memories": memories.iter().map(|m| json!({
                        "id": m.id,
                        "source": m.source,
                        "kind": m.kind,
                        "content": m.content,
                        "metadata": m.metadata,
                        "created_at": m.created_at.to_rfc3339(),
                    })).collect::<Vec<_>>()
                });
                ok_json(&body.to_string())
            }
            Err(e) => err_json(500, &e.to_string()),
        }
    } else if method == "DELETE" && path.starts_with("/api/memory/") {
        let id = path.trim_start_matches("/api/memory/");
        match store.redact(id) {
            Ok(true) => ok_json(r#"{"deleted":true}"#),
            Ok(false) => err_json(404, "not found"),
            Err(e) => err_json(500, &e.to_string()),
        }
    } else {
        err_html(404, "Not found")
    };

    stream.write_all(response.as_bytes()).await?;
    stream.flush().await?;
    Ok(())
}

fn parse_request_line(req: &str) -> (&str, &str) {
    let first = req.lines().next().unwrap_or("");
    let mut parts = first.split_whitespace();
    let method = parts.next().unwrap_or("");
    let path = parts.next().unwrap_or("/");
    (method, path)
}

fn extract_query(path: &str, key: &str) -> Option<String> {
    let q = path.split('?').nth(1)?;
    for pair in q.split('&') {
        let mut kv = pair.splitn(2, '=');
        let k = kv.next()?;
        let v = kv.next().unwrap_or("");
        if k == key {
            return Some(
                urlencoding::decode(v)
                    .map(|s| s.into_owned())
                    .unwrap_or_else(|_| v.to_string()),
            );
        }
    }
    None
}

fn ok_html(body: &str) -> String {
    build_response(200, "OK", "text/html; charset=utf-8", body)
}
fn ok_json(body: &str) -> String {
    build_response(200, "OK", "application/json", body)
}
fn err_json(code: u16, msg: &str) -> String {
    let body = json!({ "error": msg }).to_string();
    build_response(code, status_text(code), "application/json", &body)
}
fn err_html(code: u16, msg: &str) -> String {
    build_response(code, status_text(code), "text/html; charset=utf-8", msg)
}

fn status_text(code: u16) -> &'static str {
    match code {
        200 => "OK",
        400 => "Bad Request",
        404 => "Not Found",
        500 => "Internal Server Error",
        _ => "Error",
    }
}

fn build_response(code: u16, status: &str, ctype: &str, body: &str) -> String {
    format!(
        "HTTP/1.1 {code} {status}\r\n\
         Content-Type: {ctype}\r\n\
         Content-Length: {len}\r\n\
         Cache-Control: no-store\r\n\
         X-Content-Type-Options: nosniff\r\n\
         Connection: close\r\n\
         \r\n\
         {body}",
        len = body.len(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_request_line() {
        let (m, p) = parse_request_line("GET /api/search?q=foo HTTP/1.1\r\nHost: x\r\n\r\n");
        assert_eq!(m, "GET");
        assert_eq!(p, "/api/search?q=foo");
    }

    #[test]
    fn extracts_query_param() {
        let v = extract_query("/api/search?q=hello%20world&k=5", "q");
        assert_eq!(v.as_deref(), Some("hello world"));
        let k = extract_query("/api/search?q=x&k=5", "k");
        assert_eq!(k.as_deref(), Some("5"));
    }
}
