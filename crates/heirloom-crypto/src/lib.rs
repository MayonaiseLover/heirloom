//! # heirloom-crypto
//!
//! At-rest encryption for the Heirloom database. Uses **XChaCha20-Poly1305**
//! authenticated encryption with an **Argon2id**-derived key.
//!
//! Operations:
//! - `seal`: encrypt `heirloom.db` → `heirloom.db.hlm`, then shred the plaintext.
//! - `unseal`: decrypt `heirloom.db.hlm` → `heirloom.db` for live use.
//!
//! SQLite needs random access, so Heirloom doesn't run with the DB in
//! encrypted form. The workflow is: `unseal` → use → `seal`. An offline
//! attacker who steals only the `.hlm` file cannot read your memories
//! without your passphrase.
//!
//! ## File format (`.hlm v1`)
//!
//! ```text
//! offset  bytes  contents
//! ------  -----  --------
//!  0       4     magic "HLM\x01"
//!  4       1     version (1)
//!  5      16     argon2id salt
//! 21      24     XChaCha20 nonce
//! 45      4      reserved (zeros)
//! 49      ...    XChaCha20-Poly1305 ciphertext + 16-byte auth tag
//! ```
//!
//! Argon2id parameters: m=64 MiB, t=3, p=1. ~150 ms on a modern laptop —
//! slow enough to deter brute force, fast enough to feel instant on unseal.

use anyhow::{Context, Result};
use argon2::{Algorithm, Argon2, Params, Version};
use chacha20poly1305::aead::{Aead, KeyInit};
use chacha20poly1305::{XChaCha20Poly1305, XNonce};
use rand::RngCore;
use std::io::Write;
use std::path::{Path, PathBuf};
use thiserror::Error;

pub const SEALED_SUFFIX: &str = ".hlm";
const MAGIC: [u8; 4] = *b"HLM\x01";
const VERSION: u8 = 1;
const SALT_LEN: usize = 16;
const NONCE_LEN: usize = 24;
const HEADER_LEN: usize = 4 + 1 + SALT_LEN + NONCE_LEN + 4;
const KEY_LEN: usize = 32;

