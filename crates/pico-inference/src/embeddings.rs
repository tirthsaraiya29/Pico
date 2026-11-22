//! Non-generative semantic embedding and vector similarity engine.
//! Optimized for local sub-millisecond execution on Intel Core Ultra 7 CPU.

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};

pub const EMBEDDING_DIM: usize = 128;

/// Dense semantic feature vector for text representation.
#[derive(Debug, Clone, PartialEq)]
pub struct EmbeddingVector {
    pub values: Vec<f32>,
}

impl EmbeddingVector {
    pub fn new(values: Vec<f32>) -> Self {
        Self { values }
    }

    /// Computes cosine similarity between two normalized vectors: cos(θ) ∈ [-1.0, 1.0].
    pub fn cosine_similarity(&self, other: &EmbeddingVector) -> f32 {
        if self.values.len() != other.values.len() || self.values.is_empty() {
            return 0.0;
        }

        let mut dot = 0.0f32;
        let mut norm_a = 0.0f32;
        let mut norm_b = 0.0f32;

        for (a, b) in self.values.iter().zip(other.values.iter()) {
            dot += a * b;
            norm_a += a * a;
            norm_b += b * b;
        }

        let denom = norm_a.sqrt() * norm_b.sqrt();
        if denom < 1e-12 {
            0.0
        } else {
            (dot / denom).clamp(-1.0, 1.0)
        }
    }
}

/// Lightweight semantic embedding model using subword n-gram hashing and dimensional projection.
#[derive(Debug, Clone)]
pub struct SemanticEmbedder {
    dimension: usize,
}

impl Default for SemanticEmbedder {
    fn default() -> Self {
        Self::new(EMBEDDING_DIM)
    }
}

impl SemanticEmbedder {
    pub fn new(dimension: usize) -> Self {
        Self { dimension }
    }

    /// Computes dense embedding vector for a given text snippet.
    pub fn embed(&self, text: &str) -> EmbeddingVector {
        let mut vec = vec![0.0f32; self.dimension];
        let normalized = text.to_lowercase();
        let tokens: Vec<&str> = normalized
            .split(|c: char| !c.is_alphanumeric() && c != '_')
            .filter(|s| !s.is_empty())
            .collect();

        if tokens.is_empty() {
            return EmbeddingVector::new(vec);
        }

        for token in &tokens {
            // Word-level hash projection
            let mut hasher = DefaultHasher::new();
            token.hash(&mut hasher);
            let h = hasher.finish();
            let idx = (h as usize) % self.dimension;
            let sign = if (h & 1) == 0 { 1.0 } else { -1.0 };
            vec[idx] += sign * 1.5;

            // Character 3-gram subwords for morphological semantic match
            let chars: Vec<char> = token.chars().collect();
            if chars.len() >= 3 {
                for window in chars.windows(3) {
                    let mut sub_hasher = DefaultHasher::new();
                    for c in window {
                        c.hash(&mut sub_hasher);
                    }
                    let sub_h = sub_hasher.finish();
                    let sub_idx = (sub_h as usize) % self.dimension;
                    let sub_sign = if (sub_h & 1) == 0 { 0.7 } else { -0.7 };
                    vec[sub_idx] += sub_sign;
                }
            }
        }

        // L2 Normalization
        let norm: f32 = vec.iter().map(|v| v * v).sum::<f32>().sqrt();
        if norm > 1e-12 {
            for v in &mut vec {
                *v /= norm;
            }
        }

        EmbeddingVector::new(vec)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_semantic_similarity_ranking() {
        let embedder = SemanticEmbedder::default();

        let q = embedder.embed("terminate running process");
        let target_match = embedder.embed("kill process task termination");
        let unrelated = embedder.embed("bake a chocolate strawberry cake");

        let sim_target = q.cosine_similarity(&target_match);
        let sim_unrelated = q.cosine_similarity(&unrelated);

        assert!(
            sim_target > sim_unrelated,
            "Expected semantic similarity for matching domain ({} > {})",
            sim_target,
            sim_unrelated
        );
    }
}
