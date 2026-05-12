# Changelog

All notable changes to Heirloom will be documented in this file. Format: [Keep a Changelog](https://keepachangelog.com/en/1.1.0/). Versioning: [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [0.2.0] — 2026-05-12

The v0.2 cycle hardens the core. Encryption, hybrid search, three more ingesters, the client side of multi-device sync, and a one-command desktop launcher.

### Added
- **`heirloom-crypto`** — At-rest encryption using **XChaCha20-Poly1305** authenticated encryption with an **Argon2id**-derived key (m=64 MiB, t=3, p=1). Documented `.hlm v1` file format. `seal` shreds the plaintext database after encryption; `unseal` restores it.
- **`heirloom-vector`** — Pure-Rust hash-projected n-gram TF-IDF embedder with cosine similarity. Combines with the existing BM25 search via a `hybrid_score` blend. The `Embedder` trait is the seam where v0.3 can drop in BERT-quality embeddings.
- **`heirloom-sync`** — Client side of the encrypted multi-device sync protocol. Snapshot pipeline, device id management, last-write-wins merge. Protocol fully specified in `docs/design/sync-protocol.md`. Hosted relay is v0.3.
- **`heirloom-desktop`** — One-command desktop launcher. Starts the viewer and opens it in the user's default browser. Cross-platform (uses `open` / `xdg-open` / `explorer`); no GTK or native window deps.
- **`heirloom-slack`** — Slack workspace export parser. Reads `users.json` for name resolution; iterates channel JSON files; attaches channel, author, and thread metadata.
- **`heirloom-obsidian`** — Obsidian vault ingester with frontmatter parsing (`fm:` prefixed metadata) and wikilink extraction (handles `[[Page|Alias]]` and `[[Page#section]]`).
- **`heirloom-firefox`** — Firefox `places.sqlite` history ingester. Auto-discovers profiles on macOS/Windows/Linux (including Snap installs). Safe temp-copy read.
- CLI commands: `heirloom seal`, `heirloom unseal`, `heirloom desktop`, `heirloom sync {status,set-relay,push,reset}`.
- Design documents in `docs/design/`:
  - `sync-protocol.md` — full spec for the encrypted multi-device sync protocol and reference relay API.
  - `teams-architecture.md` — design for Heirloom Teams + Enterprise (separate hosted product; not in this repo).

### Changed
- Workspace expanded from 11 to 17 crates.
- Test suite grew from 34 to 52 passing tests.
- Compilation pins refreshed for Rust 1.75 compatibility.

### Known limitations
- The multi-device sync **relay is not yet built**. `heirloom sync push` produces a snapshot file at `~/.heirloom/snapshots/`; copy between devices manually until v0.3.
- The vector layer is hash-projected n-gram TF-IDF, not transformer embeddings. BERT-quality recall ships behind a `--features embeddings` flag in v0.3.
- Native desktop window ships in v1.0; the current `heirloom desktop` opens the system browser.

## [0.1.0] — 2026-05-11

First public release. See git tag `v0.1.0`. Core, MCP server, 5 ingesters, CLI, web viewer, watch daemon, export. 34 tests.

[0.2.0]: https://github.com/heirloom-dev/heirloom/releases/tag/v0.2.0
[0.1.0]: https://github.com/heirloom-dev/heirloom/releases/tag/v0.1.0
