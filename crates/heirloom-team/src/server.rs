//! HTTP server for the team-memory service. Same tokio-only style as the
//! viewer crate — no axum/hyper, just enough HTTP to handle our endpoints.
//!
//! ## Endpoints
//!
//! - `GET  /v1/health`               — health check, no auth
//! - `GET  /v1/members`              — list team members (admin only)
//! - `POST /v1/members`              — create member, returns token (admin)
//! - `POST /v1/memories`             — upload an encrypted memory
//! - `GET  /v1/memories`             — list memories with optional filters
//! - `GET  /v1/memories/<id>`        — fetch a single memory
//! - `DELETE /v1/memories/<id>`      — delete (admin only)
//! - `GET  /v1/audit`                — read audit log (admin only)
//!
//! Every request requires `Authorization: Bearer hlmt_...` except `/v1/health`.

use crate::db::{Db, Role, StoredMemory};
use anyhow::Result;
use chrono::Utc;
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::net::SocketAddr;
use std::sync::Arc;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tracing::{debug, info, warn};
use uuid::Uuid;

const MAX_REQUEST_BYTES: usize = 16 * 1024 * 1024;

pub async fn serve(db: Arc<Db>, addr: SocketAddr) -> Result<()> {
    let listener = TcpListener::bind(addr).await?;
    info!("heirloom-team listening on http://{}", addr);
    println!(
        "\n  Heirloom Teams server running at \x1b[1;36mhttp://{}\x1b[0m\n  Press Ctrl-C to stop.\n",
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
        let db = db.clone();
        let ip = peer.ip().to_string();
        tokio::spawn(async move {
            if let Err(e) = handle(stream, db, ip).await {
                debug!("handler error: {}", e);
            }
        });
    }
}

#[derive(Debug, Deserialize)]
struct UploadRequest {
    source: String,
    kind: String,
    /// Hex-encoded ciphertext.
    ciphertext_hex: String,
    #[serde(default)]
    tags: Vec<String>,
}

#[derive(Debug, Serialize)]
struct MemoryWire {
    id: String,
    source: String,
    kind: String,
    tags: Vec<String>,
    uploader_id: String,
    created_at: String,
    size_bytes: u64,
    /// Hex-encoded ciphertext. Decrypt client-side.
    ciphertext_hex: String,
}

impl From<&StoredMemory> for MemoryWire {
    fn from(m: &StoredMemory) -> Self {
        Self {
            id: m.id.clone(),
            source: m.source.clone(),
            kind: m.kind.clone(),
            tags: m.tags.clone(),
            uploader_id: m.uploader_id.clone(),
            created_at: m.created_at.to_rfc3339(),
            size_bytes: m.size_bytes,
            ciphertext_hex: hex::encode(&m.ciphertext),
        }
    }
}

async fn handle(mut stream: TcpStream, db: Arc<Db>, ip: String) -> Result<()> {
    let mut buf = vec![0u8; 64 * 1024];
    let mut total = Vec::with_capacity(64 * 1024);
    // Read until headers terminate (CRLFCRLF), then read declared body length.
    loop {
        let n = stream.read(&mut buf).await?;
        if n == 0 {
            break;
        }
        total.extend_from_slice(&buf[..n]);
        if total.len() > MAX_REQUEST_BYTES {
            return write_response(&mut stream, &err_json(413, "request too large")).await;
        }
        if let Some(header_end) = find_header_end(&total) {
            let body_start = header_end + 4;
            let headers_str = String::from_utf8_lossy(&total[..header_end]);
            let content_length = headers_str
                .lines()
                .find_map(|l| {
                    let l = l.to_ascii_lowercase();
                    if l.starts_with("content-length:") {
                        l.split(':')
                            .nth(1)
                            .and_then(|s| s.trim().parse::<usize>().ok())
                    } else {
                        None
                    }
                })
                .unwrap_or(0);
            while total.len() < body_start + content_length {
                let n = stream.read(&mut buf).await?;
                if n == 0 {
                    break;
                }
                total.extend_from_slice(&buf[..n]);
            }
            break;
        }
    }
    if total.is_empty() {
        return Ok(());
    }
    let response = build_response(&total, &db, &ip).await;
    write_response(&mut stream, &response).await
}

fn find_header_end(buf: &[u8]) -> Option<usize> {
    buf.windows(4).position(|w| w == b"\r\n\r\n")
}

async fn write_response(stream: &mut TcpStream, response: &str) -> Result<()> {
    stream.write_all(response.as_bytes()).await?;
    stream.flush().await?;
    Ok(())
}

