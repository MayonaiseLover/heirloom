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
    embedder: Mutex<Option<Box<dyn Embedder>>>,
}

/// Pluggable embedding backend.
///
/// Implementors turn a text snippet into a fixed-size `f32` vector. Heirloom's
/// store calls `embed` once per inserted memory and once per search query;
/// the resulting vectors are used to re-rank BM25 results by cosine similarity.
///
/// The default is no embedder — pure FTS5. Plug in `heirloom_vector::HashEmbedder`
/// for hybrid search, or any future BERT-quality implementation that satisfies
/// this trait.
pub trait Embedder: Send + Sync + 'static {
    /// Produce an embedding for the given text. Output length must equal `dim()`.
    fn embed(&self, text: &str) -> Vec<f32>;

    /// Fixed dimensionality of every vector this embedder produces.
    fn dim(&self) -> usize;
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
            embedder: Mutex::new(None),
        };
        store.migrate()?;
        Ok(store)
    }

    /// Open an in-memory store (useful for tests).
    pub fn in_memory() -> Result<Self> {
        let conn = Connection::open_in_memory()?;
        let store = Self {
            conn: Mutex::new(conn),
            embedder: Mutex::new(None),
        };
        store.migrate()?;
        Ok(store)
    }

    /// Attach an embedder for hybrid search. Every subsequent `add()` computes
    /// and persists a vector; every `search()` re-ranks BM25 candidates by cosine.
    /// Pre-existing memories without vectors fall back to BM25 only.
    pub fn set_embedder(&self, embedder: Box<dyn Embedder>) {
        *self.embedder.lock().unwrap() = Some(embedder);
    }

    /// Backfill vectors for all memories that don't have one yet. Returns the
    /// number of rows embedded. No-op if no embedder is configured.
    pub fn reindex_vectors(&self) -> Result<usize> {
        let embedder_guard = self.embedder.lock().unwrap();
        let Some(embedder) = embedder_guard.as_ref() else {
            return Ok(0);
        };
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare("SELECT rowid, content FROM memories WHERE vector IS NULL")?;
        let rows: Vec<(i64, String)> = stmt
            .query_map([], |row| {
                Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?))
            })?
            .filter_map(|r| r.ok())
            .collect();
        drop(stmt);
        let mut updated = 0;
        for (rowid, content) in rows {
            let v = embedder.embed(&content);
            let bytes = vec_to_bytes(&v);
            conn.execute(
                "UPDATE memories SET vector = ?1 WHERE rowid = ?2",
                params![bytes, rowid],
            )?;
            updated += 1;
        }
        Ok(updated)
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
        // v0.2.0+: add the `vector` column if missing. We use a try/catch
        // pattern because rusqlite errors on duplicate columns.
        let _ = conn.execute("ALTER TABLE memories ADD COLUMN vector BLOB", []);
        Ok(())
    }

    /// Insert a memory. Returns `Ok(false)` if a memory with the same content
    /// hash already exists (idempotent ingestion). If an embedder is configured,
    /// computes and persists a vector alongside the row.
    pub fn add(&self, memory: &Memory) -> Result<bool> {
        let vector_bytes = self.embed_bytes(&memory.content);
        let conn = self.conn.lock().unwrap();
        let metadata = serde_json::to_string(&memory.metadata)?;
        let inserted = conn.execute(
            "INSERT OR IGNORE INTO memories (id, source, kind, content, metadata, content_hash, created_at, accessed_at, vector)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
            params![
                memory.id,
                memory.source,
                memory.kind,
                memory.content,
                metadata,
                memory.content_hash,
                memory.created_at.to_rfc3339(),
                memory.accessed_at.map(|t| t.to_rfc3339()),
                vector_bytes,
            ],
        )?;
        debug!(id = %memory.id, source = %memory.source, inserted = inserted > 0, "store.add");
        Ok(inserted > 0)
    }

    fn embed_bytes(&self, content: &str) -> Option<Vec<u8>> {
        let guard = self.embedder.lock().unwrap();
        guard.as_ref().map(|e| vec_to_bytes(&e.embed(content)))
    }

    /// Bulk insert. Returns the count newly inserted (duplicates skipped).
    pub fn add_many(&self, memories: &[Memory]) -> Result<usize> {
        // Compute embeddings outside the lock so we don't block readers.
        let vectors: Vec<Option<Vec<u8>>> = {
            let guard = self.embedder.lock().unwrap();
            match guard.as_ref() {
                Some(e) => memories
                    .iter()
                    .map(|m| Some(vec_to_bytes(&e.embed(&m.content))))
                    .collect(),
                None => memories.iter().map(|_| None).collect(),
            }
        };
        let mut conn = self.conn.lock().unwrap();
        let tx = conn.transaction()?;
        let mut inserted = 0usize;
        {
            let mut stmt = tx.prepare(
                "INSERT OR IGNORE INTO memories (id, source, kind, content, metadata, content_hash, created_at, accessed_at, vector)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
            )?;
            for (memory, vec_bytes) in memories.iter().zip(vectors.iter()) {
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
                    vec_bytes,
                ])?;
                inserted += n;
            }
        }
        tx.commit()?;
        Ok(inserted)
    }

    /// Search for memories matching `query`. Uses SQLite FTS5 with BM25 ranking.
    /// When an embedder is configured (via [`Store::set_embedder`]), candidate
    /// rows are re-ranked with a hybrid BM25 + cosine score. Memories without
    /// a stored vector keep their pure-BM25 ranking.
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

        // Embed the query once if we have an embedder.
        let query_vec: Option<Vec<f32>> = {
            let guard = self.embedder.lock().unwrap();
            guard.as_ref().map(|e| e.embed(query))
        };
        let use_hybrid = query_vec.is_some();

        let conn = self.conn.lock().unwrap();
        let mut sql = String::from(
            "SELECT m.id, m.source, m.kind, m.content, m.metadata, m.content_hash,
                    m.created_at, m.accessed_at,
                    bm25(memories_fts) AS score,
                    snippet(memories_fts, 0, '<mark>', '</mark>', '…', 24) AS snip,
                    m.vector
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

        // Fetch a wider candidate window when re-ranking so the hybrid score
        // has room to reshuffle. Otherwise, just ask SQLite for k.
        let limit = if use_hybrid { (k * 4).max(20) } else { k };
        sql.push_str(&format!(" ORDER BY score LIMIT ?{}", bind_idx));
        let k_str = limit.to_string();
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
            let vec_bytes: Option<Vec<u8>> = row.get(10).ok();
            Ok((
                SearchResult {
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
                },
                vec_bytes,
            ))
        })?;

        // Collect BM25 candidates + their stored vectors.
        let mut candidates: Vec<(SearchResult, Option<Vec<u8>>)> = Vec::new();
        for r in rows {
            candidates.push(r?);
        }

        // Hybrid re-rank: blend the (already-inverted, higher-is-better) BM25
        // score with cosine similarity against the query vector. For memories
        // missing a stored vector, the cosine contribution is 0 so they keep
        // BM25-only ranking.
        if let Some(qv) = query_vec {
            // Normalize BM25 scores to [0, 1] across the candidate set so the
            // blend is comparable. Cosine is already in [-1, 1] but we clamp.
            let bm25_max = candidates
                .iter()
                .map(|(r, _)| r.score)
                .fold(0.0_f64, f64::max);
            let scale = if bm25_max > 0.0 { bm25_max } else { 1.0 };
            for (result, vec_bytes) in candidates.iter_mut() {
                let bm25_norm = (result.score / scale).clamp(0.0, 1.0) as f32;
                let cos = match vec_bytes {
                    Some(b) => {
                        let v = bytes_to_vec(b);
                        cosine_sim(&qv, &v).clamp(0.0, 1.0)
                    }
                    None => 0.0,
                };
                // alpha=0.6 → BM25 leads, vector breaks ties and surfaces
                // morphological matches (postgres ↔ postgresql, auth ↔ authentication).
                let alpha = 0.6_f32;
                result.score = (alpha * bm25_norm + (1.0 - alpha) * cos) as f64;
            }
            candidates.sort_by(|a, b| {
                b.0.score
                    .partial_cmp(&a.0.score)
                    .unwrap_or(std::cmp::Ordering::Equal)
            });
            candidates.truncate(k);
        }

        Ok(candidates.into_iter().map(|(r, _)| r).collect())
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

