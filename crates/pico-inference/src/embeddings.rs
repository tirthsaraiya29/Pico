//! Non-generative deterministic subword hash projection similarity engine.

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

/// Deterministic 64-bit FNV-1a hasher independent of Rust stdlib SipHash stability.
fn fnv1a_bytes(bytes: &[u8]) -> u64 {
    let mut hash: u64 = 0xcbf29ce484222325;
    for &b in bytes {
        hash ^= b as u64;
        hash = hash.wrapping_mul(0x100000001b3);
    }
    hash
}

/// Maps closely related action/hardware terms to canonical domain tokens so
/// lightweight subword projection preserves intent across common phrasing shifts.
fn expand_synonym_token(token: &str) -> Option<&'static str> {
    match token {
        "terminate" | "kill" | "close" | "quit" | "exit" | "end" => Some("stop_proc"),
        "launch" | "start" | "open" | "run" => Some("open_app"),
        "mute" | "silence" | "quiet" => Some("audio_mute"),
        "volume" | "sound" | "loud" | "louder" | "quieter" | "audio" | "speaker" => {
            Some("audio_level")
        }
        "brightness" | "backlight" | "brighter" | "dim" | "dimmer" => Some("display_bright"),
        "calc" | "calculate" | "compute" | "math" | "evaluate" => Some("math_calc"),
        "find" | "search" | "locate" => Some("search_find"),
        "stats" | "specs" | "hardware" | "diagnostic" | "diagnostics" => Some("sys_info"),
        _ => None,
    }
}

/// Lightweight deterministic lexical + subword n-gram hash projection embedder.
/// Not a neural Transformer; projects words, dotted/underscored segments, character 3-grams,
/// and canonical domain tokens into a fixed L2-normalized vector.
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

    fn project_token(&self, vec: &mut [f32], token: &str, weight: f32) {
        let h = fnv1a_bytes(token.as_bytes());
        let idx = (h as usize) % self.dimension;
        let sign = if (h & 1) == 0 { 1.0 } else { -1.0 };
        vec[idx] += sign * weight;
    }

    /// Computes normalized projection vector for a given text snippet.
    pub fn embed(&self, text: &str) -> EmbeddingVector {
        let mut vec = vec![0.0f32; self.dimension];
        let normalized = text.to_lowercase();
        let tokens: Vec<&str> = normalized
            .split(|c: char| !c.is_alphanumeric())
            .filter(|s| s.len() >= 2)
            .collect();

        if tokens.is_empty() {
            return EmbeddingVector::new(vec);
        }

        for &token in &tokens {
            self.project_token(&mut vec, token, 1.5);
            if let Some(canon) = expand_synonym_token(token) {
                self.project_token(&mut vec, canon, 1.2);
            }

            let chars: Vec<char> = token.chars().collect();
            if chars.len() >= 3 {
                for window in chars.windows(3) {
                    let sub: String = window.iter().collect();
                    self.project_token(&mut vec, &sub, 0.65);
                }
            }
        }

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
