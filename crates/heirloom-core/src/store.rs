use crate::memory::{Memory, SearchFilters, SearchResult};
use crate::{Error, Result};
use chrono::{DateTime, Utc};
use rusqlite::{params, Connection};
use std::path::Path;
use std::sync::Mutex;
use tracing::debug;

/// SQLite-backed memory store with FTS5 full-text search.
///
/// The store is safe to share across threads — internally it serializes access
/// through a [`Mutex`] around a single connection. Heirloom is single-user by
/// design, so contention is not a concern for v0.1.
///
/// ## Example
///
/// ```no_run
/// use heirloom_core::{Store, Memory};
/// # fn run() -> anyhow::Result<()> {
/// let store = Store::open("./heirloom.db")?;
/// let m = Memory::new("fs", "note", "hello world");
/// store.add(&m)?;
/// let hits = store.search("hello", 10, None)?;
/// assert_eq!(hits.len(), 1);
/// # Ok(())
/// # }
/// ```
pub struct Store {
    conn: Mutex<Connection>,
}

impl Store {
    /// Open (or create) a store at the given path.
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        let conn = Connection::open(path)?;
        conn.execute_batch(
            "PRAGMA journal_mode=WAL;
             PRAGMA synchronous=NORMAL;
             PRAGMA foreign_keys=ON;",
        )?;
        let store = Self {
            conn: Mutex::new(conn),
        };
        store.migrate()?;
        Ok(store)
    }

    /// Open an in-memory store (useful for tests).
    pub fn in_memory() -> Result<Self> {
        let conn = Connection::open_in_memory()?;
        let store = Self {
            conn: Mutex::new(conn),
        };
        store.migrate()?;
        Ok(store)
    }

    fn migrate(&self) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute_batch(
            r#"
            CREATE TABLE IF NOT EXISTS memories (
                id TEXT PRIMARY KEY,
                source TEXT NOT NULL,
                kind TEXT NOT NULL,
                content TEXT NOT NULL,
                metadata TEXT NOT NULL DEFAULT '{}',
                content_hash TEXT NOT NULL UNIQUE,
                created_at TEXT NOT NULL,
                accessed_at TEXT
            );

            CREATE INDEX IF NOT EXISTS idx_memories_source ON memories(source);
            CREATE INDEX IF NOT EXISTS idx_memories_created ON memories(created_at);

            CREATE VIRTUAL TABLE IF NOT EXISTS memories_fts USING fts5(
                content,
                content='memories',
                content_rowid='rowid',
                tokenize='porter unicode61'
            );

            CREATE TRIGGER IF NOT EXISTS memories_ai AFTER INSERT ON memories BEGIN
                INSERT INTO memories_fts(rowid, content) VALUES (new.rowid, new.content);
            END;

            CREATE TRIGGER IF NOT EXISTS memories_ad AFTER DELETE ON memories BEGIN
                INSERT INTO memories_fts(memories_fts, rowid, content) VALUES('delete', old.rowid, old.content);
            END;

            CREATE TRIGGER IF NOT EXISTS memories_au AFTER UPDATE ON memories BEGIN
                INSERT INTO memories_fts(memories_fts, rowid, content) VALUES('delete', old.rowid, old.content);
                INSERT INTO memories_fts(rowid, content) VALUES (new.rowid, new.content);
            END;
            "#,
        )?;
        Ok(())
    }

    /// Insert a memory. Returns `Ok(false)` if a memory with the same content
    /// hash already exists (idempotent ingestion).
    pub fn add(&self, memory: &Memory) -> Result<bool> {
        let conn = self.conn.lock().unwrap();
        let metadata = serde_json::to_string(&memory.metadata)?;
        let inserted = conn.execute(
            "INSERT OR IGNORE INTO memories (id, source, kind, content, metadata, content_hash, created_at, accessed_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            params![
                memory.id,
                memory.source,
                memory.kind,
                memory.content,
                metadata,
                memory.content_hash,
                memory.created_at.to_rfc3339(),
                memory.accessed_at.map(|t| t.to_rfc3339()),
            ],
        )?;
        debug!(id = %memory.id, source = %memory.source, inserted = inserted > 0, "store.add");
        Ok(inserted > 0)
    }

    /// Bulk insert. Returns the count newly inserted (duplicates skipped).
    pub fn add_many(&self, memories: &[Memory]) -> Result<usize> {
        let mut conn = self.conn.lock().unwrap();
        let tx = conn.transaction()?;
        let mut inserted = 0usize;
        {
            let mut stmt = tx.prepare(
                "INSERT OR IGNORE INTO memories (id, source, kind, content, metadata, content_hash, created_at, accessed_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            )?;
            for memory in memories {
                let metadata = serde_json::to_string(&memory.metadata)?;
                let n = stmt.execute(params![
                    memory.id,
                    memory.source,
                    memory.kind,
                    memory.content,
                    metadata,
                    memory.content_hash,
                    memory.created_at.to_rfc3339(),
                    memory.accessed_at.map(|t| t.to_rfc3339()),
                ])?;
                inserted += n;
            }
        }
        tx.commit()?;
        Ok(inserted)
    }

    /// Search for memories matching `query`. Uses SQLite FTS5 with BM25 ranking.
    ///
    /// `query` accepts FTS5 syntax (e.g. `"exact phrase"`, `term1 OR term2`).
    /// Raw user input is sanitized — anything weird falls back to a plain bag-of-words match.
    pub fn search(
        &self,
        query: &str,
        k: usize,
        filters: Option<SearchFilters>,
    ) -> Result<Vec<SearchResult>> {
        let sanitized = sanitize_fts_query(query);
        if sanitized.is_empty() {
            return Err(Error::InvalidQuery("empty query".into()));
        }

        let conn = self.conn.lock().unwrap();
        let mut sql = String::from(
            "SELECT m.id, m.source, m.kind, m.content, m.metadata, m.content_hash,
                    m.created_at, m.accessed_at,
                    bm25(memories_fts) AS score,
                    snippet(memories_fts, 0, '<mark>', '</mark>', '…', 24) AS snip
             FROM memories_fts
             JOIN memories m ON m.rowid = memories_fts.rowid
             WHERE memories_fts MATCH ?1",
        );

        let mut bind_idx = 2;
        let mut extra_params: Vec<String> = Vec::new();

        if let Some(f) = &filters {
            if let Some(sources) = &f.sources {
                if !sources.is_empty() {
                    let placeholders: Vec<String> = (bind_idx..bind_idx + sources.len())
                        .map(|i| format!("?{}", i))
                        .collect();
                    sql.push_str(&format!(" AND m.source IN ({})", placeholders.join(",")));
                    for s in sources {
                        extra_params.push(s.clone());
                        bind_idx += 1;
                    }
                }
            }
            if let Some(since) = f.since {
                sql.push_str(&format!(" AND m.created_at >= ?{}", bind_idx));
                extra_params.push(since.to_rfc3339());
                bind_idx += 1;
            }
            if let Some(until) = f.until {
                sql.push_str(&format!(" AND m.created_at <= ?{}", bind_idx));
                extra_params.push(until.to_rfc3339());
                bind_idx += 1;
            }
        }

        sql.push_str(&format!(" ORDER BY score LIMIT ?{}", bind_idx));
        let k_str = k.to_string();
        extra_params.push(k_str);

        let mut stmt = conn.prepare(&sql)?;
        let mut binds: Vec<&dyn rusqlite::ToSql> = vec![&sanitized as &dyn rusqlite::ToSql];
        for p in &extra_params {
            binds.push(p as &dyn rusqlite::ToSql);
        }

        let rows = stmt.query_map(binds.as_slice(), |row| {
            let metadata_json: String = row.get(4)?;
            let metadata = serde_json::from_str(&metadata_json).unwrap_or_default();
            let created_at: String = row.get(6)?;
            let accessed_at: Option<String> = row.get(7)?;
            let score: f64 = row.get(8)?;
            let snip: Option<String> = row.get(9).ok();
            Ok(SearchResult {
                memory: Memory {
                    id: row.get(0)?,
                    source: row.get(1)?,
                    kind: row.get(2)?,
                    content: row.get(3)?,
                    metadata,
                    content_hash: row.get(5)?,
                    created_at: parse_dt(&created_at),
                    accessed_at: accessed_at.as_deref().map(parse_dt),
                },
                // BM25 returns negative values; invert so higher = better.
                score: -score,
                snippet: snip,
            })
        })?;

        let mut out = Vec::new();
        for r in rows {
            out.push(r?);
        }
        Ok(out)
    }

    /// Fetch a memory by id.
    pub fn get(&self, id: &str) -> Result<Option<Memory>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT id, source, kind, content, metadata, content_hash, created_at, accessed_at
             FROM memories WHERE id = ?1",
        )?;
        let mut rows = stmt.query(params![id])?;
        if let Some(row) = rows.next()? {
            let metadata_json: String = row.get(4)?;
            let metadata = serde_json::from_str(&metadata_json).unwrap_or_default();
            let created_at: String = row.get(6)?;
            let accessed_at: Option<String> = row.get(7)?;
            Ok(Some(Memory {
                id: row.get(0)?,
                source: row.get(1)?,
                kind: row.get(2)?,
                content: row.get(3)?,
                metadata,
                content_hash: row.get(5)?,
                created_at: parse_dt(&created_at),
                accessed_at: accessed_at.as_deref().map(parse_dt),
            }))
        } else {
            Ok(None)
        }
    }

    /// Most recently ingested memories, newest first.
    pub fn recent(&self, source: Option<&str>, limit: usize) -> Result<Vec<Memory>> {
        let conn = self.conn.lock().unwrap();
        if let Some(s) = source {
            let mut stmt = conn.prepare(
                "SELECT id, source, kind, content, metadata, content_hash, created_at, accessed_at
                 FROM memories WHERE source = ?1 ORDER BY created_at DESC LIMIT ?2",
            )?;
            let rows = stmt.query_map(params![s, limit], row_to_memory)?;
            let mut out = Vec::new();
            for r in rows {
                out.push(r?);
            }
            Ok(out)
        } else {
            let mut stmt = conn.prepare(
                "SELECT id, source, kind, content, metadata, content_hash, created_at, accessed_at
                 FROM memories ORDER BY created_at DESC LIMIT ?1",
            )?;
            let rows = stmt.query_map(params![limit], row_to_memory)?;
            let mut out = Vec::new();
            for r in rows {
                out.push(r?);
            }
            Ok(out)
        }
    }

    /// Distinct source tags currently in the store, with counts.
    pub fn sources(&self) -> Result<Vec<(String, u64)>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt =
            conn.prepare("SELECT source, COUNT(*) FROM memories GROUP BY source ORDER BY 2 DESC")?;
        let rows = stmt.query_map([], |row| {
            let source: String = row.get(0)?;
            let count: i64 = row.get(1)?;
            Ok((source, count as u64))
        })?;
        let mut out = Vec::new();
        for r in rows {
            out.push(r?);
        }
        Ok(out)
    }

    /// Total memory count.
    pub fn count(&self) -> Result<u64> {
        let conn = self.conn.lock().unwrap();
        let n: i64 = conn.query_row("SELECT COUNT(*) FROM memories", [], |r| r.get(0))?;
        Ok(n as u64)
    }

    /// Hard-delete a memory by id. Returns true if a row was removed.
    pub fn redact(&self, id: &str) -> Result<bool> {
        let conn = self.conn.lock().unwrap();
        let n = conn.execute("DELETE FROM memories WHERE id = ?1", params![id])?;
        Ok(n > 0)
    }

    /// Redact every memory whose content matches the FTS query. Returns the count removed.
    pub fn redact_query(&self, query: &str) -> Result<usize> {
        let sanitized = sanitize_fts_query(query);
        if sanitized.is_empty() {
            return Err(Error::InvalidQuery("empty redact query".into()));
        }
        let conn = self.conn.lock().unwrap();
        let n = conn.execute(
            "DELETE FROM memories WHERE rowid IN (
                SELECT rowid FROM memories_fts WHERE memories_fts MATCH ?1
            )",
            params![sanitized],
        )?;
        Ok(n)
    }
}

