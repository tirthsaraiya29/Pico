//! Non-generative machine learning & ranking inference crate for Pico.
//! Focuses on dense semantic embeddings, multi-feature cross-encoder scoring, and probability calibration.

pub mod calibration;
pub mod embeddings;
pub mod ranking;

pub use calibration::Calibrator;
pub use embeddings::{EmbeddingVector, SemanticEmbedder, EMBEDDING_DIM};
pub use ranking::{CrossEncoderRanker, RankingWeights};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_inference_pipeline_end_to_end() {
        let ranker = CrossEncoderRanker::default();
        let (raw, sem, lex) = ranker.score_candidate(
            "What is the CPU and memory utilization?",
            "sys.get_system_info",
            "Retrieves operating system details, CPU load, and available RAM statistics",
            1.0,
            1.0,
        );

        assert!(raw > 0.5);
        assert!(sem > 0.0);
        assert!(lex > 0.0);

        let probs = Calibrator::softmax_calibrate(&[raw, 0.1, 0.05], 1.0);
        assert_eq!(probs.len(), 3);
        assert!(probs[0] > probs[1]);
    }
}
