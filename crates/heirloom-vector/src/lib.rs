//! # heirloom-vector
//!
//! A lightweight, pure-Rust vector layer for Heirloom. Embeds each memory
//! into a dense `f32` vector via **hash-projected word and character n-gram
//! TF-IDF**, then ranks queries by cosine similarity.
//!
//! ## Why not BERT?
//!
//! Transformer embeddings (BGE, MiniLM) give better semantic recall, but
//! they require an ONNX runtime (~50 MB) and a model file (~30 MB). For
//! v0.2, Heirloom ships a zero-dependency vector layer that captures
//! lexical and morphological similarity without pulling those weights.
//! The [`Embedder`] trait is the seam — drop in a `fastembed::TextEmbedding`
//! later and the rest of the pipeline doesn't care.
//!
//! ## How it works
//!
//! 1. Tokenize text into lowercased word and char-trigram tokens.
//! 2. Apply IDF weighting using accumulated document frequency stats.
//! 3. Hash each token into a fixed `DIM`-sized vector with sign-flip projection
//!    (so collisions partially cancel, à la feature hashing).
//! 4. L2-normalize the result. Cosine similarity is then a plain dot product.
//!
//! This isn't transformer-quality, but it picks up morphological matches
//! that pure FTS5 misses ("Postgres" vs "postgres" vs "postgresql"; "auth"
//! vs "authentication"), which is what users actually notice.

use heirloom_core::Memory;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::hash::{Hash, Hasher};

/// Dimensionality of the produced vectors. 256 is enough for hashing-based
/// representations and keeps the per-memory storage cost at 1 KiB.
pub const DIM: usize = 256;

/// Contract for any embedding implementation. The default ships with
/// Heirloom; the trait exists so v0.3 can swap in a BERT/BGE backend.
pub trait Embedder: Send + Sync {
    fn embed(&self, text: &str) -> Vec<f32>;
    fn dim(&self) -> usize;
}

/// Hash-projected n-gram embedder. Cheap, deterministic, no model file.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct HashEmbedder {
    /// Optional IDF table — token → log( N / (1 + df) ).
    /// When empty the embedder degrades to pure TF (still useful).
    pub idf: HashMap<String, f32>,
}

impl HashEmbedder {
    pub fn new() -> Self {
        Self::default()
    }

    /// Build the IDF table from an iterator of memories. Idempotent.
    pub fn fit<'a>(&mut self, memories: impl IntoIterator<Item = &'a Memory>) {
        let mut df: HashMap<String, u32> = HashMap::new();
        let mut n = 0u32;
        for m in memories {
            n += 1;
            let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
            for t in tokenize(&m.content) {
                if seen.insert(t.clone()) {
                    *df.entry(t).or_insert(0) += 1;
                }
            }
        }
        self.idf.clear();
        let n_f = (n as f32).max(1.0);
        for (token, df_count) in df {
            // Smoothed IDF: log( (N + 1) / (df + 1) ) + 1
            let idf = ((n_f + 1.0) / (df_count as f32 + 1.0)).ln() + 1.0;
            self.idf.insert(token, idf);
        }
    }
}

impl Embedder for HashEmbedder {
    fn embed(&self, text: &str) -> Vec<f32> {
        let mut v = vec![0f32; DIM];
        let mut tf: HashMap<String, u32> = HashMap::new();
        for t in tokenize(text) {
            *tf.entry(t).or_insert(0) += 1;
        }
        for (token, count) in tf {
            let weight = (count as f32).sqrt() * self.idf.get(&token).copied().unwrap_or(1.0);
            let (bucket, sign) = bucket_and_sign(&token);
            v[bucket] += sign * weight;
        }
        l2_normalize(&mut v);
        v
    }

    fn dim(&self) -> usize {
        DIM
    }
}

/// Cosine similarity between two L2-normalized vectors of equal length.
/// Equivalent to a dot product after normalization.
pub fn cosine(a: &[f32], b: &[f32]) -> f32 {
    debug_assert_eq!(a.len(), b.len());
    a.iter().zip(b.iter()).map(|(x, y)| x * y).sum()
}

