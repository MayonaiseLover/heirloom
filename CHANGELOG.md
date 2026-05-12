# Changelog

All notable changes to Heirloom will be documented in this file. Format: [Keep a Changelog](https://keepachangelog.com/en/1.1.0/). Versioning: [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [1.0.0] — 2026-05-12

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

[1.0.0]: https://github.com/heirloom-dev/heirloom/releases/tag/v1.0.0
[0.2.0]: https://github.com/heirloom-dev/heirloom/releases/tag/v0.2.0
[0.1.0]: https://github.com/heirloom-dev/heirloom/releases/tag/v0.1.0
