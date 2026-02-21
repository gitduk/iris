//! Embedding generation (v1: deterministic SHA-256 hash placeholder).
//!
//! Will be replaced with real vector embeddings (e.g. OpenAI text-embedding-3-small)
//! once the LLM provider trait supports embedding endpoints.

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};

/// Embedding dimension for the v1 placeholder.
const EMBED_DIM: usize = 32;

/// Generate a placeholder embedding from text content.
///
/// Produces a deterministic 32-byte vector by hashing the content with
/// multiple seeds. Same input always yields the same output.
pub fn generate(content: &str) -> Vec<u8> {
    let mut embedding = Vec::with_capacity(EMBED_DIM);
    for seed in 0..EMBED_DIM {
        let mut hasher = DefaultHasher::new();
        seed.hash(&mut hasher);
        content.hash(&mut hasher);
        embedding.push((hasher.finish() % 256) as u8);
    }
    embedding
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deterministic() {
        let a = generate("hello world");
        let b = generate("hello world");
        assert_eq!(a, b);
    }

    #[test]
    fn correct_dimension() {
        let v = generate("test input");
        assert_eq!(v.len(), EMBED_DIM);
    }

    #[test]
    fn different_inputs_differ() {
        let a = generate("hello");
        let b = generate("world");
        assert_ne!(a, b);
    }

    #[test]
    fn empty_input_works() {
        let v = generate("");
        assert_eq!(v.len(), EMBED_DIM);
    }
}
