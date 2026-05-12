# Changelog

All notable changes to Heirloom will be documented in this file. The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [0.1.0] — 2026-05-11

The first public release. A working local memory layer that AI tools can query over MCP — with five ingesters, a web viewer, and an auto-capture daemon out of the box.

### Added
- **`heirloom-core`** — SQLite + FTS5 storage with BM25-ranked full-text search, snippet highlighting, idempotent content-hash deduplication, source and time-range filters, hard redaction, and stopword-aware natural-language query handling.
- **`heirloom-ingester`** — `Ingester` trait and `IngestContext` for building plugins.
- **`heirloom-fs`** — Filesystem ingester for `.md`, `.markdown`, `.txt`, `.org`, and `.rst` files. Skips hidden files, dependency directories, and files larger than 5 MiB.
- **`heirloom-browser`** — Chromium-family history ingester (Chrome, Brave, Arc, Edge, Vivaldi). Reads from a temp copy of `History` so the browser doesn't have to be closed. Pulls URL, title, visit count, and last-visit timestamp — no cookies, no form data, no passwords.
- **`heirloom-claude`** — Parser for Anthropic Claude `conversations.json` exports. Handles both the legacy `text` field and the newer structured `content` array.
- **`heirloom-chatgpt`** — Parser for OpenAI ChatGPT `conversations.json` exports. Walks the message tree from `current_node` for stable chronological ordering. Filters out `system` and `tool` roles.
- **`heirloom-claude-code`** — Reads Claude Code session transcripts from `~/.claude/projects/`. Handles both JSONL and JSON-array session files. Attaches `session_id`, `cwd`, and `project` to each turn. Direct local equivalent of npm-based "claude-mem" tools.
- **`heirloom-mcp`** — Minimal Model Context Protocol server over stdio JSON-RPC. Exposes `search_memory`, `recent_memories`, `list_sources`, and `get_memory`.
- **`heirloom-viewer`** — Self-contained local web viewer at `http://127.0.0.1:7878`. Embedded HTML, dark theme, keyboard shortcuts, one-click redact. No framework dependency.
- **`heirloom-watch`** — Auto-capture daemon. Reads `~/.heirloom/config.toml` and runs configured ingesters on a fixed schedule.
- **`heirloom` CLI** with subcommands `init`, `add`, `ingest`, `search`, `recent`, `serve`, `viewer`, `watch`, `export`, `redact`, `status`, and `doctor`. All commands support `--json` for piping.
- Drop-in MCP configuration snippets for Claude Desktop and Cursor in `examples/`.
- Example `config.toml` documenting the auto-capture format.
- 33 unit and integration tests across the workspace.

### Known limitations
- No at-rest encryption (planned for v0.2).
- No vector embeddings (planned for v0.2). Full-text search via FTS5 handles English stemming but not semantic similarity.
- Firefox history (`places.sqlite`) is not yet supported.
- Tested on Linux. macOS and Windows are expected to work but lack CI coverage in this release.

[0.1.0]: https://github.com/heirloom-dev/heirloom/releases/tag/v0.1.0
