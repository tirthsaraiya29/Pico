//! `pico-decision`: Probabilistic Decision Intelligence for Project Pico.
//! Evaluates user intent, dynamically retrieves candidate tools, executes cross-encoder
//! ranking with calibrated probabilities, resolves arguments deterministically, and synthesizes outcomes.

pub mod argument;
pub mod engine;
pub mod index;
pub mod intent;
pub mod synthesizer;

pub use argument::{resolve_arguments, ArgumentResolution};
pub use engine::{DecisionEngine, DecisionResult};
pub use index::{ToolEntry, ToolIndex};
pub use intent::{IntentClassification, IntentClassifier, UserIntent};
pub use synthesizer::ResponseSynthesizer;

#[cfg(test)]
mod tests {
    use super::*;
    use pico_core::config::DecisionConfig;
    use pico_core::traits::PermissionTier;
    use serde_json::json;

    #[tokio::test]
    async fn test_full_decision_flow_math() {
        let config = DecisionConfig::default();
        let engine = DecisionEngine::new(config);

        // Register calculator tool in the index
        {
            let tool_idx = engine.tool_index();
            let mut idx = tool_idx.write().unwrap();
            idx.add_entry(ToolEntry {
                name: "calculator".to_string(),
                description: "Evaluates mathematical arithmetic expressions strictly".to_string(),
                category: "calculator".to_string(),
                os: "all".to_string(),
                permission_tier: PermissionTier::ReadOnly,
                parameters_schema: json!({
                    "type": "object",
                    "properties": {
                        "expression": { "type": "string" }
                    },
                    "required": ["expression"]
                }),
            });
        }

        let result = engine.decide("Calculate 42 * 10").await.unwrap();
        match result {
            DecisionResult::ActionSelected {
                action_name,
                arguments,
                confidence,
                ..
            } => {
                assert_eq!(action_name, "calculator");
                assert_eq!(arguments.get("expression").unwrap(), "42 * 10");
                assert!(confidence > 0.0);
            }
            _ => panic!(
                "Expected ActionSelected for math calculation, got {:?}",
                result
            ),
        }
    }

    #[tokio::test]
    async fn test_direct_conversation_flow() {
        let config = DecisionConfig::default();
        let engine = DecisionEngine::new(config);

        let result = engine.decide("Hello there, who are you?").await.unwrap();
        match result {
            DecisionResult::DirectConversation { response } => {
                assert!(response.contains("Pico"));
            }
            _ => panic!("Expected DirectConversation, got {:?}", result),
        }
    }
}