fn vec_to_bytes(v: &[f32]) -> Vec<u8> {
    let mut out = Vec::with_capacity(v.len() * 4);
    for x in v {
        out.extend_from_slice(&x.to_le_bytes());
    }
    out
}

fn bytes_to_vec(b: &[u8]) -> Vec<f32> {
    let n = b.len() / 4;
    let mut out = Vec::with_capacity(n);
    for i in 0..n {
        let chunk: [u8; 4] = [b[i * 4], b[i * 4 + 1], b[i * 4 + 2], b[i * 4 + 3]];
        out.push(f32::from_le_bytes(chunk));
    }
    out
}

fn cosine_sim(a: &[f32], b: &[f32]) -> f32 {
    if a.len() != b.len() || a.is_empty() {
        return 0.0;
    }
    // Vectors from heirloom-vector are pre-normalized so cosine collapses to dot product.
    // We compute the full norm-aware form anyway in case future embedders skip normalization.
    let mut dot = 0.0f32;
    let mut na = 0.0f32;
    let mut nb = 0.0f32;
    for i in 0..a.len() {
        dot += a[i] * b[i];
        na += a[i] * a[i];
        nb += b[i] * b[i];
    }
    let denom = (na.sqrt() * nb.sqrt()).max(f32::EPSILON);
    dot / denom
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

    /// Minimal embedder that returns a constant non-zero vector. Just enough
    /// to exercise the embed-on-insert / re-rank code paths without depending
    /// on heirloom-vector (which would create a cycle for this crate's tests).
    struct ConstantEmbedder;
    impl Embedder for ConstantEmbedder {
        fn embed(&self, text: &str) -> Vec<f32> {
            let mut v = vec![0f32; 4];
            // Deterministic but content-sensitive enough that two different
            // strings produce different vectors. Not real semantics — just
            // wiring proof.
            for (i, b) in text.bytes().enumerate() {
                v[i % 4] += (b as f32) / 128.0;
            }
            let norm: f32 = v
                .iter()
                .map(|x| x * x)
                .sum::<f32>()
                .sqrt()
                .max(f32::EPSILON);
            for x in &mut v {
                *x /= norm;
            }
            v
        }
        fn dim(&self) -> usize {
            4
        }
    }

    #[test]
    fn embedder_writes_vector_column_on_insert() {
        let store = Store::in_memory().unwrap();
        store.set_embedder(Box::new(ConstantEmbedder));
        let m = Memory::new("fs", "note", "hello world");
        store.add(&m).unwrap();

        // Verify the row has non-null vector bytes.
        let conn = store.conn.lock().unwrap();
        let bytes: Option<Vec<u8>> = conn
            .query_row(
                "SELECT vector FROM memories WHERE id = ?1",
                params![m.id],
                |row| row.get(0),
            )
            .unwrap();
        assert!(
            bytes.is_some(),
            "vector column should be populated when embedder is attached"
        );
        let bytes = bytes.unwrap();
        assert_eq!(bytes.len(), 4 * 4, "vector should be 4 f32s = 16 bytes");
    }

    #[test]
    fn search_falls_back_to_bm25_for_pre_embedder_rows() {
        // Insert rows without an embedder — they get NULL vector.
        let store = Store::in_memory().unwrap();
        store
            .add(&Memory::new("fs", "note", "the auth refactor uses PKCE"))
            .unwrap();
        store
            .add(&Memory::new("fs", "note", "groceries: milk eggs bread"))
            .unwrap();
        // Now attach an embedder and search — should still find the right row.
        store.set_embedder(Box::new(ConstantEmbedder));
        let hits = store.search("auth refactor", 5, None).unwrap();
        assert!(!hits.is_empty());
        assert!(hits[0].memory.content.contains("auth"));
    }

    #[test]
    fn reindex_backfills_missing_vectors() {
        let store = Store::in_memory().unwrap();
        // Insert before attaching embedder → no vector stored.
        store.add(&Memory::new("fs", "note", "alpha")).unwrap();
        store.add(&Memory::new("fs", "note", "beta")).unwrap();
        store.set_embedder(Box::new(ConstantEmbedder));
        let n = store.reindex_vectors().unwrap();
        assert_eq!(n, 2);
        // Idempotent: running again should backfill 0.
        let n2 = store.reindex_vectors().unwrap();
        assert_eq!(n2, 0);
    }

    #[test]
    fn hybrid_search_does_not_break_filters() {
        let store = Store::in_memory().unwrap();
        store.set_embedder(Box::new(ConstantEmbedder));
        store
            .add(&Memory::new("fs", "note", "alpha postgres tuning"))
            .unwrap();
        store
            .add(&Memory::new("browser", "page", "alpha redis cluster"))
            .unwrap();
        let filters = SearchFilters {
            sources: Some(vec!["fs".into()]),
            ..Default::default()
        };
        let hits = store.search("alpha", 5, Some(filters)).unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].memory.source, "fs");
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
