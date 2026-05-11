# Changelog

All notable changes to Heirloom will be documented in this file. The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [0.1.0] — 2026-05-11

The first public release. A working local memory layer that AI tools can query over MCP.

### Added
- `heirloom-core` — SQLite + FTS5 storage with BM25-ranked full-text search, snippet highlighting, idempotent content-hash deduplication, source and time-range filters, and hard redaction.
- `heirloom-ingester` — `Ingester` trait and `IngestContext` for building plugins.
- `heirloom-fs` — Filesystem ingester for `.md`, `.markdown`, `.txt`, `.org`, and `.rst` files. Skips hidden files, dependency directories, and files larger than 5 MiB.
- `heirloom-mcp` — Minimal Model Context Protocol server over stdio JSON-RPC. Exposes `search_memory`, `recent_memories`, `list_sources`, and `get_memory`.
- `heirloom` CLI with subcommands `init`, `add`, `ingest`, `search`, `recent`, `serve`, `redact`, `status`, and `doctor`. All commands support `--json` for piping.
- Drop-in MCP configuration snippets for Claude Desktop and Cursor in `examples/`.
- 23 unit and integration tests across the workspace.

### Known limitations
- No at-rest encryption (planned for v0.2).
- No vector embeddings (planned for v0.2).
- Only one ingester ships in v0.1 — the `browser` and `claude` ingesters land in v0.2.
- Tested on Linux. macOS and Windows are expected to work but lack CI coverage in this release.

[0.1.0]: https://github.com/heirloom-dev/heirloom/releases/tag/v0.1.0
