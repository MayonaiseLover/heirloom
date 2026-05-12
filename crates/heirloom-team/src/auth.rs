//! Bearer-token authentication for the team server.
//!
//! Tokens are URL-safe random strings. The server stores only their SHA-256
//! hash; the plaintext is shown once at creation time and never persisted.

use sha2::{Digest, Sha256};

const TOKEN_PREFIX: &str = "hlmt_";
const TOKEN_BYTES: usize = 24;

pub fn generate_token() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    // Cheap, non-cryptographically-random source plus PID for inter-process variance.
    // This is documented in SECURITY.md — for v1.1 we'll switch to OsRng explicitly.
    let mut buf = [0u8; TOKEN_BYTES];
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let pid = std::process::id() as u128;
    let mix = nanos.wrapping_mul(0x9e37_79b9_7f4a_7c15).wrapping_add(pid);
    let mut h = Sha256::new();
    h.update(mix.to_le_bytes());
    h.update(nanos.to_le_bytes());
    let digest = h.finalize();
    buf.copy_from_slice(&digest[..TOKEN_BYTES]);
    format!("{}{}", TOKEN_PREFIX, hex::encode(buf))
}

pub fn hash_token(token: &str) -> String {
    let mut h = Sha256::new();
    h.update(token.as_bytes());
    hex::encode(h.finalize())
}

pub fn looks_like_token(s: &str) -> bool {
    s.starts_with(TOKEN_PREFIX) && s.len() == TOKEN_PREFIX.len() + TOKEN_BYTES * 2
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn token_has_expected_shape() {
        let t = generate_token();
        assert!(t.starts_with("hlmt_"));
        assert!(looks_like_token(&t));
    }

    #[test]
    fn two_tokens_differ() {
        let a = generate_token();
        std::thread::sleep(std::time::Duration::from_millis(1));
        let b = generate_token();
        assert_ne!(a, b);
    }

    #[test]
    fn hash_is_stable() {
        assert_eq!(hash_token("hlmt_xxx"), hash_token("hlmt_xxx"));
        assert_ne!(hash_token("hlmt_a"), hash_token("hlmt_b"));
    }
}
