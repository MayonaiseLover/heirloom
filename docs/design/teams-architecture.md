# Heirloom Teams & Enterprise — Architecture

> **Status:** Design phase. **No code in this repository implements Teams or
> Enterprise yet.** This document specifies what the products will look like
> so contributors and prospective customers can shape the design early.
>
> Heirloom (single-user, the binary in this repo) will always stay free and
> MIT-licensed. Teams and Enterprise are planned as a separate hosted service.

## What Teams is

Heirloom Teams is a **shared memory pool** for an organization. Where the
single-user binary stores one person's memory in a local SQLite file, Teams
stores a *team's* shared memory in a hosted Postgres + object store, and
distributes it to each team member's local Heirloom installation.

Concretely: every engineer on a team installs Heirloom locally, joins their
team's pool, and now every AI tool they use can answer questions like *"what
did we decide about the auth refactor last sprint?"* with knowledge from any
team member's contributions.

## What it is *not*

- It is **not** a replacement for the local single-user store. Personal
  memory stays local; only memories the user explicitly tags as `team:`
  (or are ingested from explicitly team-scoped sources) are synced.
- It is **not** "always-on observation." Teams ingests the same sources the
  single-user binary does — files, browser, AI exports — into a shared
  pool. There's no separate Teams agent watching anyone.
- It is **not** the relay used by personal multi-device sync. Different
  product, different access model, different storage.

## Architecture

```
                        ┌─────────────────────────┐
                        │   Heirloom Teams API    │
                        │   (Rust + axum + sqlx)  │
                        └────────────┬────────────┘
                                     │
              ┌──────────────────────┼──────────────────────┐
              ▼                      ▼                      ▼
   ┌──────────────────┐  ┌──────────────────┐   ┌──────────────────┐
   │   Postgres       │  │  Object store    │   │   IdP (OIDC)     │
   │  (memory index   │  │  (encrypted      │   │   Okta / Google  │
   │   + permissions) │  │   blob ciphertext)│  │   / Microsoft    │
   └──────────────────┘  └──────────────────┘   └──────────────────┘
                                     ▲
                                     │
                ┌────────────────────┼────────────────────┐
                │                    │                    │
                ▼                    ▼                    ▼
       ┌──────────────┐    ┌──────────────┐    ┌──────────────┐
       │  Member A    │    │  Member B    │    │  Member C    │
       │  heirloom    │    │  heirloom    │    │  heirloom    │
       │  + team:eng  │    │  + team:eng  │    │  + team:eng  │
       └──────────────┘    └──────────────┘    └──────────────┘
```

### Authentication

OIDC against the org's IdP. The Heirloom CLI gains a `heirloom team login`
command that runs a PKCE flow, drops a refresh token into the system
keychain, and refreshes silently on subsequent runs.

### Authorization

Three roles at the team level:

| Role | Read | Write | Manage members |
|---|---|---|---|
| Member | ✅ | ✅ | ❌ |
| Admin | ✅ | ✅ | ✅ |
| Read-only | ✅ | ❌ | ❌ |

A future v1.1 adds per-source ACLs (e.g. "only Engineering can see code
review memories"). Out of scope for v1.0.

### Encryption

Team blobs use the same `.hlm v1` envelope from `heirloom-crypto`. The
encryption key is **derived from a team-scoped passphrase**, distributed
out-of-band on join, and never sent to the Teams API. The API sees only
ciphertext, sizes, timestamps, and access logs — exactly like the personal
sync relay.

An optional **enterprise key escrow** mode lets the org IdP custodian
re-encrypt blobs with an org master key for compliance use. This is opt-in,
clearly disclosed in the UI, and turned **off** by default.

### Search

Heirloom Teams ships hybrid retrieval:

1. **Server-side metadata filters** — the Postgres index holds tags,
   sources, contributors, timestamps. No plaintext.
2. **Client-side semantic ranking** — once candidate blob ids come back,
   the client downloads, decrypts, and runs the same BM25+vector hybrid
   the single-user binary uses.

This keeps semantic ranking honest (server can't tamper with relevance) at
the cost of latency on the first query. v1.1 explores letting clients
publish *encrypted* embedding vectors so initial filtering can shortlist
without the full download.

### Audit log

Every API call is recorded with:

- Actor (OIDC `sub`)
- Action (`upload`, `list`, `download`, `delete`)
- Resource (snapshot id)
- Timestamp
- Source IP

Admins can stream the audit log to their SIEM via S3-compatible object
store delivery or a webhook.

## Pricing (planned)

- **Teams**: $20 / seat / month, up to 50 seats.
- **Enterprise**: $contact. Includes SSO/SCIM, on-prem deployment, the key
  escrow mode, dedicated support, signed SLAs, and the audit-log webhook.

The single-user binary in this repository will always be free.

## Deployment options

- **Heirloom Cloud (hosted by us)** — turnkey, multi-tenant. Default.
- **Self-hosted Teams** — Docker compose / Helm chart on the org's
  infrastructure. Enterprise-tier customers get this; the same Rust binary
  serves it.
- **Bring-your-own-bucket** — keep the API hosted by us but point blob
  storage at the org's S3 bucket. Compromise option for customers who want
  managed software but custody of the bytes.

## Status

This is design only. Building Teams is a v1.0 milestone (~Q4 2026 if v0.2
and v0.3 ship on schedule), and not a path the open-source community is
expected to drive — that's the Anthropic/founders side of the company.

If you have feedback on the design, open an issue tagged `teams`.
