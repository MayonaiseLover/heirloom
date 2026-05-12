//! Storage layer for the team server. SQLite with a tight schema:
//!
//! - `teams`        — top-level org units (one per server for v1.0)
//! - `members`      — users with a token, a role, and a name
//! - `memories`     — opaque encrypted blobs with searchable metadata
//! - `audit_log`    — append-only record of every API call
//!
//! Memory `content` is server-side **ciphertext**. The server cannot decrypt
//! it. Members fetch ciphertext and unseal locally with the team passphrase.

use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};
use std::path::Path;
use std::sync::Mutex;
use uuid::Uuid;

pub struct Db {
    conn: Mutex<Connection>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Team {
    pub id: String,
    pub name: String,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Member {
    pub id: String,
    pub team_id: String,
    pub name: String,
    pub role: Role,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    Admin,
    Member,
    ReadOnly,
}

impl Role {
    pub fn from_db(s: &str) -> Self {
        match s {
            "admin" => Role::Admin,
            "readonly" => Role::ReadOnly,
            _ => Role::Member,
        }
    }
    pub fn to_db(self) -> &'static str {
        match self {
            Role::Admin => "admin",
            Role::Member => "member",
            Role::ReadOnly => "readonly",
        }
    }
    pub fn can_write(self) -> bool {
        matches!(self, Role::Admin | Role::Member)
    }
    pub fn can_admin(self) -> bool {
        matches!(self, Role::Admin)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StoredMemory {
    pub id: String,
    pub team_id: String,
    pub uploader_id: String,
    pub source: String,
    pub kind: String,
    /// Server-side ciphertext. Decryption is the client's problem.
    pub ciphertext: Vec<u8>,
    /// Caller-supplied tags exposed unencrypted for filtering (e.g. "tech-stack").
    pub tags: Vec<String>,
    pub created_at: DateTime<Utc>,
    pub size_bytes: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuditEntry {
    pub id: String,
    pub team_id: String,
    pub actor: String,
    pub action: String,
    pub resource: Option<String>,
    pub ip: Option<String>,
    pub at: DateTime<Utc>,
}

impl Db {
    pub fn open(path: &Path) -> Result<Self> {
        let conn = Connection::open(path).with_context(|| format!("opening {}", path.display()))?;
        conn.execute_batch(
            "PRAGMA journal_mode=WAL;
             PRAGMA synchronous=NORMAL;
             PRAGMA foreign_keys=ON;",
        )?;
        let db = Self {
            conn: Mutex::new(conn),
        };
        db.migrate()?;
        Ok(db)
    }

    pub fn in_memory() -> Result<Self> {
        let conn = Connection::open_in_memory()?;
        let db = Self {
            conn: Mutex::new(conn),
        };
        db.migrate()?;
        Ok(db)
    }

    fn migrate(&self) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute_batch(
            r#"
            CREATE TABLE IF NOT EXISTS teams (
                id TEXT PRIMARY KEY,
                name TEXT NOT NULL,
                created_at TEXT NOT NULL
            );

            CREATE TABLE IF NOT EXISTS members (
                id TEXT PRIMARY KEY,
                team_id TEXT NOT NULL REFERENCES teams(id),
                name TEXT NOT NULL,
                role TEXT NOT NULL DEFAULT 'member',
                created_at TEXT NOT NULL
            );

            CREATE TABLE IF NOT EXISTS tokens (
                token_hash TEXT PRIMARY KEY,
                member_id TEXT NOT NULL REFERENCES members(id) ON DELETE CASCADE,
                created_at TEXT NOT NULL,
                revoked_at TEXT
            );

            CREATE TABLE IF NOT EXISTS memories (
                id TEXT PRIMARY KEY,
                team_id TEXT NOT NULL REFERENCES teams(id),
                uploader_id TEXT NOT NULL REFERENCES members(id),
                source TEXT NOT NULL,
                kind TEXT NOT NULL,
                ciphertext BLOB NOT NULL,
                tags TEXT NOT NULL DEFAULT '[]',
                created_at TEXT NOT NULL,
                size_bytes INTEGER NOT NULL
            );
            CREATE INDEX IF NOT EXISTS idx_memories_team ON memories(team_id, created_at DESC);
            CREATE INDEX IF NOT EXISTS idx_memories_source ON memories(team_id, source);

            CREATE TABLE IF NOT EXISTS audit_log (
                id TEXT PRIMARY KEY,
                team_id TEXT NOT NULL,
                actor TEXT NOT NULL,
                action TEXT NOT NULL,
                resource TEXT,
                ip TEXT,
                at TEXT NOT NULL
            );
            CREATE INDEX IF NOT EXISTS idx_audit_team_at ON audit_log(team_id, at DESC);
            "#,
        )?;
        Ok(())
    }

    pub fn create_team(&self, name: &str) -> Result<Team> {
        let team = Team {
            id: Uuid::new_v4().to_string(),
            name: name.to_string(),
            created_at: Utc::now(),
        };
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT INTO teams (id, name, created_at) VALUES (?1, ?2, ?3)",
            params![team.id, team.name, team.created_at.to_rfc3339()],
        )?;
        Ok(team)
    }

    pub fn first_team(&self) -> Result<Option<Team>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare("SELECT id, name, created_at FROM teams LIMIT 1")?;
        let mut rows = stmt.query([])?;
        if let Some(row) = rows.next()? {
            let created_at: String = row.get(2)?;
            Ok(Some(Team {
                id: row.get(0)?,
                name: row.get(1)?,
                created_at: parse_dt(&created_at),
            }))
        } else {
            Ok(None)
        }
    }

    pub fn create_member(&self, team_id: &str, name: &str, role: Role) -> Result<(Member, String)> {
        let member = Member {
            id: Uuid::new_v4().to_string(),
            team_id: team_id.to_string(),
            name: name.to_string(),
            role,
            created_at: Utc::now(),
        };
        let token = crate::auth::generate_token();
        let token_hash = crate::auth::hash_token(&token);
        let conn = self.conn.lock().unwrap();
        let tx = conn.unchecked_transaction()?;
        tx.execute(
            "INSERT INTO members (id, team_id, name, role, created_at) VALUES (?1, ?2, ?3, ?4, ?5)",
            params![
                member.id,
                member.team_id,
                member.name,
                role.to_db(),
                member.created_at.to_rfc3339()
            ],
        )?;
        tx.execute(
            "INSERT INTO tokens (token_hash, member_id, created_at) VALUES (?1, ?2, ?3)",
            params![token_hash, member.id, member.created_at.to_rfc3339()],
        )?;
        tx.commit()?;
        Ok((member, token))
    }

    pub fn list_members(&self, team_id: &str) -> Result<Vec<Member>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT id, team_id, name, role, created_at FROM members WHERE team_id = ?1 ORDER BY created_at",
        )?;
        let rows = stmt.query_map([team_id], |row| {
            let role: String = row.get(3)?;
            let created_at: String = row.get(4)?;
            Ok(Member {
                id: row.get(0)?,
                team_id: row.get(1)?,
                name: row.get(2)?,
                role: Role::from_db(&role),
                created_at: parse_dt(&created_at),
            })
        })?;
        let mut out = Vec::new();
        for r in rows {
            out.push(r?);
        }
        Ok(out)
    }

    pub fn lookup_member_by_token(&self, token: &str) -> Result<Option<Member>> {
        let hash = crate::auth::hash_token(token);
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT m.id, m.team_id, m.name, m.role, m.created_at
             FROM tokens t JOIN members m ON m.id = t.member_id
             WHERE t.token_hash = ?1 AND t.revoked_at IS NULL
             LIMIT 1",
        )?;
        let mut rows = stmt.query([hash])?;
        if let Some(row) = rows.next()? {
            let role: String = row.get(3)?;
            let created_at: String = row.get(4)?;
            Ok(Some(Member {
                id: row.get(0)?,
                team_id: row.get(1)?,
                name: row.get(2)?,
                role: Role::from_db(&role),
                created_at: parse_dt(&created_at),
            }))
        } else {
            Ok(None)
        }
    }

    pub fn revoke_token_for_member(&self, member_id: &str) -> Result<usize> {
        let conn = self.conn.lock().unwrap();
        let n = conn.execute(
            "UPDATE tokens SET revoked_at = ?1 WHERE member_id = ?2 AND revoked_at IS NULL",
            params![Utc::now().to_rfc3339(), member_id],
        )?;
        Ok(n)
    }

    pub fn put_memory(&self, m: &StoredMemory) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT INTO memories (id, team_id, uploader_id, source, kind, ciphertext, tags, created_at, size_bytes)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
            params![
                m.id,
                m.team_id,
                m.uploader_id,
                m.source,
                m.kind,
                m.ciphertext,
                serde_json::to_string(&m.tags)?,
                m.created_at.to_rfc3339(),
                m.size_bytes as i64,
            ],
        )?;
        Ok(())
    }

    pub fn list_memories(
        &self,
        team_id: &str,
        source: Option<&str>,
        since: Option<DateTime<Utc>>,
        limit: i64,
    ) -> Result<Vec<StoredMemory>> {
        let conn = self.conn.lock().unwrap();
        let limit = limit.min(1000).max(1);
        // Build statement dynamically to keep bound parameters typed.
        if let (Some(src), Some(s)) = (source, since) {
            let mut stmt = conn.prepare(
                "SELECT id, team_id, uploader_id, source, kind, ciphertext, tags, created_at, size_bytes
                 FROM memories WHERE team_id = ?1 AND source = ?2 AND created_at >= ?3
                 ORDER BY created_at DESC LIMIT ?4",
            )?;
            let rows =
                stmt.query_map(params![team_id, src, s.to_rfc3339(), limit], row_to_memory)?;
            collect(rows)
        } else if let Some(src) = source {
            let mut stmt = conn.prepare(
                "SELECT id, team_id, uploader_id, source, kind, ciphertext, tags, created_at, size_bytes
                 FROM memories WHERE team_id = ?1 AND source = ?2
                 ORDER BY created_at DESC LIMIT ?3",
            )?;
            let rows = stmt.query_map(params![team_id, src, limit], row_to_memory)?;
            collect(rows)
        } else if let Some(s) = since {
            let mut stmt = conn.prepare(
                "SELECT id, team_id, uploader_id, source, kind, ciphertext, tags, created_at, size_bytes
                 FROM memories WHERE team_id = ?1 AND created_at >= ?2
                 ORDER BY created_at DESC LIMIT ?3",
            )?;
            let rows = stmt.query_map(params![team_id, s.to_rfc3339(), limit], row_to_memory)?;
            collect(rows)
        } else {
            let mut stmt = conn.prepare(
                "SELECT id, team_id, uploader_id, source, kind, ciphertext, tags, created_at, size_bytes
                 FROM memories WHERE team_id = ?1 ORDER BY created_at DESC LIMIT ?2",
            )?;
            let rows = stmt.query_map(params![team_id, limit], row_to_memory)?;
            collect(rows)
        }
    }

    pub fn get_memory(&self, team_id: &str, id: &str) -> Result<Option<StoredMemory>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT id, team_id, uploader_id, source, kind, ciphertext, tags, created_at, size_bytes
             FROM memories WHERE team_id = ?1 AND id = ?2",
        )?;
        let mut rows = stmt.query(params![team_id, id])?;
        if let Some(row) = rows.next()? {
            Ok(Some(row_to_memory(row)?))
        } else {
            Ok(None)
        }
    }

    pub fn delete_memory(&self, team_id: &str, id: &str) -> Result<bool> {
        let conn = self.conn.lock().unwrap();
        let n = conn.execute(
            "DELETE FROM memories WHERE team_id = ?1 AND id = ?2",
            params![team_id, id],
        )?;
        Ok(n > 0)
    }

    pub fn audit(
        &self,
        team_id: &str,
        actor: &str,
        action: &str,
        resource: Option<&str>,
        ip: Option<&str>,
    ) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT INTO audit_log (id, team_id, actor, action, resource, ip, at) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![
                Uuid::new_v4().to_string(),
                team_id,
                actor,
                action,
                resource,
                ip,
                Utc::now().to_rfc3339(),
            ],
        )?;
        Ok(())
    }

    pub fn list_audit(&self, team_id: &str, limit: i64) -> Result<Vec<AuditEntry>> {
        let conn = self.conn.lock().unwrap();
        let limit = limit.min(10_000).max(1);
        let mut stmt = conn.prepare(
            "SELECT id, team_id, actor, action, resource, ip, at FROM audit_log
             WHERE team_id = ?1 ORDER BY at DESC LIMIT ?2",
        )?;
        let rows = stmt.query_map(params![team_id, limit], |row| {
            let at: String = row.get(6)?;
            Ok(AuditEntry {
                id: row.get(0)?,
                team_id: row.get(1)?,
                actor: row.get(2)?,
                action: row.get(3)?,
                resource: row.get(4)?,
                ip: row.get(5)?,
                at: parse_dt(&at),
            })
        })?;
        let mut out = Vec::new();
        for r in rows {
            out.push(r?);
        }
        Ok(out)
    }
}