#[derive(Error, Debug)]
pub enum CryptoError {
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    #[error("argon2: {0}")]
    Kdf(String),
    #[error("aead: ciphertext rejected — wrong passphrase or corrupted file")]
    Aead,
    #[error("bad header: {0}")]
    Header(&'static str),
    #[error("file already exists: {0}")]
    Exists(PathBuf),
    #[error("file not found: {0}")]
    NotFound(PathBuf),
}

pub fn seal(plain_path: &Path, passphrase: &str, keep_plaintext: bool) -> Result<PathBuf> {
    if !plain_path.exists() {
        return Err(CryptoError::NotFound(plain_path.to_path_buf()).into());
    }
    let out_path = sealed_path_for(plain_path);
    if out_path.exists() {
        return Err(CryptoError::Exists(out_path).into());
    }

    let mut salt = [0u8; SALT_LEN];
    let mut nonce = [0u8; NONCE_LEN];
    rand::thread_rng().fill_bytes(&mut salt);
    rand::thread_rng().fill_bytes(&mut nonce);

    let key = derive_key(passphrase, &salt)?;
    let cipher = XChaCha20Poly1305::new((&key).into());

    let plaintext =
        std::fs::read(plain_path).with_context(|| format!("reading {}", plain_path.display()))?;
    let ciphertext = cipher
        .encrypt(XNonce::from_slice(&nonce), plaintext.as_ref())
        .map_err(|_| CryptoError::Aead)?;

    let mut f = std::fs::File::create(&out_path)
        .with_context(|| format!("creating {}", out_path.display()))?;
    f.write_all(&MAGIC)?;
    f.write_all(&[VERSION])?;
    f.write_all(&salt)?;
    f.write_all(&nonce)?;
    f.write_all(&[0u8; 4])?; // reserved
    f.write_all(&ciphertext)?;
    f.flush()?;

    if !keep_plaintext {
        shred(plain_path)?;
    }
    Ok(out_path)
}

pub fn unseal(plain_path: &Path, passphrase: &str) -> Result<()> {
    let sealed_path = sealed_path_for(plain_path);
    if !sealed_path.exists() {
        return Err(CryptoError::NotFound(sealed_path).into());
    }
    if plain_path.exists() {
        return Err(CryptoError::Exists(plain_path.to_path_buf()).into());
    }

    let buf = std::fs::read(&sealed_path)
        .with_context(|| format!("reading {}", sealed_path.display()))?;
    if buf.len() < HEADER_LEN + 16 {
        return Err(CryptoError::Header("file too short").into());
    }
    if buf[..4] != MAGIC {
        return Err(CryptoError::Header("magic mismatch").into());
    }
    if buf[4] != VERSION {
        return Err(CryptoError::Header("unsupported version").into());
    }
    let salt = &buf[5..5 + SALT_LEN];
    let nonce = &buf[5 + SALT_LEN..5 + SALT_LEN + NONCE_LEN];
    let ciphertext = &buf[HEADER_LEN..];

    let key = derive_key(passphrase, salt)?;
    let cipher = XChaCha20Poly1305::new((&key).into());
    let plaintext = cipher
        .decrypt(XNonce::from_slice(nonce), ciphertext)
        .map_err(|_| CryptoError::Aead)?;

    std::fs::write(plain_path, plaintext)?;
    Ok(())
}

pub fn sealed_path_for(plain: &Path) -> PathBuf {
    let mut s = plain.as_os_str().to_owned();
    s.push(SEALED_SUFFIX);
    PathBuf::from(s)
}

/// Encrypt an in-memory byte slice. Returns the full `.hlm` framed bytes.
/// Used by the sync layer to produce snapshot blobs.
pub fn seal_bytes(plaintext: &[u8], passphrase: &str) -> Result<Vec<u8>> {
    let mut salt = [0u8; SALT_LEN];
    let mut nonce = [0u8; NONCE_LEN];
    rand::thread_rng().fill_bytes(&mut salt);
    rand::thread_rng().fill_bytes(&mut nonce);

    let key = derive_key(passphrase, &salt)?;
    let cipher = XChaCha20Poly1305::new((&key).into());
    let ciphertext = cipher
        .encrypt(XNonce::from_slice(&nonce), plaintext)
        .map_err(|_| CryptoError::Aead)?;

    let mut out = Vec::with_capacity(HEADER_LEN + ciphertext.len());
    out.extend_from_slice(&MAGIC);
    out.push(VERSION);
    out.extend_from_slice(&salt);
    out.extend_from_slice(&nonce);
    out.extend_from_slice(&[0u8; 4]);
    out.extend_from_slice(&ciphertext);
    Ok(out)
}

/// Decrypt an in-memory `.hlm` blob produced by [`seal_bytes`].
pub fn unseal_bytes(sealed: &[u8], passphrase: &str) -> Result<Vec<u8>> {
    if sealed.len() < HEADER_LEN + 16 {
        return Err(CryptoError::Header("file too short").into());
    }
    if sealed[..4] != MAGIC {
        return Err(CryptoError::Header("magic mismatch").into());
    }
    if sealed[4] != VERSION {
        return Err(CryptoError::Header("unsupported version").into());
    }
    let salt = &sealed[5..5 + SALT_LEN];
    let nonce = &sealed[5 + SALT_LEN..5 + SALT_LEN + NONCE_LEN];
    let ciphertext = &sealed[HEADER_LEN..];
    let key = derive_key(passphrase, salt)?;
    let cipher = XChaCha20Poly1305::new((&key).into());
    cipher
        .decrypt(XNonce::from_slice(nonce), ciphertext)
        .map_err(|_| CryptoError::Aead.into())
}

fn derive_key(passphrase: &str, salt: &[u8]) -> Result<[u8; KEY_LEN]> {
    let params =
        Params::new(64 * 1024, 3, 1, Some(KEY_LEN)).map_err(|e| CryptoError::Kdf(e.to_string()))?;
    let argon = Argon2::new(Algorithm::Argon2id, Version::V0x13, params);
    let mut key = [0u8; KEY_LEN];
    argon
        .hash_password_into(passphrase.as_bytes(), salt, &mut key)
        .map_err(|e| CryptoError::Kdf(e.to_string()))?;
    Ok(key)
}

fn shred(path: &Path) -> Result<()> {
    let len = std::fs::metadata(path)?.len();
    {
        let mut f = std::fs::OpenOptions::new().write(true).open(path)?;
        let zeros = vec![0u8; 8192];
        let mut remaining = len;
        while remaining > 0 {
            let n = std::cmp::min(remaining, zeros.len() as u64) as usize;
            f.write_all(&zeros[..n])?;
            remaining -= n as u64;
        }
        f.flush()?;
    }
    std::fs::remove_file(path)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn seal_then_unseal_roundtrip() {
        let tmp = tempfile::tempdir().unwrap();
        let plain = tmp.path().join("heirloom.db");
        std::fs::write(&plain, b"hello secret world").unwrap();
        let sealed = seal(&plain, "correct horse battery staple", false).unwrap();
        assert!(sealed.exists());
        assert!(!plain.exists());
        unseal(&plain, "correct horse battery staple").unwrap();
        assert_eq!(std::fs::read(&plain).unwrap(), b"hello secret world");
    }

    #[test]
    fn wrong_passphrase_fails() {
        let tmp = tempfile::tempdir().unwrap();
        let plain = tmp.path().join("heirloom.db");
        std::fs::write(&plain, b"secret").unwrap();
        let _ = seal(&plain, "real", false).unwrap();
        let result = unseal(&plain, "wrong");
        assert!(result.is_err());
    }

    #[test]
    fn refuses_to_overwrite_existing_sealed_file() {
        let tmp = tempfile::tempdir().unwrap();
        let plain = tmp.path().join("heirloom.db");
        std::fs::write(&plain, b"x").unwrap();
        std::fs::write(sealed_path_for(&plain), b"already there").unwrap();
        let result = seal(&plain, "p", false);
        assert!(result.is_err());
    }

    #[test]
    fn corrupted_ciphertext_rejected() {
        let tmp = tempfile::tempdir().unwrap();
        let plain = tmp.path().join("heirloom.db");
        std::fs::write(&plain, b"secret data").unwrap();
        let sealed = seal(&plain, "pass", false).unwrap();
        // Flip a byte in the ciphertext region.
        let mut buf = std::fs::read(&sealed).unwrap();
        let last_byte_idx = buf.len() - 1;
        buf[last_byte_idx] ^= 0xff;
        std::fs::write(&sealed, buf).unwrap();
        let result = unseal(&plain, "pass");
        assert!(result.is_err());
    }
}
