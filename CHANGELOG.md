# Changelog

All notable changes to Heirloom will be documented in this file. Format: [Keep a Changelog](https://keepachangelog.com/en/1.1.0/). Versioning: [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [1.0.1] — 2026-05-12

This release closes the gaps identified in the v1.0.0 pre-launch audit. Every feature the README claims is now wired through to the user-facing CLI and verified end-to-end.

### Fixed
- **Hybrid search is now actually used at runtime.** The `heirloom-vector` `HashEmbedder` is attached to every `Store` opened by the CLI. `add()` computes a vector and persists it to a new `vector` BLOB column; `search()` fetches the column, re-ranks BM25 candidates by cosine similarity, and returns blended results. Memories without vectors fall back gracefully to BM25-only ranking.
- **Team-server bearer tokens use a cryptographically secure RNG.** Replaced the previous time + PID derivation with `rand::rngs::OsRng.fill_bytes()`. Added a high-entropy histogram test that fails on any pseudo-random source.
- **`heirloom sync pull --from PATH` now exists.** Decrypts a `.hlm` snapshot, opens it as SQLite, iterates every memory, and merges into the local store with last-write-wins. The merge auto-embeds via the local embedder, so pulled memories get hybrid-search vectors immediately.
- **`heirloom team {ping,members,audit,push,pull}` now make real HTTP requests.** A minimal tokio-based HTTP/1.1 client is built into the CLI (no `ureq`/`url`/`idna` dep chain), so a team member can push encrypted memories to a self-hosted server and another member can pull and decrypt them. Smoke-tested end-to-end across two `HEIRLOOM_HOME` directories against a live server.
- **MCP server now exposes `add_memory`.** Agents can write to the user's store, not just read. The tool schema instructs models to be conservative and to set `source = "agent"` so AI-written memories are distinguishable from ingested ones.
- **`heirloom reindex`** backfills hybrid-search vectors for memories created before v1.0.1 (e.g. upgrading from a v0.1.x store). Idempotent — running it twice on a fully-indexed store reports 0 backfilled.

### Added
- Five new tests in `heirloom-core` covering the embed-on-insert path, the NULL-vector BM25 fallback, the `reindex_vectors` backfill, and that filters still work under hybrid scoring.
- `Embedder` trait now lives in `heirloom-core` so the store can take any embedder without depending on `heirloom-vector`. `HashEmbedder` implements both `heirloom_core::Embedder` and `heirloom_vector::Embedder` so it plugs in directly via `Store::set_embedder`.
- Inline `team_http` module documenting the rationale: HTTPS is intentionally not built in — users put nginx/caddy/Cloudflare Tunnel in front of the team server.

### Honest disclosure
- The team-server-side HTTP transport now works for the bearer-token + role model. **Per-source ACLs**, **OIDC/SAML**, and the **Postgres backend** remain v1.1+ work — but those are scope items, not bugs in v1.0 claims.
- Ingesters are still only unit-tested against synthetic fixtures. **Run each ingester against your own real data once** before launching publicly so you catch any real-world schema mismatches early.

## [1.0.0] — 2026-05-12

The v1.0 cycle adds the **self-hostable Heirloom Teams server** and rounds out the integration story across the MCP-aware ecosystem.

### Added
- **`heirloom-team`** — Self-hostable team-memory server (`heirloom-team-server` binary, plus the library). SQLite-backed storage for memories and a comprehensive audit log; bearer-token authentication scoped per member; role-based access (admin / member / read-only); HTTP API for upload, list, fetch, delete, audit, member management.
  - End-to-end encryption: the server stores only ciphertext. Members seal locally with the team passphrase before upload.
- **`heirloom team` CLI subcommands** for joining a team, configuring the relay, and pushing/pulling encrypted memories.
- **Integration docs at `docs/INTEGRATIONS.md`** covering Claude Desktop, Claude Code, Cursor, Google Antigravity, OpenClaw, Continue, Cline, Zed, Windsurf, and custom-agent integration.
- **Animated SVG demo at `assets/demo.svg`** for the README — single file, renders inline on GitHub.

## [0.2.0] — 2026-05-12

At-rest encryption (XChaCha20-Poly1305 + Argon2id), hybrid lexical+vector search infrastructure, three more ingesters (slack, obsidian, firefox), desktop launcher, client-side sync pipeline. 52 tests.

## [0.1.0] — 2026-05-11

First public release. Core, MCP server, 5 ingesters, CLI, web viewer, watch daemon, export. 34 tests.

[1.0.1]: https://github.com/MayonaiseLover/heirloom/releases/tag/v1.0.1
[1.0.0]: https://github.com/MayonaiseLover/heirloom/releases/tag/v1.0.0
[0.2.0]: https://github.com/MayonaiseLover/heirloom/releases/tag/v0.2.0
[0.1.0]: https://github.com/MayonaiseLover/heirloom/releases/tag/v0.1.0

The v1.0 cycle adds the **self-hostable Heirloom Teams server** and rounds out the integration story across the MCP-aware ecosystem.

### Added
- **`heirloom-team`** — Self-hostable team-memory server (`heirloom-team-server` binary, plus the library). SQLite-backed storage for memories and a comprehensive audit log; bearer-token authentication scoped per member; role-based access (admin / member / read-only); HTTP API for upload, list, fetch, delete, audit, member management.
  - End-to-end encryption: the server stores only ciphertext. Members seal locally with the team passphrase before upload.
  - 10 tests covering db layer + server, including token revocation, audit log ordering, and HTTP parsing.
- **`heirloom team` CLI subcommands** for joining a team, configuring the relay, and pushing/pulling encrypted memories.
- **Integration docs at `docs/INTEGRATIONS.md`** covering Claude Desktop, Claude Code, Cursor, Google Antigravity, OpenClaw, Continue, Cline, Zed, Windsurf, and custom-agent integration. New example config files:
  - `examples/mcp-antigravity.json` — Google Antigravity (Gemini 3 + Claude 4.6 + GPT-OSS, released Nov 2025).
  - `examples/mcp-openclaw.json` — OpenClaw personal AI agent (multi-channel, MCP-native).
- **Animated SVG demo at `assets/demo.svg`** for the README — single file, renders inline on GitHub.

### Changed
- README updated with the v1.0 comparison table including Teams + integration coverage.
- Default domain references updated from `heirloom.web.app` to `heirloom.web.app` (Firebase hosting) pending registration of the `.dev` domain.
- Roadmap restructured to reflect what shipped (v0.1–v1.0) vs what's queued (v1.1–v2.0).

### Status of deferred items
The following remain on the roadmap and are honestly labeled as such in the README and design docs:
- **Hosted personal-sync relay** (v1.1). The client and protocol spec are complete; the hosted service is not.
- **OIDC/SAML SSO** for Teams (v1.1). Bearer tokens cover v1.0 honestly.
- **Postgres backend** for Teams (v1.1). SQLite is fine to several million memories.
- **BERT-quality embeddings via `fastembed-rs`** behind a feature flag (v1.1).
- **Native desktop window via `wry`** (v2.0). For now `heirloom desktop` opens the system browser.

## [0.2.0] — 2026-05-12

At-rest encryption (XChaCha20-Poly1305 + Argon2id), hybrid lexical+vector search, three more ingesters (slack, obsidian, firefox), desktop launcher, client-side sync pipeline. 52 tests.

## [0.1.0] — 2026-05-11

First public release. Core, MCP server, 5 ingesters, CLI, web viewer, watch daemon, export. 34 tests.

[1.0.0]: https://github.com/MayonaiseLover/heirloom/releases/tag/v1.0.0
[0.2.0]: https://github.com/MayonaiseLover/heirloom/releases/tag/v0.2.0
[0.1.0]: https://github.com/MayonaiseLover/heirloom/releases/tag/v0.1.0
