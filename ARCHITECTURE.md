# Architecture

A 10-minute tour of how Heirloom is put together. Read this before opening a PR that touches more than one crate.

## Workspace layout

```
heirloom/
├── crates/
│   ├── heirloom-core/        Storage, search, memory types
│   ├── heirloom-ingester/    Ingester trait + IngestContext
│   ├── heirloom-mcp/         MCP server (stdio JSON-RPC)
│   ├── heirloom-cli/         The `heirloom` binary
│   └── ingesters/
│       └── heirloom-fs/      Filesystem ingester (reference impl)
```

One binary, several libraries. Each library has a small, deliberate public API.

## Data model

A `Memory` is the atomic record. Everything ingested becomes one or more of these.

```rust
pub struct Memory {
    pub id: String,                          // UUID
    pub source: String,                      // "fs", "browser", "claude", ...
    pub kind: String,                        // "note", "page", "message", ...
    pub content: String,                     // The remembered text
    pub metadata: HashMap<String, String>,   // path, url, title, author, ...
    pub content_hash: String,                // SHA-256 — dedup key
    pub created_at: DateTime<Utc>,
    pub accessed_at: Option<DateTime<Utc>>,
}
```

Memories are typed by `(source, kind)`. The source corresponds 1:1 with an ingester name. The kind is the ingester's choice — for `fs` it's the file extension, for a future `slack` ingester it might be `message` or `thread`.

## Storage layer

Single SQLite file at `~/.heirloom/heirloom.db`. Two tables:

- `memories` — the rows above, plus an autoincrement `rowid`.
- `memories_fts` — an [FTS5](https://www.sqlite.org/fts5.html) virtual table over `content`, kept in sync via triggers (`memories_ai`, `memories_ad`, `memories_au`).

The store uses `PRAGMA journal_mode=WAL` and serializes all access through a single `Mutex<Connection>`. Heirloom is single-user, so contention is not a concern in v0.1.

### Why not pgvector / Qdrant / etc.?

v0.1 prioritizes a single-binary install with zero external dependencies. FTS5 is good enough for most queries against personal-scale corpora (tens of thousands of memories, in the typical case). v0.2 adds embedding-backed vector recall using `fastembed-rs` and a sidecar HNSW index — the schema is already shaped to accommodate it.

### Dedup

Every memory carries a `content_hash` (SHA-256 of trimmed content). The `memories.content_hash` column is `UNIQUE`, so re-ingesting the same content is a silent no-op. This makes ingesters idempotent without each one having to reason about it.

## Ingester contract

The `Ingester` trait is intentionally small:

```rust
#[async_trait]
pub trait Ingester: Send + Sync {
    fn name(&self) -> &'static str;
    fn description(&self) -> &'static str;
    async fn ingest(&self, ctx: &IngestContext) -> anyhow::Result<IngestReport>;
}
```

`IngestContext` carries the shared `Store`, an optional `since` cutoff for incremental runs, and a `HashMap<String, String>` of options pulled from the CLI flags or `config.toml`. Ingesters are expected to honor `since` if they can, but it's not required.

### Why `&'static str` for `name()`?

Names appear on every memory as the `source` tag and are referenced in queries (`heirloom search foo --source fs`). They must be stable across versions, so we encode that constraint at the type level.

## MCP server

`heirloom-mcp` implements the [Model Context Protocol](https://modelcontextprotocol.io) directly — JSON-RPC 2.0 over stdio. No third-party MCP crate.

Supported methods:

| Method                       | Behavior                                              |
|------------------------------|-------------------------------------------------------|
| `initialize`                 | Returns server info + capabilities                    |
| `notifications/initialized`  | Acknowledged silently                                 |
| `ping`                       | Returns `{}`                                          |
| `tools/list`                 | Returns the four tools below                          |
| `tools/call`                 | Dispatches to one of `search_memory`, `recent_memories`, `list_sources`, `get_memory` |
| `resources/list`             | Returns empty list (we don't expose resources in v0.1) |
| `prompts/list`               | Returns empty list                                    |

All `tools/call` responses are wrapped as `{"content":[{"type":"text","text":"<json>"}]}` per the MCP spec.

### Important: logs go to stderr

The MCP transport is line-delimited JSON on **stdout**. Any tracing output must go to **stderr** or the protocol breaks. `heirloom-cli` sets the tracing subscriber's writer to `std::io::stderr` for exactly this reason. Don't `println!` from the server crate.

## CLI

`heirloom-cli` is a thin clap-derived router. Each subcommand:

1. Resolves `HEIRLOOM_HOME` (env override → `$HOME/.heirloom/`)
2. Opens the store at `<home>/heirloom.db`
3. Dispatches to a `cmd_*` function

The `--json` flag is honored by every command that produces output.

## Why Rust 1.75 pins?

Several transitive dependencies have recently begun requiring `edition2024`, which needs Rust 1.85+. To support a broader install base (and especially Linux distros packaging stable Rust), we pin a known-good set of older versions in the workspace `Cargo.toml`. When the ecosystem stabilizes around edition2024, we'll bump the MSRV in a single coordinated commit.

## Adding a new MCP tool

If you want to expose new functionality to AI clients without a CLI subcommand:

1. Add a JSON schema to `handle_tools_list` in `crates/heirloom-mcp/src/lib.rs`.
2. Add a `tool_yourname(args, store)` function and dispatch to it from `handle_tools_call`.
3. Add a test in the `tests` module that drives it through `handle_request`.

Keep the schema descriptions written *for the AI* — they're effectively the prompt that gets the model to use the tool correctly.

## Performance notes

- `Store::add_many` wraps inserts in a single transaction. Ingesters should batch in chunks of ~128.
- FTS5 with `tokenize='porter unicode61'` handles English stemming and basic Unicode normalization. Good default; revisit per-locale if needed.
- The BM25 score returned by SQLite is negative. We invert it (`-score`) so callers see "higher is better".

## What's deliberately missing

- **No vector embeddings yet.** v0.2.
- **No encryption at rest yet.** v0.2.
- **No background daemon.** Ingestion is invoked manually in v0.1. v0.2 adds `heirloom watch`.
- **No web UI.** v0.3 ships a Tauri desktop app.
- **No multi-tenant store.** Single user, single file. Teams come in v1.0 as a separate service.