async fn build_response(request: &[u8], db: &Arc<Db>, ip: &str) -> String {
    let header_end = match find_header_end(request) {
        Some(i) => i,
        None => return err_json(400, "malformed request"),
    };
    let headers_str = String::from_utf8_lossy(&request[..header_end]);
    let body = &request[(header_end + 4)..];
    let first = headers_str.lines().next().unwrap_or("");
    let mut parts = first.split_whitespace();
    let method = parts.next().unwrap_or("");
    let path = parts.next().unwrap_or("/");

    if path == "/v1/health" {
        return ok_json(&json!({ "ok": true, "version": env!("CARGO_PKG_VERSION") }).to_string());
    }

    // Bearer auth required for everything else.
    let token = match extract_bearer(&headers_str) {
        Some(t) => t,
        None => return err_json(401, "missing bearer token"),
    };
    let member = match db.lookup_member_by_token(&token) {
        Ok(Some(m)) => m,
        Ok(None) => return err_json(401, "invalid or revoked token"),
        Err(e) => return err_json(500, &e.to_string()),
    };

    match (method, path_without_query(path)) {
        ("GET", "/v1/members") => {
            if !member.role.can_admin() {
                return err_json(403, "admin only");
            }
            let _ = db.audit(
                &member.team_id,
                &member.name,
                "members.list",
                None,
                Some(ip),
            );
            match db.list_members(&member.team_id) {
                Ok(ms) => ok_json(&serde_json::to_string(&ms).unwrap_or_default()),
                Err(e) => err_json(500, &e.to_string()),
            }
        }
        ("POST", "/v1/members") => {
            if !member.role.can_admin() {
                return err_json(403, "admin only");
            }
            #[derive(Deserialize)]
            struct CreateMember {
                name: String,
                role: Option<String>,
            }
            let req: CreateMember = match serde_json::from_slice(body) {
                Ok(v) => v,
                Err(e) => return err_json(400, &format!("bad json: {}", e)),
            };
            let role = match req.role.as_deref() {
                Some("admin") => Role::Admin,
                Some("readonly") => Role::ReadOnly,
                _ => Role::Member,
            };
            match db.create_member(&member.team_id, &req.name, role) {
                Ok((m, token)) => {
                    let _ = db.audit(
                        &member.team_id,
                        &member.name,
                        "members.create",
                        Some(&m.id),
                        Some(ip),
                    );
                    ok_json(&json!({ "member": m, "token": token, "note": "this token is shown once; copy now" }).to_string())
                }
                Err(e) => err_json(500, &e.to_string()),
            }
        }
        ("POST", "/v1/memories") => {
            if !member.role.can_write() {
                return err_json(403, "read-only role");
            }
            let req: UploadRequest = match serde_json::from_slice(body) {
                Ok(v) => v,
                Err(e) => return err_json(400, &format!("bad json: {}", e)),
            };
            let ciphertext = match hex::decode(&req.ciphertext_hex) {
                Ok(b) => b,
                Err(_) => return err_json(400, "ciphertext_hex must be valid hex"),
            };
            let mem = StoredMemory {
                id: Uuid::new_v4().to_string(),
                team_id: member.team_id.clone(),
                uploader_id: member.id.clone(),
                source: req.source,
                kind: req.kind,
                size_bytes: ciphertext.len() as u64,
                ciphertext,
                tags: req.tags,
                created_at: Utc::now(),
            };
            match db.put_memory(&mem) {
                Ok(()) => {
                    let _ = db.audit(
                        &member.team_id,
                        &member.name,
                        "memories.put",
                        Some(&mem.id),
                        Some(ip),
                    );
                    ok_json(&json!({ "id": mem.id, "size_bytes": mem.size_bytes }).to_string())
                }
                Err(e) => err_json(500, &e.to_string()),
            }
        }
        ("GET", "/v1/memories") => {
            let q = parse_query(path);
            let source = q.get("source").cloned();
            let limit: i64 = q.get("limit").and_then(|s| s.parse().ok()).unwrap_or(50);
            let since = q
                .get("since")
                .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok())
                .map(|d| d.with_timezone(&Utc));
            match db.list_memories(&member.team_id, source.as_deref(), since, limit) {
                Ok(list) => {
                    let _ = db.audit(
                        &member.team_id,
                        &member.name,
                        "memories.list",
                        None,
                        Some(ip),
                    );
                    let wire: Vec<MemoryWire> = list.iter().map(|m| m.into()).collect();
                    ok_json(
                        &serde_json::to_string(&json!({ "memories": wire })).unwrap_or_default(),
                    )
                }
                Err(e) => err_json(500, &e.to_string()),
            }
        }
        ("GET", p) if p.starts_with("/v1/memories/") => {
            let id = p.trim_start_matches("/v1/memories/").to_string();
            match db.get_memory(&member.team_id, &id) {
                Ok(Some(m)) => {
                    let _ = db.audit(
                        &member.team_id,
                        &member.name,
                        "memories.get",
                        Some(&id),
                        Some(ip),
                    );
                    let wire: MemoryWire = (&m).into();
                    ok_json(&serde_json::to_string(&wire).unwrap_or_default())
                }
                Ok(None) => err_json(404, "not found"),
                Err(e) => err_json(500, &e.to_string()),
            }
        }
        ("DELETE", p) if p.starts_with("/v1/memories/") => {
            if !member.role.can_admin() {
                return err_json(403, "admin only");
            }
            let id = p.trim_start_matches("/v1/memories/").to_string();
            match db.delete_memory(&member.team_id, &id) {
                Ok(true) => {
                    let _ = db.audit(
                        &member.team_id,
                        &member.name,
                        "memories.delete",
                        Some(&id),
                        Some(ip),
                    );
                    ok_json(r#"{"deleted":true}"#)
                }
                Ok(false) => err_json(404, "not found"),
                Err(e) => err_json(500, &e.to_string()),
            }
        }
        ("GET", "/v1/audit") => {
            if !member.role.can_admin() {
                return err_json(403, "admin only");
            }
            let q = parse_query(path);
            let limit: i64 = q.get("limit").and_then(|s| s.parse().ok()).unwrap_or(200);
            match db.list_audit(&member.team_id, limit) {
                Ok(list) => {
                    ok_json(&serde_json::to_string(&json!({ "entries": list })).unwrap_or_default())
                }
                Err(e) => err_json(500, &e.to_string()),
            }
        }
        _ => err_json(404, "not found"),
    }
}

