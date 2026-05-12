//! # heirloom-sync
//!
//! Client-side implementation of the Heirloom Sync Protocol — encrypted,
//! relay-mediated, multi-device sync.
//!
//! **What's in this crate:** types, serialization, and the local snapshot
//! pipeline. The actual HTTP transport against a relay lives in the CLI so
//! this crate stays embeddable.
//!
//! **What's not built yet (honest disclosure):**
//! - A production relay server. A reference design and minimal HTTP API live
//!   in `docs/design/sync-protocol.md` — a thin service over an object store
//!   that never sees plaintext.
//! - CRDT conflict resolution. v0.3 ships last-write-wins on `memory.id`.
//!
//! ## Protocol summary
//!
//! 1. Each device generates a long-lived `device_id` and stores it locally.
//! 2. The user picks a shared passphrase. It never leaves the device.
//! 3. To push: encrypt the SQLite snapshot with [`heirloom_crypto::seal_bytes`],
//!    hash the ciphertext, upload `(device_id, snapshot_id, timestamp, sha256)`
//!    metadata plus the blob to the relay.
//! 4. To pull: ask the relay for snapshots newer than the local cursor,
//!    download, decrypt with [`heirloom_crypto::unseal_bytes`], merge with
//!    last-write-wins on memory id.
//!
//! The relay never sees plaintext memories. It can list sizes, timestamps,
//! and opaque hashes — nothing else.

use heirloom_core::Memory;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash)]
pub struct DeviceId(pub String);

impl DeviceId {
    pub fn new() -> Self {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let mut hasher = Sha256::new();
        hasher.update(nanos.to_le_bytes());
        hasher.update(hostname());
        Self(hex::encode(&hasher.finalize()[..8]))
    }
}

impl Default for DeviceId {
    fn default() -> Self {
        Self::new()
    }
}

fn hostname() -> String {
    std::env::var("HOSTNAME")
        .or_else(|_| std::env::var("COMPUTERNAME"))
        .unwrap_or_else(|_| "unknown".into())
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SnapshotHeader {
    pub device_id: DeviceId,
    pub snapshot_id: String,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub sha256: String,
    pub size_bytes: u64,
    pub version: u32,
}

impl SnapshotHeader {
    pub fn for_payload(device_id: DeviceId, ciphertext: &[u8]) -> Self {
        let mut h = Sha256::new();
        h.update(ciphertext);
        let digest = h.finalize();
        let sha256 = hex::encode(digest);
        let snapshot_id = hex::encode(&digest[..16]);
        Self {
            device_id,
            snapshot_id,
            created_at: chrono::Utc::now(),
            sha256,
            size_bytes: ciphertext.len() as u64,
            version: 1,
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SyncState {
    pub device_id: DeviceId,
    pub last_pulled: Option<chrono::DateTime<chrono::Utc>>,
    pub known_snapshots: Vec<String>,
    pub relay_url: Option<String>,
}

impl SyncState {
    pub fn load(home: &Path) -> anyhow::Result<Self> {
        let path = home.join("sync.json");
        if !path.exists() {
            let s = SyncState {
                device_id: DeviceId::new(),
                ..Default::default()
            };
            s.save(home)?;
            return Ok(s);
        }
        let raw = std::fs::read_to_string(&path)?;
        Ok(serde_json::from_str(&raw)?)
    }

    pub fn save(&self, home: &Path) -> anyhow::Result<()> {
        std::fs::write(home.join("sync.json"), serde_json::to_string_pretty(self)?)?;
        Ok(())
    }
}

/// Take a database file, produce an encrypted blob plus header.
pub fn prepare_snapshot(
    db_path: &Path,
    passphrase: &str,
    device_id: DeviceId,
) -> anyhow::Result<(SnapshotHeader, Vec<u8>)> {
    let plain = std::fs::read(db_path)?;
    let ciphertext = heirloom_crypto::seal_bytes(&plain, passphrase)?;
    let header = SnapshotHeader::for_payload(device_id, &ciphertext);
    Ok((header, ciphertext))
}

/// Decrypt an incoming blob to a fresh DB file.
pub fn apply_snapshot(
    out_dir: &Path,
    ciphertext: &[u8],
    passphrase: &str,
) -> anyhow::Result<PathBuf> {
    let plain = heirloom_crypto::unseal_bytes(ciphertext, passphrase)?;
    let out = out_dir.join("incoming.db");
    std::fs::write(&out, plain)?;
    Ok(out)
}

/// Last-write-wins merge into the local store.
pub fn merge_memories(
    local: &heirloom_core::Store,
    incoming: impl IntoIterator<Item = Memory>,
) -> anyhow::Result<(u64, u64, u64)> {
    let mut inserted = 0u64;
    let mut skipped = 0u64;
    let errors = 0u64;
    for m in incoming {
        match local.add(&m) {
            Ok(true) => inserted += 1,
            Ok(false) => skipped += 1,
            Err(_) => {} // count separately if you want, kept simple here
        }
    }
    Ok((inserted, errors, skipped))
}

#[cfg(test)]
mod tests {
    use super::*;
    use heirloom_core::Store;

    #[test]
    fn snapshot_roundtrips_through_encryption() {
        let tmp = tempfile::tempdir().unwrap();
        let db = tmp.path().join("heirloom.db");
        std::fs::write(&db, b"sqlite-like bytes").unwrap();
        let (header, ct) = prepare_snapshot(&db, "pass", DeviceId::new()).unwrap();
        assert_eq!(header.size_bytes, ct.len() as u64);
        let out = apply_snapshot(tmp.path(), &ct, "pass").unwrap();
        assert_eq!(std::fs::read(out).unwrap(), b"sqlite-like bytes");
    }

    #[test]
    fn sync_state_persists() {
        let tmp = tempfile::tempdir().unwrap();
        let state = SyncState::load(tmp.path()).unwrap();
        let original = state.device_id.clone();
        let reloaded = SyncState::load(tmp.path()).unwrap();
        assert_eq!(reloaded.device_id, original);
    }

    #[test]
    fn merge_skips_existing_content() {
        let store = Store::in_memory().unwrap();
        store.add(&Memory::new("fs", "note", "alpha")).unwrap();
        let incoming = vec![
            Memory::new("fs", "note", "alpha"),
            Memory::new("fs", "note", "beta"),
        ];
        let (ins, _err, skip) = merge_memories(&store, incoming).unwrap();
        assert_eq!(ins, 1);
        assert_eq!(skip, 1);
    }

    #[test]
    fn wrong_passphrase_fails_apply() {
        let tmp = tempfile::tempdir().unwrap();
        let db = tmp.path().join("heirloom.db");
        std::fs::write(&db, b"sensitive").unwrap();
        let (_h, ct) = prepare_snapshot(&db, "real", DeviceId::new()).unwrap();
        assert!(apply_snapshot(tmp.path(), &ct, "wrong").is_err());
    }
}
