# Security policy

## Reporting a vulnerability

Email `security@heirloom-webb.web.app` rather than opening a public issue. We'll acknowledge within 72 hours.

If the issue is urgent and you can't reach us by email, open a [security advisory](https://github.com/MayonaiseLover/heirloom/security/advisories/new) on GitHub — it stays private until disclosed.

## Threat model

Heirloom is a single-user, on-device tool. Its threat model reflects that.

### In scope

- **Local data confidentiality at rest** *(partial in v0.1, full in v0.2)*. The SQLite database is currently stored unencrypted at `~/.heirloom/heirloom.db`. v0.2 will add age-based encryption with a key derived from a user-held passphrase or system keychain.
- **MCP boundary integrity.** The MCP server should only ever return data the user has explicitly ingested. It must not enumerate filesystem paths outside the configured ingestion roots.
- **Input safety.** User queries pass through FTS5 — we sanitize raw input to prevent FTS query injection from causing parse errors or unintended matches.
- **Dependency hygiene.** We track upstream CVEs in our Rust dependencies via `cargo audit` in CI.

### Out of scope (v0.1)

- **Defending against a local attacker with read access to your home directory.** Until v0.2 adds at-rest encryption, anyone who can read `~/.heirloom/heirloom.db` can read your memories. Treat the file accordingly.
- **Defending against malicious AI clients.** Any MCP client you connect Heirloom to can call `search_memory` and read what it returns. Only connect clients you trust.
- **Sandboxing ingesters.** Ingesters run with the user's full privileges. Only install ingesters from trusted sources.
- **Network attackers.** Heirloom doesn't make outbound connections by default. If you enable an ingester that does (none ship in v0.1), that ingester's network surface is its own concern.

## What Heirloom does *not* do

- Heirloom does not phone home.
- Heirloom does not collect telemetry — not even opt-in in v0.1.
- Heirloom does not auto-update.
- Heirloom does not start on login unless you explicitly configure it to.

## Disclosure

We aim to release a patch within 14 days of confirming a vulnerability. We'll credit reporters in the release notes unless asked otherwise.
