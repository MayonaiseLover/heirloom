# Heirloom Sync Protocol

> **Status:** Draft v1, design phase. The client pipeline (`heirloom-sync`)
> is implemented and tested. The hosted relay service is **not** built —
> the section below specifies the API a reference relay must serve.

## Goals

1. **End-to-end encrypted.** The relay never sees plaintext memories.
2. **Multi-device.** Two or more devices owned by the same user converge.
3. **Stateless on the client side.** A device that loses its local state
   can re-sync from scratch given the passphrase and relay URL.
4. **Boring transport.** HTTPS + JSON. No bespoke protocols.
5. **No accounts.** Devices identify themselves by a per-device id and a
   shared passphrase. The relay does not know the user's email.

## Non-goals (for v0.3)

- CRDT-grade conflict resolution. We use last-write-wins on `memory.id`.
- Real-time updates. Sync is pull-on-demand or interval-based.
- Multi-user / team memory. That's a different product (Heirloom Teams).

## Threat model

| Adversary | Can read | Can modify | Can delete |
|---|---|---|---|
| Network attacker (in transit) | Nothing (TLS) | Nothing (TLS) | N/A |
| Relay operator | Header metadata, ciphertext, sizes, timestamps | Nothing (server-side cannot mint a valid blob without the passphrase) | The blob (denial of service) |
| Adversary with offline copy of the relay backend | Same as relay operator | Same | Same |
| Adversary who steals one device | Local plaintext | The user's view, after passphrase entry | Same |
| Adversary who steals the passphrase but not a device | All blobs they can intercept from the relay | New blobs they can mint | Same |

The takeaway: **the passphrase is the entire security boundary**. Heirloom
recommends 6+ random Diceware words or equivalent (≥75 bits of entropy).

## Wire format

All blobs use the `.hlm v1` envelope from [`heirloom-crypto`]:

```
[ MAGIC(4) | VERSION(1) | SALT(16) | NONCE(24) | RESERVED(4) | XCHACHA20_POLY1305_CIPHERTEXT | TAG(16) ]
```

Key derivation: **Argon2id**, m=64 MiB, t=3, p=1 → 32 bytes.
AEAD: **XChaCha20-Poly1305**.

## Relay API

The relay is a thin HTTP service over an object store. Endpoints:

### `POST /snapshots`

Upload a sealed snapshot.

Request:
```http
POST /snapshots HTTP/1.1
Content-Type: application/octet-stream
X-Heirloom-Device-Id: <hex>
X-Heirloom-Snapshot-Id: <hex>
X-Heirloom-Created-At: 2026-05-12T10:00:00Z
X-Heirloom-Sha256: <hex of body>
X-Heirloom-Version: 1
```
Body: raw `.hlm` ciphertext.

Response:
```json
{ "snapshot_id": "...", "accepted_at": "..." }
```

The relay **must** verify that the SHA-256 of the body matches the header
before accepting. The relay **must not** attempt to decrypt.

### `GET /snapshots?since=<rfc3339>&device_id=<hex>`

List headers for snapshots newer than `since`. The optional `device_id`
filter excludes snapshots uploaded *by* the calling device (which it
already has).

Response:
```json
{
  "snapshots": [
    {
      "device_id": "...",
      "snapshot_id": "...",
      "created_at": "...",
      "sha256": "...",
      "size_bytes": 1234567,
      "version": 1
    }
  ]
}
```

### `GET /snapshots/<id>`

Download a snapshot's raw ciphertext. Streams the bytes; the relay never
sees plaintext.

### `DELETE /snapshots/<id>`

Delete a snapshot. The relay may impose a retention policy (e.g. "keep
only the 10 most recent per device") and reject deletes outside that.

## Client flow

### Push

1. Pause local writes (or accept that the snapshot reflects state at copy time).
2. Read `~/.heirloom/heirloom.db` into memory.
3. `heirloom_crypto::seal_bytes(plain, passphrase)` → ciphertext.
4. SHA-256 the ciphertext → `snapshot_id` and `sha256` header.
5. `POST /snapshots` with the body and headers.
6. Persist `snapshot_id` to `~/.heirloom/sync.json` so we don't re-pull our own.

### Pull

1. `GET /snapshots?since=<last_pulled>`.
2. For each header not in `known_snapshots`:
   - `GET /snapshots/<id>` to fetch the ciphertext.
   - `heirloom_crypto::unseal_bytes(ciphertext, passphrase)`.
   - Open the resulting DB as a read-only SQLite, iterate memories.
   - `heirloom_sync::merge_memories` into the local store (LWW on id).
3. Update `last_pulled` and append the new ids to `known_snapshots`.

## Reference relay implementation

A working reference relay is ~150 lines of Rust over [axum](https://github.com/tokio-rs/axum) plus an S3-compatible object store (or local filesystem). Outline:

```rust
// GET /snapshots
async fn list_snapshots(Query(q): Query<ListQuery>, State(store): State<Store>) -> Json<ListResponse> {
    Json(ListResponse { snapshots: store.list_since(q.since, q.device_id).await })
}

// POST /snapshots
async fn upload(headers: HeaderMap, State(store): State<Store>, body: Bytes) -> Result<Json<UploadResponse>> {
    let received_sha = hex::encode(Sha256::digest(&body));
    let header_sha = headers.get("x-heirloom-sha256").ok_or(BadRequest)?.to_str()?;
    if received_sha != header_sha { return Err(BadRequest); }
    let snapshot_id = headers.get("x-heirloom-snapshot-id").ok_or(BadRequest)?.to_str()?.to_string();
    store.put(&snapshot_id, body).await?;
    Ok(Json(UploadResponse { snapshot_id, accepted_at: Utc::now() }))
}

// GET /snapshots/:id
async fn download(Path(id): Path<String>, State(store): State<Store>) -> Result<Bytes> {
    store.get(&id).await.ok_or(NotFound)
}
```

We will publish this as `heirloom-relay` once v0.3 ships. Self-hosting is
the recommended deployment — that way you control the bytes at rest too.

## What's not specified yet

- Multi-passphrase rotation. v1.0.
- Quota enforcement. Up to the relay operator.
- Rate limiting. Up to the relay operator.
- Snapshot pruning policy. Outline: keep N most recent per device, but
  always retain the oldest if the device hasn't pushed in 30+ days.