/// Combine BM25 (already normalized so higher = better) with cosine.
/// `alpha` weights the BM25 contribution; (1 - alpha) weights the vector.
/// Default `0.5` is a sensible blend.
pub fn hybrid_score(bm25: f32, cos: f32, alpha: f32) -> f32 {
    let a = alpha.clamp(0.0, 1.0);
    a * bm25 + (1.0 - a) * cos
}

fn l2_normalize(v: &mut [f32]) {
    let norm = v.iter().map(|x| x * x).sum::<f32>().sqrt();
    if norm > f32::EPSILON {
        for x in v.iter_mut() {
            *x /= norm;
        }
    }
}

fn bucket_and_sign(token: &str) -> (usize, f32) {
    let mut h = std::collections::hash_map::DefaultHasher::new();
    token.hash(&mut h);
    let raw = h.finish();
    let bucket = (raw as usize) % DIM;
    let sign = if (raw >> 1) & 1 == 0 { 1.0 } else { -1.0 };
    (bucket, sign)
}

fn tokenize(text: &str) -> impl Iterator<Item = String> + '_ {
    let lower = text.to_lowercase();
    let words: Vec<String> = lower
        .split(|c: char| !c.is_alphanumeric())
        .filter(|s| s.len() >= 2 && s.len() <= 32)
        .map(|s| format!("w:{}", s))
        .collect();
    let mut trigrams: Vec<String> = Vec::new();
    for w in words.iter().take(64) {
        let w = w.trim_start_matches("w:");
        if w.len() < 4 {
            continue;
        }
        let chars: Vec<char> = w.chars().collect();
        for window in chars.windows(3) {
            let tg: String = window.iter().collect();
            trigrams.push(format!("c:{}", tg));
        }
    }
    words.into_iter().chain(trigrams)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn embeds_to_correct_dim() {
        let e = HashEmbedder::new();
        let v = e.embed("hello world");
        assert_eq!(v.len(), DIM);
    }

    #[test]
    fn similar_texts_have_higher_cosine() {
        let e = HashEmbedder::new();
        let a = e.embed("the authentication refactor used PKCE for the OAuth flow");
        let b = e.embed("auth refactoring with PKCE in OAuth");
        let c = e.embed("baking a sourdough loaf for the weekend");
        let ab = cosine(&a, &b);
        let ac = cosine(&a, &c);
        assert!(
            ab > ac,
            "auth-vs-auth ({:.3}) should beat auth-vs-baking ({:.3})",
            ab,
            ac
        );
    }

    #[test]
    fn morphological_match_via_trigrams() {
        let e = HashEmbedder::new();
        let q = e.embed("postgres");
        let d1 = e.embed("postgresql tuning notes");
        let d2 = e.embed("redis cluster failover");
        assert!(cosine(&q, &d1) > cosine(&q, &d2));
    }

    #[test]
    fn idf_fit_downweights_common_words() {
        let mut e = HashEmbedder::new();
        let m1 = Memory::new("fs", "note", "the quick brown fox");
        let m2 = Memory::new("fs", "note", "the lazy fox sleeps");
        let m3 = Memory::new("fs", "note", "another the entry");
        e.fit([&m1, &m2, &m3]);
        let the = e.idf.get("w:the").copied().unwrap_or(0.0);
        let fox = e.idf.get("w:fox").copied().unwrap_or(0.0);
        assert!(
            fox > the,
            "fox ({:.3}) should outweigh the ({:.3})",
            fox,
            the
        );
    }

    #[test]
    fn hybrid_score_blends() {
        assert_eq!(hybrid_score(2.0, 0.0, 1.0), 2.0);
        assert_eq!(hybrid_score(0.0, 0.8, 0.0), 0.8);
        assert!((hybrid_score(2.0, 0.8, 0.5) - 1.4).abs() < 1e-5);
    }
}
