<div align="center">

<img src="assets/logo.svg" width="96" alt="Heirloom logo" />

# Heirloom

**Every AI is amnesiac. Heirloom gives them yours.**

A local-first, MCP-native personal memory layer for AI. One install, then every MCP-aware AI tool — Claude, Cursor, ChatGPT desktop, custom agents — suddenly knows you.

[![CI](https://github.com/heirloom-dev/heirloom/actions/workflows/ci.yml/badge.svg)](https://github.com/heirloom-dev/heirloom/actions/workflows/ci.yml)
[![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)
[![MCP](https://img.shields.io/badge/MCP-compatible-7c3aed.svg)](https://modelcontextprotocol.io)
[![Rust](https://img.shields.io/badge/built_with-Rust-orange.svg)](https://www.rust-lang.org/)
[![Encrypted](https://img.shields.io/badge/encryption-XChaCha20--Poly1305-34d399.svg)](docs/design/sync-protocol.md)

</div>

---

```
$ heirloom search "the auth bug Sam was reviewing"

1. [fs] auth.md  (1.402)
   Refactoring the OAuth flow to use <mark>PKCE</mark>. <mark>Sam</mark> is reviewing.
   Deadline is Friday before the demo.
   ↳ /Users/me/notes/auth.md
```

Every AI tool you use has its own siloed, half-broken memory. You re-explain yourself every conversation. You switch tools and lose months of context. The closed alternatives ([Rewind](https://rewind.ai), [Microsoft Recall](https://www.microsoft.com/en-us/windows/copilot-plus-pcs)) ship your life to someone else's server. The "claude-mem" cluster locks you to Claude Code and usually needs npm, Python, or external services like Chroma or Supabase to work.

Heirloom is a single Rust binary that ingests what *you* let it (notes, browser history, AI conversations, Slack and Obsidian exports), stores it locally in an encrypted SQLite database, and exposes it over [**MCP**](https://modelcontextprotocol.io) so any AI can ask: *"what does the user know about this?"*

Your memory is a file you own. Not a SaaS account.

## Why Heirloom over the alternatives?

| | **Heirloom** | claude-mem | claude-brain | claude-memory | Rewind / Recall |
|---|---|---|---|---|---|
| Works with any MCP client | ✅ | ❌ Claude Code only | ❌ Claude Code only | ❌ Claude Code only | ❌ Single app |
| Single binary, no runtime | ✅ Rust | ❌ Needs npm + worker | ✅ Rust | ❌ Needs npm | ✅ |
| No API keys required | ✅ | ❌ LLM for compression | ✅ | ❌ Supabase | ❌ |
| Local-first by default | ✅ | Partial (Chroma) | ✅ | ❌ Cloud sync | ❌ |
| At-rest encryption | ✅ XChaCha20-Poly1305 / Argon2id | ❌ | ❌ | ❌ | ✅ |
| Hybrid lexical + vector search | ✅ BM25 + n-gram TF-IDF | ✅ | Lexical only | ✅ | ✅ |
| Open source | ✅ MIT | ✅ Apache | ✅ | ✅ MIT | ❌ |
| Web viewer included | ✅ | ✅ | ❌ | ❌ | ✅ |
| Auto-capture daemon | ✅ | ✅ hooks | ❌ | ✅ hooks | ✅ |
| Ingest sources beyond AI | ✅ 8 sources | ❌ | ❌ | ❌ | ✅ |
| Pluggable ingester architecture | ✅ | ❌ | ❌ | ❌ | ❌ |
| Encrypted multi-device sync | 🚧 client done, relay v0.3 | ❌ | ❌ | ✅ Supabase | ❌ |

## Quickstart

```bash
curl -sSL https://heirloom.dev/install | sh
heirloom init

# Ingest something
heirloom ingest fs --path ~/Documents/notes
heirloom ingest browser            # auto-detects Chrome/Brave/Arc/Edge/Vivaldi
heirloom ingest firefox            # places.sqlite history
heirloom ingest claude-code        # ~/.claude/projects/ sessions

# Try it
heirloom search "what was that auth bug"

# Open the dashboard
heirloom desktop
```

Then drop this into Claude Desktop's config:

```json
{
  "mcpServers": {
    "heirloom": {
      "command": "heirloom",
      "args": ["serve"]
    }
  }
}
```

Restart Claude Desktop. Ask it something about your past. Watch it answer.

## Ingesters

| Name | Status | Description |
|---|---|---|
| `fs` | ✅ shipped | `.md` / `.txt` / `.rst` / `.org` files |
| `browser` | ✅ shipped | Chrome / Brave / Arc / Edge / Vivaldi history (safe temp-copy read) |
| `firefox` | ✅ shipped | Firefox `places.sqlite` history |
| `claude` | ✅ shipped | Claude `conversations.json` export |
| `chatgpt` | ✅ shipped | ChatGPT `conversations.json` export |
| `claude-code` | ✅ shipped | Claude Code session transcripts |
| `slack` | ✅ shipped | Slack workspace export (point at the unzipped directory) |
| `obsidian` | ✅ shipped | Obsidian vault with frontmatter + wikilink metadata |
| `linear` | 💡 wanted | Linear issues + comments |
| `apple-notes` | 💡 wanted | Apple Notes via JXA |
| `kindle` | 💡 wanted | Kindle highlights + clippings |
| `spotify` | 💡 wanted | Listening history |
| `strava` | 💡 wanted | Workouts |
| `letterboxd` | 💡 wanted | Films watched + reviews |

Build your own in ~50 lines — see [CONTRIBUTING.md](CONTRIBUTING.md) and copy `crates/ingesters/heirloom-fs` as a template.

## Encryption at rest

Heirloom uses **XChaCha20-Poly1305** authenticated encryption with an **Argon2id**-derived key (m=64 MiB, t=3, p=1) for the at-rest format.

```bash
HEIRLOOM_PASSPHRASE='correct horse battery staple' heirloom seal
# heirloom.db is now heirloom.db.hlm (encrypted) and the plaintext is shredded

HEIRLOOM_PASSPHRASE='correct horse battery staple' heirloom unseal
# back to a working plaintext heirloom.db
```

SQLite needs random access, so Heirloom can't run with the file encrypted live. The `seal` / `unseal` workflow gives you real protection against offline attackers while keeping search latency at SQLite speed. See [SECURITY.md](SECURITY.md) for the full threat model.

## Hybrid search

Pure FTS5 misses morphological variants (`postgres` ≠ `postgresql`, `auth` ≠ `authentication`). Pure transformer embeddings need ~80 MB of model + ONNX runtime. Heirloom v0.2 ships a middle ground:

- **BM25** for exact-keyword recall (already from FTS5).
- **Hash-projected n-gram TF-IDF vectors** for morphological + lexical similarity. Pure Rust, zero external models, embedded in the binary.

The `Embedder` trait in `heirloom-vector` is the seam — a v0.3 feature flag will let you swap in a `fastembed-rs` BERT-quality backend if you want transformer-grade recall and don't mind the deps.

## Multi-device sync (status: client done, relay deferred)

The client-side pipeline is implemented and tested:

```bash
heirloom sync status                                    # show device id + relay config
heirloom sync set-relay https://relay.heirloom.dev      # configure a relay
HEIRLOOM_PASSPHRASE='...' heirloom sync push            # produce an encrypted snapshot
```

Encryption is end-to-end with the same `.hlm v1` envelope as `seal`. The relay never sees plaintext memories — only ciphertext sizes, timestamps, and opaque hashes. The full protocol spec is in [docs/design/sync-protocol.md](docs/design/sync-protocol.md).

**What's *not* done:** the production relay service. v0.1 produces snapshot files locally; you can copy them between devices manually for now. The reference relay (~150 lines of axum + S3) ships in v0.3.

## Web viewer & desktop

```
heirloom viewer     # local dashboard at http://127.0.0.1:7878
heirloom desktop    # same dashboard, opens in your default browser
```

Single embedded HTML file. Dark theme. Keyboard shortcuts. One-click redact. No JavaScript framework. No analytics. No telemetry.

A native window (via `wry` on Linux + GTK/webkit, plus the macOS/Windows webviews) is on the v1.0 roadmap — for now, "desktop" means "viewer, but the launcher opens your browser for you", which works on every platform without GTK build deps.

## CLI

```
heirloom init                      Initialize a fresh memory store
heirloom add "..."                 Add a memory directly
heirloom ingest <name> [--path P]  Run an ingester
heirloom search "..." [-k N]       Search the store
heirloom recent [-s source]        Show newest memories
heirloom serve                     Start the MCP server on stdio
heirloom viewer [--addr A]         Start the local web viewer
heirloom desktop                   Open the viewer in your default browser
heirloom watch                     Start the auto-capture daemon
heirloom seal                      Encrypt the database file
heirloom unseal                    Decrypt the sealed database
heirloom sync status               Show sync state
heirloom sync push                 Encrypt + prepare a snapshot
heirloom export [-o FILE]          Export everything as JSONL
heirloom redact --id <uuid>        Hard-delete a memory
heirloom redact --query "..."      Delete every memory matching a query
heirloom status                    Counts by source
heirloom doctor                    Diagnose common issues
```

All commands accept `--json` for piping.

## Auto-capture

```toml
# ~/.heirloom/config.toml
[watch]
interval_minutes = 60

[[watch.tasks]]
ingester = "fs"
path = "/Users/me/Documents/notes"

[[watch.tasks]]
ingester = "browser"

[[watch.tasks]]
ingester = "claude-code"
```

Then `heirloom watch` and forget.

## Privacy & security

- **Local-first.** All data lives in `~/.heirloom/heirloom.db`. Nothing is uploaded.
- **At-rest encryption** (v0.2). XChaCha20-Poly1305 + Argon2id.
- **No telemetry.** Heirloom does not phone home. Not even opt-in.
- **Opt-in per source.** Ingesters run only when you invoke them or list them in `config.toml`.
- **Redaction first.** Hard-delete via CLI or one-click in the viewer.
- **Viewer binds to loopback only.** No LAN exposure.

If you find a security issue, please email `security@heirloom.dev` rather than filing a public issue.

## Roadmap

- [x] **v0.1** — Core, MCP server, 5 ingesters, CLI, web viewer, watch daemon, export
- [x] **v0.2** — At-rest encryption (XChaCha20-Poly1305 + Argon2id), 3 more ingesters (slack, obsidian, firefox), hybrid lexical+vector search, desktop launcher, client-side sync pipeline
- [ ] **v0.3** — Hosted reference relay for multi-device sync, optional `fastembed-rs` BERT-quality embeddings behind a feature flag, native window (wry-based), Homebrew/debian/rpm packages
- [ ] **v1.0** — Heirloom Teams (shared memory pools), enterprise SSO/audit. **Separate hosted product.** See [docs/design/teams-architecture.md](docs/design/teams-architecture.md).

## Contributing

We especially want **new ingesters**. They're the highest-leverage contribution and the design is intentionally tiny — you can ship one in an evening. Pick from the [wanted list](https://github.com/heirloom-dev/heirloom/issues?q=is%3Aissue+label%3Aingester) or propose your own.

For everything else, see [CONTRIBUTING.md](CONTRIBUTING.md).

## License

[MIT](LICENSE). Use it. Fork it. Build something on top of it.

## Acknowledgments

Built on [SQLite](https://www.sqlite.org), [rusqlite](https://github.com/rusqlite/rusqlite), [Tokio](https://tokio.rs), [chacha20poly1305 / argon2 (RustCrypto)](https://github.com/RustCrypto), and the [Model Context Protocol](https://modelcontextprotocol.io). Inspired by Vannevar Bush's [Memex](https://en.wikipedia.org/wiki/Memex), in spirit if not in mechanism.
