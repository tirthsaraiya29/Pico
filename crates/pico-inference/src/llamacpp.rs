use crate::openai::OpenAiCompatibleProvider;
use async_trait::async_trait;
use pico_core::error::Result;
use pico_core::session::Message;
use pico_core::traits::{InferenceProvider, InferenceResponse, TokenStream};
use std::path::PathBuf;
pub struct LlamaCppProvider {
    inner: OpenAiCompatibleProvider,
    _binary_path: Option<PathBuf>,
}

impl LlamaCppProvider {
    pub fn new(
        endpoint: impl Into<String>,
        model: impl Into<String>,
        temperature: f32,
        system_prompt: Option<String>,
        binary_path: Option<PathBuf>,
    ) -> Self {
        let ep = endpoint.into();
        let inner = OpenAiCompatibleProvider::new(ep, model, None, temperature, system_prompt);

        Self {
            inner,
            _binary_path: binary_path,
        }
    }

    /// Helper to find default local llama-server binary if present.
    pub fn detect_local_server() -> Option<PathBuf> {
        let candidate = PathBuf::from("C:\\llama.cpp\\build\\bin\\Release\\llama-server.exe");
        if candidate.exists() {
            Some(candidate)
        } else {
            None
        }
    }
}

#[async_trait]
impl InferenceProvider for LlamaCppProvider {
    fn provider_name(&self) -> &str {
        "llamacpp"
    }

    fn model_name(&self) -> &str {
        self.inner.model_name()
    }

    async fn stream_completion(
        &self,
        messages: &[Message],
        system_prompt: Option<&str>,
        tools: Option<&[serde_json::Value]>,
    ) -> Result<TokenStream> {
        self.inner
            .stream_completion(messages, system_prompt, tools)
            .await
    }

    async fn generate_response(
        &self,
        messages: &[Message],
        system_prompt: Option<&str>,
        tools: Option<&[serde_json::Value]>,
    ) -> Result<InferenceResponse> {
        self.inner
            .generate_response(messages, system_prompt, tools)
            .await
    }
}
