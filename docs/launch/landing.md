# heirloom.dev — landing copy

Drop this into the static site of your choice (Astro, Next.js, Vercel template). It's the marketing-side companion to the README — same facts, more focused on the *feeling*.

---

## Hero

**Every AI is amnesiac.**
**Heirloom gives them yours.**

A local-first memory layer for every AI tool you use. One install, then Claude, Cursor, ChatGPT, and any custom agent suddenly know what you know — without sending your life to anyone's cloud.

```bash
curl -sSL heirloom.dev/install | sh
heirloom init
```

→ **Get it on GitHub** · → **View the docs**

## The three benefits

### You stop repeating yourself.

Every AI conversation starts at zero. You explain your stack, your team, last week's decisions, the project you're working on. Heirloom remembers it once and every AI knows it forever.

### Switching tools is free.

Move from Claude to Cursor to a custom agent. Your memory comes with you. No vendor can hold your history hostage because your history isn't theirs — it's a file in your home directory.

### Your memory is yours.

Local-first by default. Stored in SQLite at `~/.heirloom/heirloom.db`. No telemetry. No phone-home. No SaaS account. Encryption at rest in v0.2. If you want sync across devices, that's opt-in and encrypted. If you don't, nothing leaves your machine.

## How it works (one screen)

```
You → Claude/Cursor/ChatGPT → MCP → Heirloom → SQLite (local)
                                       ↑
                                   ingesters
                                       ↑
                               notes, files, history, exports
```

A Rust binary speaks the [Model Context Protocol](https://modelcontextprotocol.io) over stdio. Any MCP-aware AI client connects with one config line. Each ingester is a small plugin — currently `fs` for notes, with browser, Claude exports, ChatGPT exports, Slack, Obsidian, Linear, Kindle, and Spotify in the queue.

## FAQ

**Is this a SaaS?**
No. It's a local binary. There is no Heirloom account. There is no Heirloom cloud (yet — when there is, it'll be opt-in for sync, and the data going over the wire will be encrypted on your machine first).

**What does it actually run?**
A single Rust binary (~5MB) called `heirloom`. It stores data in SQLite, serves an MCP endpoint over stdio, and runs ingesters when you invoke them.

**Will it slow down my AI?**
No. A `search_memory` call against tens of thousands of records typically returns in well under 100ms on a laptop.

**Does it work with [my AI client]?**
If your client speaks MCP (Model Context Protocol), yes. As of v0.1 that includes Claude Desktop, Cursor, Continue, Cline, and any client built on the MCP SDK.

**What if I want my memory to sync across devices?**
v0.2 will add optional encrypted sync. The encryption happens locally; we never see your plaintext.

**How is this different from [Rewind / Microsoft Recall / Mem.ai]?**
Three differences: open source, local-first, and not locked to one vendor. Rewind and Recall record everything continuously; Heirloom only ingests what you point it at. Mem is a hosted note app; Heirloom is a substrate that any AI tool reads from.

**Is it really private?**
The v0.1 SQLite file is unencrypted at rest — anyone with read access to your home directory can read your memories. v0.2 fixes that with age-based encryption. The MCP boundary is the only thing reading the store, and only the AI clients you configure can talk to it.

**Can I use it for work?**
For personal-use Heirloom on your own machine: yes. For shared team memory, wait for v1.0 (Heirloom Teams) — building that needs different storage and auth than the single-user binary.

## Pricing

**Heirloom is free and open-source under the MIT license.** Always.

Future paid tiers (none of which exist yet):

- **Heirloom Cloud** — encrypted sync across your devices. ~$8/mo when it ships.
- **Heirloom Teams** — shared memory pools for organizations. ~$20/seat/mo at v1.0.
- **Heirloom Enterprise** — SSO, audit logs, on-prem deployment, compliance.

The local single-user version stays free forever.

## Footer

GitHub · Docs · Twitter · Blog · `security@heirloom.dev`

© 2026 Heirloom Contributors · [MIT License](https://github.com/heirloom-dev/heirloom/blob/main/LICENSE)