fn parse_dt(s: &str) -> DateTime<Utc> {
    DateTime::parse_from_rfc3339(s)
        .map(|d| d.with_timezone(&Utc))
        .unwrap_or_else(|_| Utc::now())
}

fn row_to_memory(row: &rusqlite::Row) -> rusqlite::Result<Memory> {
    let metadata_json: String = row.get(4)?;
    let metadata = serde_json::from_str(&metadata_json).unwrap_or_default();
    let created_at: String = row.get(6)?;
    let accessed_at: Option<String> = row.get(7)?;
    Ok(Memory {
        id: row.get(0)?,
        source: row.get(1)?,
        kind: row.get(2)?,
        content: row.get(3)?,
        metadata,
        content_hash: row.get(5)?,
        created_at: parse_dt(&created_at),
        accessed_at: accessed_at.as_deref().map(parse_dt),
    })
}

/// Sanitize raw user input for FTS5 MATCH. We strip operators that often
/// produce parse errors and OR remaining tokens so natural-language queries
/// "just work" — single-token misses don't kill the whole search.
fn sanitize_fts_query(raw: &str) -> String {
    let cleaned: String = raw
        .chars()
        .map(|c| match c {
            '"' | '\'' | '(' | ')' | '*' | ':' => ' ',
            c => c,
        })
        .collect();

    let tokens: Vec<String> = cleaned
        .split_whitespace()
        .filter(|t| !t.is_empty() && !is_stopword(t))
        .map(|t| {
            // Quote each token to neutralize FTS5 operators inside it.
            format!("\"{}\"", t.replace('"', ""))
        })
        .collect();

    tokens.join(" OR ")
}