fn row_to_memory(row: &rusqlite::Row) -> rusqlite::Result<StoredMemory> {
    let tags_json: String = row.get(6)?;
    let created_at: String = row.get(7)?;
    let size_bytes: i64 = row.get(8)?;
    Ok(StoredMemory {
        id: row.get(0)?,
        team_id: row.get(1)?,
        uploader_id: row.get(2)?,
        source: row.get(3)?,
        kind: row.get(4)?,
        ciphertext: row.get(5)?,
        tags: serde_json::from_str(&tags_json).unwrap_or_default(),
        created_at: parse_dt(&created_at),
        size_bytes: size_bytes as u64,
    })
}

fn collect<I: Iterator<Item = rusqlite::Result<StoredMemory>>>(
    iter: I,
) -> Result<Vec<StoredMemory>> {
    let mut out = Vec::new();
    for r in iter {
        out.push(r?);
    }
    Ok(out)
}

fn parse_dt(s: &str) -> DateTime<Utc> {
    DateTime::parse_from_rfc3339(s)
        .map(|d| d.with_timezone(&Utc))
        .unwrap_or_else(|_| Utc::now())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn create_team_and_member_returns_token() {
        let db = Db::in_memory().unwrap();
        let team = db.create_team("Acme Eng").unwrap();
        let (admin, token) = db.create_member(&team.id, "alice", Role::Admin).unwrap();
        assert!(!token.is_empty());
        let resolved = db.lookup_member_by_token(&token).unwrap().unwrap();
        assert_eq!(resolved.id, admin.id);
        assert_eq!(resolved.role, Role::Admin);
    }

    #[test]
    fn revoke_token_makes_it_invalid() {
        let db = Db::in_memory().unwrap();
        let team = db.create_team("T").unwrap();
        let (m, token) = db.create_member(&team.id, "bob", Role::Member).unwrap();
        assert!(db.lookup_member_by_token(&token).unwrap().is_some());
        db.revoke_token_for_member(&m.id).unwrap();
        assert!(db.lookup_member_by_token(&token).unwrap().is_none());
    }

    #[test]
    fn put_then_list_memories() {
        let db = Db::in_memory().unwrap();
        let team = db.create_team("T").unwrap();
        let (m, _) = db.create_member(&team.id, "alice", Role::Member).unwrap();
        let mem = StoredMemory {
            id: Uuid::new_v4().to_string(),
            team_id: team.id.clone(),
            uploader_id: m.id,
            source: "fs".into(),
            kind: "note".into(),
            ciphertext: b"opaque-ciphertext-bytes".to_vec(),
            tags: vec!["engineering".into()],
            created_at: Utc::now(),
            size_bytes: 23,
        };
        db.put_memory(&mem).unwrap();
        let listed = db.list_memories(&team.id, None, None, 10).unwrap();
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].tags, vec!["engineering".to_string()]);
    }

    #[test]
    fn audit_log_records_actions() {
        let db = Db::in_memory().unwrap();
        let team = db.create_team("T").unwrap();
        db.audit(
            &team.id,
            "alice",
            "upload",
            Some("mem-123"),
            Some("10.0.0.1"),
        )
        .unwrap();
        db.audit(&team.id, "alice", "list", None, Some("10.0.0.1"))
            .unwrap();
        let entries = db.list_audit(&team.id, 10).unwrap();
        assert_eq!(entries.len(), 2);
        // Newest first.
        assert_eq!(entries[0].action, "list");
    }
}
