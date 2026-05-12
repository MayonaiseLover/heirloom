<div align="center">

<img src="assets/logo.svg" width="96" alt="Heirloom logo" />

# Heirloom

**Every AI is amnesiac. Heirloom gives them yours.**

A local-first, MCP-native personal memory layer for AI. One install, then every MCP-aware AI tool — Claude, Cursor, ChatGPT desktop, custom agents — suddenly knows you.

[![CI](https://github.com/heirloom-dev/heirloom/actions/workflows/ci.yml/badge.svg)](https://github.com/heirloom-dev/heirloom/actions/workflows/ci.yml)
[![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)
[![MCP](https://img.shields.io/badge/MCP-compatible-7c3aed.svg)](https://modelcontextprotocol.io)
[![Rust](https://img.shields.io/badge/built_with-Rust-orange.svg)](https://www.rust-lang.org/)

</div>

---

```
$ heirloom search "the auth bug Sam was reviewing"

1. [fs] auth.md  (1.402)
   Refactoring the OAuth flow to use <mark>PKCE</mark>. <mark>Sam</mark> is reviewing.
   Deadline is Friday before the demo.
   ↳ /Users/me/notes/auth.md
```

Every AI tool you use has its own siloed, half-broken memory. You re-explain yourself every conversation. You switch tools and lose months of context. The closed alternatives ([Rewind](https://rewind.ai), [Microsoft Recall](https://www.microsoft.com/en-us/windows/copilot-plus-pcs)) ship your life to someone else's server. The "claude-mem" cluster of open-source projects locks you to Claude Code and usually needs npm, Python, or external services like Chroma or Supabase to actually work.

Heirloom is a single Rust binary that ingests what *you* let it (notes, browser history, AI conversations from Claude/ChatGPT/Claude Code), stores it locally in SQLite, and exposes it over [**MCP**](https://modelcontextprotocol.io) so any AI can ask: *"what does the user know about this?"*

Your memory is a file you own. Not a SaaS account.

## Why Heirloom over the alternatives?

| | **Heirloom** | claude-mem | claude-brain | claude-memory | Rewind / Recall |
|---|---|---|---|---|---|
| **Works with any MCP client** | ✅ | ❌ Claude Code only | ❌ Claude Code only | ❌ Claude Code only | ❌ Single app |
| **Single binary, no runtime** | ✅ Rust | ❌ Needs npm + worker | ✅ Rust | ❌ Needs npm | ✅ |
| **No API keys required** | ✅ | ❌ LLM for compression | ✅ | ❌ Supabase | ❌ |
| **Local-first by default** | ✅ | Partial (Chroma) | ✅ | ❌ Cloud sync | ❌ |
| **Open source** | ✅ MIT | ✅ Apache | ✅ | ✅ MIT | ❌ |
| **Web viewer included** | ✅ `localhost:7878` | ✅ `localhost:37777` | ❌ | ❌ | ✅ |
| **Ingests sources beyond AI sessions** | ✅ fs, browser, exports | ❌ | ❌ | ❌ | ✅ |
| **Pluggable ingester architecture** | ✅ | ❌ | ❌ | ❌ | ❌ |
| **Auto-capture daemon** | ✅ `heirloom watch` | ✅ hooks | ❌ | ✅ hooks | ✅ |

## Why

- **You stop repeating yourself.** Claude already knows your tech stack, your team, last week's decisions.
- **Switching AIs is free.** Move from Claude to Cursor to a custom agent — your memory comes with you.
- **Nothing leaves your machine.** Local-first by default. No telemetry. No phone-home. Ever.

## Quickstart

```bash
# Install (Linux/macOS)
curl -sSL https://heirloom.dev/install | sh
# or build from source
cargo install --git https://github.com/heirloom-dev/heirloom heirloom-cli

# Initialize
heirloom init

# Ingest your notes
heirloom ingest fs --path ~/Documents/notes

# Or your browser history (auto-detects Chrome/Brave/Arc/Edge)
heirloom ingest browser

# Or your Claude Code sessions
heirloom ingest claude-code

# Try it
heirloom search "what was that auth bug"

# Open the web viewer
heirloom viewer
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
| *"What did Claude Code and I decide about the auth refactor?"* | The session transcript, by date. |
| *"List my Q2 priorities."* | The doc, ranked by relevance. |
| *"Who's reviewing the OAuth refactor?"* | The line that names the reviewer. |

## How it works

```
┌──────────────────────────────────────────────────────────┐
│  Your AI (Claude / Cursor / ChatGPT / custom agent)      │
└──────────────────────┬───────────────────────────────────┘
                       │  MCP (stdio JSON-RPC)
                       ▼
┌──────────────────────────────────────────────────────────┐
│  heirloom serve            heirloom viewer (web UI)      │
│   ├─ search_memory          ↑                            │
│   ├─ recent_memories        │  http://127.0.0.1:7878     │
│   ├─ list_sources           │                            │
│   └─ get_memory             │                            │
└──────────────────────┬──────┴────────────────────────────┘
                       │
                       ▼
┌──────────────────────────────────────────────────────────┐
│  SQLite + FTS5 (encrypted at rest — v0.2)                │
└──────────────────────┬───────────────────────────────────┘
                       │
       ┌───────────────┼───────────────┬──────────────────┐
       ▼               ▼               ▼                  ▼
┌─────────────┐ ┌──────────────┐ ┌──────────────┐ ┌──────────────┐
│ ingester:fs │ │ingester:     │ │ingester:     │ │ingester:     │
│             │ │  browser     │ │  claude /    │ │ claude-code  │
│             │ │              │ │  chatgpt     │ │              │
└─────────────┘ └──────────────┘ └──────────────┘ └──────────────┘
```

A single Rust binary. SQLite + FTS5 for storage and search. Ingesters are tiny crates that turn external sources into `Memory` records — you can write one in a weekend.

## Ingesters

| Name | Status | Description |
|---|---|---|
| `fs` | ✅ shipped | Walks a directory, ingests `.md` / `.txt` / `.rst` / `.org` |
| `browser` | ✅ shipped | Chrome / Brave / Arc / Edge / Vivaldi history (reads from a temp copy, never blocks the browser) |
| `claude` | ✅ shipped | Claude `conversations.json` export |
| `chatgpt` | ✅ shipped | ChatGPT `conversations.json` export |
| `claude-code` | ✅ shipped | Claude Code session transcripts from `~/.claude/projects/` |
| `slack` | 💡 wanted | Slack workspace export |
| `linear` | 💡 wanted | Linear issues + comments |
| `obsidian` | 💡 wanted | Obsidian vault with link graph |
| `apple-notes` | 💡 wanted | Apple Notes via JXA |
| `kindle` | 💡 wanted | Kindle highlights + clippings |
| `firefox` | 💡 wanted | Firefox `places.sqlite` history |
| `spotify` | 💡 wanted | Listening history |
| `strava` | 💡 wanted | Workouts |
| `letterboxd` | 💡 wanted | Films watched + reviews |

**Build your own in ~50 lines.** See [CONTRIBUTING.md](CONTRIBUTING.md) and the [`heirloom-ingester`](crates/heirloom-ingester) trait. Open a PR — claim one from [the wanted list](https://github.com/heirloom-dev/heirloom/issues?q=is%3Aissue+label%3Aingester).

## Auto-capture

Don't want to run `heirloom ingest` manually? Drop a `config.toml` into `~/.heirloom/`:

```toml
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

Then run `heirloom watch` (or set up a launchd / systemd unit) and forget about it. Heirloom does the rest.

## Web viewer

```
heirloom viewer
```

Opens a local dashboard at `http://127.0.0.1:7878`. Search, browse, redact. Dark mode. Keyboard shortcuts (`/` to focus, `Esc` to clear). No external services, no analytics, no JavaScript framework — single embedded HTML file served from the same binary.

## CLI

```
heirloom init                      Initialize a fresh memory store
heirloom add "..."                 Add a memory directly
heirloom ingest <name> [--path P]  Run an ingester (fs, browser, claude, chatgpt, claude-code)
heirloom search "..." [-k N]       Search the store
heirloom recent [-s source]        Show newest memories
heirloom serve                     Start the MCP server on stdio
heirloom viewer [--addr A]         Start the local web viewer
heirloom watch                     Start the auto-capture daemon
heirloom export [-o FILE]          Export everything as JSONL
heirloom redact --id <uuid>        Hard-delete a memory
heirloom redact --query "..."      Delete every memory matching a query
heirloom status                    Counts by source
heirloom doctor                    Diagnose common issues
```

All commands accept `--json` for piping.

## Privacy & security

- **Local-first.** All data lives in `~/.heirloom/heirloom.db`. Nothing is uploaded.
- **No telemetry.** Heirloom does not phone home. Not even opt-in for v0.1.
- **Opt-in per source.** Ingesters only run when you invoke them, or when you've explicitly listed them in `config.toml` for auto-capture.
- **Redaction first.** `heirloom redact --query "..."` hard-deletes every matching memory, including from the FTS index. The web viewer has a one-click redact button on every card.
- **Viewer binds to loopback only.** `127.0.0.1` by default — no LAN exposure.
- **Encryption at rest** is on the v0.2 roadmap — see [SECURITY.md](SECURITY.md) for the threat model.

If you find a security issue, please email `security@heirloom.dev` rather than filing a public issue.

## Roadmap

- [x] **v0.1** — Rust core, SQLite FTS5, MCP server, 5 ingesters, CLI, web viewer, auto-capture daemon, export
- [ ] **v0.2** — Age-based at-rest encryption, vector embeddings (BGE-small via `fastembed-rs`), `slack` and `obsidian` ingesters, Homebrew tap, debian/rpm packages
- [ ] **v0.3** — Tauri desktop UI, encrypted multi-device sync (relay-based, E2E)
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
