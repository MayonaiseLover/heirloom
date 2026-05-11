use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use uuid::Uuid;

/// A single unit of remembered content.
///
/// A `Memory` is the atomic record stored by Heirloom. Every ingester produces
/// a stream of these, regardless of where the content originated.
///
/// ## Example
///
/// ```
/// use heirloom_core::Memory;
///
/// let m = Memory::new("fs", "note", "Reminder to call Sam tomorrow")
///     .with_meta("path", "/notes/sam.md");
/// assert_eq!(m.source, "fs");
/// ```
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Memory {
    /// Stable unique identifier.
    pub id: String,

    /// Ingester or origin tag, e.g. `fs`, `browser`, `claude`.
    pub source: String,

    /// Subtype within the source, e.g. `note`, `page`, `message`.
    pub kind: String,

    /// The remembered text content.
    pub content: String,

    /// Free-form metadata. Common keys: `path`, `url`, `title`, `author`.
    pub metadata: HashMap<String, String>,

    /// Content hash, used for deduplication.
    pub content_hash: String,

    /// When this memory was first observed.
    pub created_at: DateTime<Utc>,

    /// When this memory was last surfaced by a search.
    pub accessed_at: Option<DateTime<Utc>>,
}

impl Memory {
    /// Construct a new memory. Generates a fresh UUID and content hash.
    pub fn new(
        source: impl Into<String>,
        kind: impl Into<String>,
        content: impl Into<String>,
    ) -> Self {
        let content = content.into();
        let content_hash = hash_content(&content);
        Self {
            id: Uuid::new_v4().to_string(),
            source: source.into(),
            kind: kind.into(),
            content,
            metadata: HashMap::new(),
            content_hash,
            created_at: Utc::now(),
            accessed_at: None,
        }
    }

    /// Attach a metadata key/value pair (builder style).
    pub fn with_meta(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.metadata.insert(key.into(), value.into());
        self
    }
}

fn hash_content(s: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(s.as_bytes());
    hex::encode(hasher.finalize())
}

/// A single hit from [`crate::Store::search`].
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchResult {
    /// The matching memory.
    pub memory: Memory,

    /// Relevance score. Higher is more relevant. Currently BM25-derived.
    pub score: f64,

    /// Optional snippet with FTS5 highlights wrapped in `<mark>` tags.
    pub snippet: Option<String>,
}

/// Optional filters to narrow a search.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SearchFilters {
    /// Restrict results to these source tags.
    pub sources: Option<Vec<String>>,

    /// Restrict results to memories created on or after this instant.
    pub since: Option<DateTime<Utc>>,

    /// Restrict results to memories created on or before this instant.
    pub until: Option<DateTime<Utc>>,
}
