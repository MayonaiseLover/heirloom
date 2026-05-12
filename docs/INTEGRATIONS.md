# Integrating Heirloom with your AI client

Heirloom exposes its memory through a stdio MCP server. Any client that speaks the [Model Context Protocol](https://modelcontextprotocol.io) can use it — no per-client adapter needed. Below are drop-in config snippets for the clients we test against. If your client isn't listed but supports MCP, the snippet structure is virtually identical: invoke `heirloom serve` over stdio.

## Quick reference

```bash
heirloom init                                              # one-time setup
heirloom ingest fs --path ~/Documents/notes                # populate from notes
heirloom ingest browser                                    # populate from history
heirloom ingest claude-code                                # populate from Claude Code
```

Then connect a client from the list below and ask it anything about your past.

## Claude Desktop

Edit `~/Library/Application Support/Claude/claude_desktop_config.json` (macOS) or `%APPDATA%\Claude\claude_desktop_config.json` (Windows):

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

Restart Claude Desktop. The `search_memory`, `recent_memories`, `list_sources`, and `get_memory` tools appear in any conversation.

See [`examples/mcp-claude-desktop.json`](../examples/mcp-claude-desktop.json).

## Claude Code

Heirloom doubles as Claude Code's persistent memory across sessions — closing the gap with [claude-mem](https://github.com/thedotmack/claude-mem) and friends, without the npm install or external services.

```bash
claude mcp add heirloom -- heirloom serve
```

Or paste the snippet into `~/.claude.json`:

```json
{
  "mcpServers": {
    "heirloom": { "command": "heirloom", "args": ["serve"] }
  }
}
```

Pair this with `heirloom ingest claude-code` to feed previous session transcripts back into Heirloom — and now every new session starts with full historical context.

## Cursor

`Settings → MCP → Add new MCP server`, paste:

```json
{
  "mcpServers": {
    "heirloom": {
      "command": "heirloom",
      "args": ["serve"],
      "env": { "HEIRLOOM_LOG": "info" }
    }
  }
}
```

See [`examples/mcp-cursor.json`](../examples/mcp-cursor.json).

## Google Antigravity

`Settings → Customizations → Open MCP Config`:

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

Antigravity's MCP store is GUI-driven, but for any custom MCP server (including this one) the raw config approach works. See [`examples/mcp-antigravity.json`](../examples/mcp-antigravity.json).

## OpenClaw

Add to your workspace `mcp.json`, or use the CLI:

```bash
openclaw mcp add heirloom --command heirloom --args serve
```

OpenClaw discovers tools automatically once the server is configured. See [`examples/mcp-openclaw.json`](../examples/mcp-openclaw.json).

## Continue, Cline, Zed, Windsurf, and friends

Every MCP-aware editor uses the same shape: a command (`heirloom`), arguments (`["serve"]`), and stdio transport. Drop the Cursor snippet into whichever config file your editor uses.

## Custom agents

If you're building an agent yourself, point any MCP SDK at `heirloom serve` as a stdio child process. The tools surfaced are:

| Tool | Purpose |
|---|---|
| `search_memory(query, k, sources?, since?, until?)` | Full-text + hybrid vector search |
| `recent_memories(source?, limit)` | Newest memories, optionally per-source |
| `list_sources()` | Show what sources are populated |
| `get_memory(id)` | Fetch a single memory by id |

Heirloom's MCP server also responds to `initialize`, `tools/list`, `ping`, and the standard notification methods. There are no resources or prompts exposed in v1.0 — only tools.

## Troubleshooting

- **Tool not appearing in the client?** Make sure `heirloom` is on the client's `PATH`. Many GUI clients launch without your shell's environment — use an absolute path like `"/usr/local/bin/heirloom"` if in doubt.
- **Server starts but returns nothing?** Run `heirloom status` to confirm the store has memories. If it's empty, ingest something first.
- **JSON parsing errors in the client logs?** Tracing output is correctly routed to stderr in v0.2+. If you see logs on stdout, you're running an older build — upgrade.
- **`heirloom doctor`** runs a self-check covering filesystem permissions, database open, and FTS5 search.
