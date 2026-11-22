use crate::llamacpp::LlamaCppProvider;
use crate::ollama::OllamaProvider;
use crate::openai::OpenAiCompatibleProvider;
use pico_core::config::InferenceConfig;
use pico_core::traits::InferenceProvider;
use std::sync::Arc;

pub struct InferenceFactory;

impl InferenceFactory {
    pub fn create_provider(config: &InferenceConfig) -> Arc<dyn InferenceProvider> {
        match config.provider.to_lowercase().as_str() {
            "ollama" => Arc::new(OllamaProvider::new(
                &config.endpoint,
                &config.model,
                config.temperature,
                Some(config.system_prompt.clone()),
            )),
            "llamacpp" => {
                let binary = LlamaCppProvider::detect_local_server();
                Arc::new(LlamaCppProvider::new(
                    &config.endpoint,
                    &config.model,
                    config.temperature,
                    Some(config.system_prompt.clone()),
                    binary,
                ))
            }
            _ => Arc::new(OpenAiCompatibleProvider::new(
                &config.endpoint,
                &config.model,
                config.api_key.clone(),
                config.temperature,
                Some(config.system_prompt.clone()),
            )),
        }
    }
}
