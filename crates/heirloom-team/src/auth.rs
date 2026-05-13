//! Bearer-token authentication for the team server.
//!
//! Tokens are URL-safe random strings generated from the OS CSPRNG
//! (`rand::rngs::OsRng`). The server stores only their SHA-256 hash;
//! the plaintext is shown once at creation time and never persisted.

use rand::RngCore;
use sha2::{Digest, Sha256};

const TOKEN_PREFIX: &str = "hlmt_";
const TOKEN_BYTES: usize = 24;

/// Generate a cryptographically secure bearer token.
///
/// Uses `OsRng` which on Linux reads from `getrandom(2)`, on macOS from
/// `/dev/urandom` via `getentropy(2)`, and on Windows from `BCryptGenRandom`.
/// All three are CSPRNGs suitable for security tokens.
pub fn generate_token() -> String {
    let mut buf = [0u8; TOKEN_BYTES];
    rand::rngs::OsRng.fill_bytes(&mut buf);
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
    use std::collections::HashSet;

    #[test]
    fn token_has_expected_shape() {
        let t = generate_token();
        assert!(t.starts_with("hlmt_"));
        assert!(looks_like_token(&t));
        assert_eq!(t.len(), 5 + TOKEN_BYTES * 2);
    }

    #[test]
    fn tokens_are_unique_under_burst() {
        // A weak time-based RNG would collide here; OsRng will not.
        let mut seen = HashSet::new();
        for _ in 0..1000 {
            assert!(seen.insert(generate_token()));
        }
    }

    #[test]
    fn hash_is_stable_but_diverges_on_input_change() {
        assert_eq!(hash_token("hlmt_xxx"), hash_token("hlmt_xxx"));
        assert_ne!(hash_token("hlmt_a"), hash_token("hlmt_b"));
    }

    #[test]
    fn token_has_high_entropy() {
        // 1000 tokens × 24 bytes each = 24,000 bytes — should have all 256 byte values.
        let mut histogram = [0u32; 256];
        for _ in 0..1000 {
            let t = generate_token();
            let hex_body = &t[5..];
            let bytes = hex::decode(hex_body).unwrap();
            for b in bytes {
                histogram[b as usize] += 1;
            }
        }
        let covered = histogram.iter().filter(|&&c| c > 0).count();
        assert!(
            covered > 240,
            "low entropy — only {} distinct byte values seen",
            covered
        );
    }
}
