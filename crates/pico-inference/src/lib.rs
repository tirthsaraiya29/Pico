pub mod factory;
pub mod llamacpp;
pub mod ollama;
pub mod openai;
pub mod structured;

pub use factory::InferenceFactory;
pub use llamacpp::LlamaCppProvider;
pub use ollama::OllamaProvider;
pub use openai::OpenAiCompatibleProvider;
pub use structured::StructuredDecisionParser;

#[cfg(test)]
mod tests {
    use super::*;
    use pico_core::config::InferenceConfig;

    #[test]
    fn test_inference_factory_creation() {
        let cfg1 = InferenceConfig {
            provider: "ollama".into(),
            ..Default::default()
        };
        let provider = InferenceFactory::create_provider(&cfg1);
        assert_eq!(provider.provider_name(), "ollama");

        let cfg2 = InferenceConfig {
            provider: "openai".into(),
            ..Default::default()
        };
        let openai_provider = InferenceFactory::create_provider(&cfg2);
        assert_eq!(openai_provider.provider_name(), "openai");
    }

    #[tokio::test]
    async fn test_live_ollama_call() {
        use pico_core::traits::InferenceProvider;
        let provider =
            OllamaProvider::new("http://127.0.0.1:11434", "granite4.2:latest", 0.2, None);
        let msgs = vec![pico_core::session::Message::new_user(
            pico_core::session::SessionId::new(),
            "Respond with the word PICO_TEST_OK and nothing else",
        )];
        match provider.generate_response(&msgs, None, None).await {
            Ok(resp) => println!("LIVE OLLAMA SUCCESS: {}", resp.content),
            Err(e) => println!("LIVE OLLAMA ERROR: {:?}", e),
        }
    }
}
