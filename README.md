<div align="center">

<img src="assets/logo.svg" width="96" alt="Heirloom logo" />

# Heirloom

**Every AI is amnesiac. Heirloom gives them yours.**

Local-first, MCP-native personal memory for AI. One install, then every MCP-aware AI tool — Claude, Cursor, ChatGPT desktop, custom agents — suddenly knows you.

[![CI](https://github.com/heirloom-dev/heirloom/actions/workflows/ci.yml/badge.svg)](https://github.com/heirloom-dev/heirloom/actions/workflows/ci.yml)
[![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)
[![MCP](https://img.shields.io/badge/MCP-compatible-7c3aed.svg)](https://modelcontextprotocol.io)

</div>

---

```
$ heirloom search "the auth bug Sam was reviewing"

1. [fs] auth.md  (1.402)
   Refactoring the OAuth flow to use <mark>PKCE</mark>. <mark>Sam</mark> is reviewing.
   Deadline is Friday before the demo.
   ↳ /Users/me/notes/auth.md
```

Every AI tool you use has its own siloed, half-broken memory. You re-explain yourself every conversation. You switch tools and lose months of context. The closed alternatives ([Rewind](https://rewind.ai), [Microsoft Recall](https://www.microsoft.com/en-us/windows/copilot-plus-pcs)) ship your life to someone else's server.

Heirloom is a small daemon that ingests what *you* let it (notes, files, browser history, AI chats), stores it locally in an encrypted SQLite database, and exposes it over [**MCP**](https://modelcontextprotocol.io) so any AI can ask: *"what does the user know about this?"*

Your memory is a file you own. Not a SaaS account.

## Why

- **You stop repeating yourself.** Claude already knows your tech stack, your team, last week's decisions.
- **Switching AIs is free.** Move from Claude to Cursor to a custom agent — your memory comes with you.
- **Nothing leaves your machine.** Local-first by default. No telemetry. No phone-home. Ever.

## Quickstart

```bash
# Install (Linux/macOS — Homebrew tap coming with v0.2)
cargo install --git https://github.com/heirloom-dev/heirloom heirloom-cli

# Initialize
heirloom init

# Ingest your notes
heirloom ingest fs --path ~/Documents/notes

# Try it
heirloom search "what was that auth bug"
```

Then drop this into Claude Desktop's config (`~/Library/Application Support/Claude/claude_desktop_config.json` on macOS):

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

## Show me

| You ask | Heirloom returns |
|---|---|
| *"What was I deciding about caching last month?"* | The note where you wrote it down. |
| *"Find the link to that Postgres tuning post Sam shared."* | The exact browser entry, with URL. |
| *"What did I tell ChatGPT about my project structure?"* | The conversation turn, verbatim. |
| *"List my Q2 priorities."* | The doc, ranked by relevance. |
| *"What books did I save about distributed systems?"* | Every memory tagged that direction. |
| *"Who's reviewing the OAuth refactor?"* | The line that names the reviewer. |

## How it works

```
┌──────────────────────────────────────────────────────────┐
│  Your AI (Claude / Cursor / ChatGPT / custom agent)      │
└──────────────────────┬───────────────────────────────────┘
                       │  MCP (stdio JSON-RPC)
                       ▼
┌──────────────────────────────────────────────────────────┐
│  heirloom serve                                          │
│   ├─ search_memory       ├─ recent_memories              │
│   ├─ list_sources        └─ get_memory                   │
└──────────────────────┬───────────────────────────────────┘
                       │
                       ▼
┌──────────────────────────────────────────────────────────┐
│  SQLite + FTS5 (encrypted at rest — v0.2)                │
└──────────────────────┬───────────────────────────────────┘
                       │
       ┌───────────────┴───────────────┐
       ▼                               ▼
┌──────────────┐               ┌──────────────┐
│  ingester:fs │     ...       │ ingester:??? │
└──────────────┘               └──────────────┘
```

A single Rust binary. SQLite for storage. FTS5 for search. Ingesters are tiny crates that turn external sources into `Memory` records — you can write one in a weekend.

## Ingesters

| Name | Status | Description |
|---|---|---|
| `fs` | ✅ shipped | Walks a directory, ingests `.md` / `.txt` / `.rst` / `.org` |
| `browser` | 🚧 v0.2 | Chrome / Brave / Arc history + readable page text |
| `claude` | 🚧 v0.2 | Claude `conversations.json` export |
| `chatgpt` | 🚧 v0.2 | ChatGPT `conversations.json` export |
| `slack` | 💡 wanted | Slack workspace export |
| `linear` | 💡 wanted | Linear issues + comments |
| `obsidian` | 💡 wanted | Obsidian vault with link graph |
| `apple-notes` | 💡 wanted | Apple Notes via JXA |
| `kindle` | 💡 wanted | Kindle highlights + clippings |
| `spotify` | 💡 wanted | Listening history |
| `strava` | 💡 wanted | Workouts |
| `letterboxd` | 💡 wanted | Films watched + reviews |

**Build your own in ~50 lines.** See [CONTRIBUTING.md](CONTRIBUTING.md) and the [`heirloom-ingester`](crates/heirloom-ingester) trait. Open a PR — claim one from [the wanted list](https://github.com/heirloom-dev/heirloom/issues?q=is%3Aissue+label%3Aingester).

## CLI

```
heirloom init                      Initialize a fresh memory store
heirloom add "..."                 Add a memory directly
heirloom ingest <name> [--path P]  Run an ingester
heirloom search "..." [-k N]       Search the store
heirloom recent [-s source]        Show newest memories
heirloom serve                     Start the MCP server on stdio
heirloom redact --id <uuid>        Hard-delete a memory
heirloom redact --query "..."      Delete every memory matching a query
heirloom status                    Counts by source
heirloom doctor                    Diagnose common issues
```

All commands accept `--json` for piping.

## Privacy & security

- **Local-first.** All data lives in `~/.heirloom/heirloom.db`. Nothing is uploaded.
- **No telemetry.** Heirloom does not phone home. Not even opt-in for v0.1.
- **Opt-in per source.** Ingesters only run when you invoke them. None run on a schedule unless you set one up.
- **Redaction first.** `heirloom redact --query "..."` hard-deletes every matching memory, including from the FTS index.
- **Encryption at rest** is on the v0.2 roadmap — see [SECURITY.md](SECURITY.md) for the threat model.

If you find a security issue, please email `security@heirloom.dev` rather than filing a public issue.

## Roadmap

- [x] **v0.1** — Rust core, SQLite FTS5, MCP server, `fs` ingester, CLI
- [ ] **v0.2** — `browser` / `claude` / `chatgpt` ingesters, age encryption at rest, vector embeddings (BGE-small via `fastembed-rs`), `heirloom watch` daemon, Homebrew tap
- [ ] **v0.3** — Tauri desktop UI for browsing and redaction, scheduled ingestion, encrypted multi-device sync
- [ ] **v1.0** — Heirloom Teams (shared memory pools), enterprise SSO/audit

## Contributing

We especially want **new ingesters**. They're the highest-leverage contribution and the design is intentionally tiny — you can ship one in an evening.

1. Pick something from the [wanted list](https://github.com/heirloom-dev/heirloom/issues?q=is%3Aissue+label%3Aingester) (or propose your own).
2. Copy `crates/ingesters/heirloom-fs` as a template.
3. Implement the `Ingester` trait. Tests welcome.
4. Open a PR.

For everything else, see [CONTRIBUTING.md](CONTRIBUTING.md).

## License

[MIT](LICENSE). Use it. Fork it. Build something on top of it.

## Acknowledgments

Built on the shoulders of [SQLite](https://www.sqlite.org), [rusqlite](https://github.com/rusqlite/rusqlite), [Tokio](https://tokio.rs), and the [Model Context Protocol](https://modelcontextprotocol.io). Inspired by Vannevar Bush's [Memex](https://en.wikipedia.org/wiki/Memex), in spirit if not in mechanism.