fn is_stopword(t: &str) -> bool {
    matches!(
        t.to_ascii_lowercase().as_str(),
        "the"
            | "a"
            | "an"
            | "of"
            | "to"
            | "in"
            | "on"
            | "for"
            | "and"
            | "or"
            | "is"
            | "was"
            | "were"
            | "be"
            | "been"
            | "being"
            | "are"
            | "am"
            | "but"
            | "with"
            | "by"
            | "at"
            | "as"
            | "from"
            | "that"
            | "this"
            | "these"
            | "those"
            | "i"
            | "me"
            | "my"
            | "we"
            | "our"
            | "you"
            | "your"
            | "it"
            | "its"
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Memory;

    #[test]
    fn open_in_memory_and_migrate() {
        let store = Store::in_memory().unwrap();
        assert_eq!(store.count().unwrap(), 0);
    }

    #[test]
    fn add_and_retrieve() {
        let store = Store::in_memory().unwrap();
        let m = Memory::new("fs", "note", "Buy oat milk on Tuesday");
        assert!(store.add(&m).unwrap());
        let fetched = store.get(&m.id).unwrap().unwrap();
        assert_eq!(fetched.content, "Buy oat milk on Tuesday");
    }

    #[test]
    fn dedup_by_content_hash() {
        let store = Store::in_memory().unwrap();
        let m1 = Memory::new("fs", "note", "same content");
        let m2 = Memory::new("fs", "note", "same content");
        assert!(store.add(&m1).unwrap());
        assert!(!store.add(&m2).unwrap(), "second insert should dedupe");
        assert_eq!(store.count().unwrap(), 1);
    }

    #[test]
    fn fts_search_finds_term() {
        let store = Store::in_memory().unwrap();
        store
            .add(&Memory::new(
                "fs",
                "note",
                "Reminder to call Sam tomorrow about the dashboard",
            ))
            .unwrap();
        store
            .add(&Memory::new(
                "fs",
                "note",
                "Grocery list: oats, almonds, kale",
            ))
            .unwrap();
        let hits = store.search("dashboard", 10, None).unwrap();
        assert_eq!(hits.len(), 1);
        assert!(hits[0].memory.content.contains("dashboard"));
        assert!(hits[0].snippet.as_ref().unwrap().contains("<mark>"));
    }

    #[test]
    fn search_handles_messy_query() {
        let store = Store::in_memory().unwrap();
        store
            .add(&Memory::new("fs", "note", "hello world"))
            .unwrap();
        let hits = store.search("hello: (world)*", 10, None).unwrap();
        assert_eq!(hits.len(), 1);
    }

    #[test]
    fn search_filters_by_source() {
        let store = Store::in_memory().unwrap();
        store
            .add(&Memory::new("fs", "note", "shared term alpha"))
            .unwrap();
        store
            .add(&Memory::new("browser", "page", "shared term beta"))
            .unwrap();
        let hits = store
            .search(
                "shared",
                10,
                Some(SearchFilters {
                    sources: Some(vec!["browser".into()]),
                    ..Default::default()
                }),
            )
            .unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].memory.source, "browser");
    }

    #[test]
    fn recent_lists_newest_first() {
        let store = Store::in_memory().unwrap();
        let mut m1 = Memory::new("fs", "note", "first");
        m1.created_at = DateTime::parse_from_rfc3339("2024-01-01T00:00:00Z")
            .unwrap()
            .with_timezone(&Utc);
        let mut m2 = Memory::new("fs", "note", "second");
        m2.created_at = DateTime::parse_from_rfc3339("2025-01-01T00:00:00Z")
            .unwrap()
            .with_timezone(&Utc);
        store.add(&m1).unwrap();
        store.add(&m2).unwrap();
        let recent = store.recent(None, 10).unwrap();
        assert_eq!(recent[0].content, "second");
    }

    #[test]
    fn sources_counts_per_origin() {
        let store = Store::in_memory().unwrap();
        store.add(&Memory::new("fs", "note", "a")).unwrap();
        store.add(&Memory::new("fs", "note", "b")).unwrap();
        store.add(&Memory::new("browser", "page", "c")).unwrap();
        let mut sources = store.sources().unwrap();
        sources.sort();
        assert_eq!(
            sources,
            vec![("browser".to_string(), 1), ("fs".to_string(), 2)]
        );
    }

    #[test]
    fn redact_by_id_removes_row() {
        let store = Store::in_memory().unwrap();
        let m = Memory::new("fs", "note", "secret stuff");
        store.add(&m).unwrap();
        assert!(store.redact(&m.id).unwrap());
        assert_eq!(store.count().unwrap(), 0);
    }

    #[test]
    fn redact_by_query_removes_matches() {
        let store = Store::in_memory().unwrap();
        store
            .add(&Memory::new("fs", "note", "keep me around"))
            .unwrap();
        store
            .add(&Memory::new("fs", "note", "forget the password 1234"))
            .unwrap();
        let n = store.redact_query("password").unwrap();
        assert_eq!(n, 1);
        assert_eq!(store.count().unwrap(), 1);
    }

    #[test]
    fn recent_accepts_very_large_limit() {
        // Regression: passing usize::MAX caused SQLite LIMIT overflow.
        // We expect store.recent to tolerate any limit up to i64::MAX.
        let store = Store::in_memory().unwrap();
        store.add(&Memory::new("fs", "note", "alpha")).unwrap();
        store.add(&Memory::new("fs", "note", "beta")).unwrap();
        let all = store.recent(None, i64::MAX as usize).unwrap();
        assert_eq!(all.len(), 2);
    }

    #[test]
    fn bulk_insert_skips_duplicates() {
        let store = Store::in_memory().unwrap();
        let batch = vec![
            Memory::new("fs", "note", "alpha"),
            Memory::new("fs", "note", "beta"),
            Memory::new("fs", "note", "alpha"),
        ];
        let n = store.add_many(&batch).unwrap();
        assert_eq!(n, 2);
        assert_eq!(store.count().unwrap(), 2);
    }
}
