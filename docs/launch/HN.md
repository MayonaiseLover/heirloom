# Show HN drafts

## Title (pick one)

- **Show HN: Heirloom – Local-first, MCP-native memory for every AI**
- **Show HN: Heirloom – Give Claude, Cursor, and ChatGPT a shared memory you own**

The first is cleaner. The second is more visceral. A/B in your head, pick one.

## Body

I got tired of every AI tool being amnesiac in its own special way. Claude doesn't know what I told ChatGPT. Cursor doesn't know what's in my notes. Every conversation starts with the same five paragraphs of context, and switching tools means losing months of accumulated history.

Heirloom is a small Rust binary that fixes this. It ingests what *you* let it (currently: a directory of notes — browser history and AI exports land in v0.2), stores it locally in a SQLite database, and exposes it over [MCP](https://modelcontextprotocol.io). Any MCP-aware client — Claude Desktop, Cursor, Continue, your own agent — can now query your memory with natural language.

```
$ heirloom search "the auth bug Sam was reviewing"

1. [fs] auth.md  (1.402)
   Refactoring the OAuth flow to use PKCE. Sam is reviewing.
   Deadline is Friday before the demo.
```

What it actually is right now:

- A single Rust binary, ~5MB.
- SQLite + FTS5 for storage and search. Vector embeddings come in v0.2.
- An MCP server over stdio JSON-RPC. Speaks `search_memory`, `recent_memories`, `list_sources`, `get_memory`.
- One ingester (filesystem) in v0.1. A trait that takes ~50 lines to implement.
- No telemetry. No phone-home. No cloud account. Nothing leaves your machine.

What I'm hoping the HN crowd does:

1. Tell me where the design is wrong before I commit further.
2. Build the ingesters I haven't gotten to yet. There's a wanted list with ~15 sources — Slack, Obsidian, Linear, Kindle, Spotify, Strava — and each is a weekend project.

The README has the install + Claude Desktop config snippet, both verbatim copy-paste:

https://github.com/MayonaiseLover/heirloom

A few things I deliberately did not do for v0.1:

- **No at-rest encryption** — the SQLite file is plaintext. Coming in v0.2 (age-based). Treat the file accordingly until then.
- **No vector search** — FTS5 is fast and works on personal-scale corpora. Embeddings via `fastembed-rs` are queued for v0.2.
- **No GUI** — the CLI plus MCP is the v0.1 product. Tauri app comes in v0.3.

The bigger story I'm trying to tell: your memory shouldn't be a SaaS account, and it shouldn't be locked inside one AI vendor's app. It should be a file you own, encrypted at rest, that every model you use can read. Heirloom is the smallest possible attempt at that.

Happy to answer anything.

---

## Posting checklist

- [ ] Post Tuesday morning, 8-10am Pacific. Avoid weekends.
- [ ] First comment from a real second account: a concrete use case (e.g. "tried this with my Obsidian vault, the search was surprisingly good")
- [ ] Have the repo, the install one-liner, and the Claude Desktop snippet pinned on the README so the top reply to almost any "how do I" question is a link
- [ ] Don't argue. Reply to substantive critique once with a thoughtful answer. Ignore the rest.
- [ ] Cross-post to /r/LocalLLaMA, /r/rust, /r/selfhosted within 2 hours
- [ ] X thread: lead with the demo GIF, end with the repo link. No emojis.