fn extract_bearer(headers: &str) -> Option<String> {
    for line in headers.lines() {
        if let Some(value) = line
            .strip_prefix("Authorization: Bearer ")
            .or_else(|| line.strip_prefix("authorization: Bearer "))
        {
            return Some(value.trim().to_string());
        }
    }
    None
}

fn path_without_query(p: &str) -> &str {
    p.split('?').next().unwrap_or(p)
}

fn parse_query(p: &str) -> std::collections::HashMap<String, String> {
    let mut out = std::collections::HashMap::new();
    if let Some(q) = p.split('?').nth(1) {
        for pair in q.split('&') {
            let mut kv = pair.splitn(2, '=');
            let (Some(k), Some(v)) = (kv.next(), kv.next()) else {
                continue;
            };
            let decoded = urlencoding::decode(v)
                .map(|s| s.into_owned())
                .unwrap_or_else(|_| v.to_string());
            out.insert(k.to_string(), decoded);
        }
    }
    out
}

fn ok_json(body: &str) -> String {
    response(200, "OK", "application/json", body)
}

fn err_json(code: u16, msg: &str) -> String {
    let body = json!({ "error": msg }).to_string();
    response(code, status_text(code), "application/json", &body)
}

fn status_text(code: u16) -> &'static str {
    match code {
        200 => "OK",
        400 => "Bad Request",
        401 => "Unauthorized",
        403 => "Forbidden",
        404 => "Not Found",
        413 => "Payload Too Large",
        500 => "Internal Server Error",
        _ => "Error",
    }
}

fn response(code: u16, status: &str, ctype: &str, body: &str) -> String {
    format!(
        "HTTP/1.1 {code} {status}\r\nContent-Type: {ctype}\r\nContent-Length: {len}\r\nCache-Control: no-store\r\nX-Content-Type-Options: nosniff\r\nConnection: close\r\n\r\n{body}",
        len = body.len()
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_query_with_filters() {
        let q = parse_query("/v1/memories?source=fs&limit=20");
        assert_eq!(q.get("source").unwrap(), "fs");
        assert_eq!(q.get("limit").unwrap(), "20");
    }

    #[test]
    fn extracts_bearer_case_insensitively() {
        let h = "GET /v1/health HTTP/1.1\r\nauthorization: Bearer hlmt_abc\r\nHost: x\r\n\r\n";
        assert_eq!(extract_bearer(h).as_deref(), Some("hlmt_abc"));
    }

    #[test]
    fn locates_header_end() {
        let req = b"GET / HTTP/1.1\r\nHost: x\r\n\r\nbody";
        // CRLFCRLF starts at position 24, body begins at 28.
        let pos = find_header_end(req).unwrap();
        assert_eq!(&req[pos..pos + 4], b"\r\n\r\n");
        assert_eq!(&req[pos + 4..], b"body");
    }
}
