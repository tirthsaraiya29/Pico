use crate::structured::StructuredDecisionParser;
use async_trait::async_trait;
use pico_core::error::{PicoError, Result};
use pico_core::session::{Message, Role, ToolCall};
use pico_core::traits::{InferenceProvider, InferenceResponse, TokenStream};
use serde::Deserialize;
use std::time::Duration;
use tracing::debug;

pub struct OllamaProvider {
    endpoint: String,
    model: String,
    client: reqwest::Client,
    temperature: f32,
    system_prompt: Option<String>,
}

impl OllamaProvider {
    pub fn new(
        endpoint: impl Into<String>,
        model: impl Into<String>,
        temperature: f32,
        system_prompt: Option<String>,
    ) -> Self {
        let client = reqwest::Client::builder()
            .no_proxy()
            .timeout(Duration::from_secs(180))
            .build()
            .unwrap_or_default();

        Self {
            endpoint: endpoint.into().trim_end_matches('/').to_string(),
            model: model.into(),
            client,
            temperature,
            system_prompt,
        }
    }

    fn format_messages(
        &self,
        messages: &[Message],
        override_system: Option<&str>,
    ) -> Vec<serde_json::Value> {
        let mut list = Vec::new();

        // Insert system prompt
        let sys = override_system.or(self.system_prompt.as_deref());
        if let Some(sys_content) = sys {
            list.push(serde_json::json!({
                "role": "system",
                "content": sys_content
            }));
        }

        for msg in messages {
            let role_str = match msg.role {
                Role::System => "system",
                Role::User => "user",
                Role::Assistant => "assistant",
                Role::Tool => "tool",
            };

            let mut obj = serde_json::json!({
                "role": role_str,
                "content": msg.content
            });

            if let Some(ref tool_calls) = msg.tool_calls {
                let tc_json: Vec<serde_json::Value> = tool_calls
                    .iter()
                    .map(|tc| {
                        serde_json::json!({
                            "id": tc.id,
                            "type": "function",
                            "function": {
                                "name": tc.name,
                                "arguments": tc.arguments
                            }
                        })
                    })
                    .collect();
                obj["tool_calls"] = serde_json::Value::Array(tc_json);
            }

            list.push(obj);
        }

        list
    }
}

#[derive(Deserialize)]
struct OllamaChatResponse {
    message: Option<OllamaResponseMessage>,
    #[allow(dead_code)]
    prompt_eval_count: Option<usize>,
    eval_count: Option<usize>,
}

#[derive(Deserialize)]
struct OllamaResponseMessage {
    content: String,
    tool_calls: Option<Vec<OllamaToolCall>>,
}

#[derive(Deserialize)]
struct OllamaToolCall {
    function: OllamaFunctionCall,
}

#[derive(Deserialize)]
struct OllamaFunctionCall {
    name: String,
    arguments: serde_json::Value,
}

#[async_trait]
impl InferenceProvider for OllamaProvider {
    fn provider_name(&self) -> &str {
        "ollama"
    }

    fn model_name(&self) -> &str {
        &self.model
    }

    async fn stream_completion(
        &self,
        messages: &[Message],
        system_prompt: Option<&str>,
        tools: Option<&[serde_json::Value]>,
    ) -> Result<TokenStream> {
        let url = format!("{}/api/chat", self.endpoint);
        let formatted_messages = self.format_messages(messages, system_prompt);

        let mut body = serde_json::json!({
            "model": self.model,
            "messages": formatted_messages,
            "stream": true,
            "options": {
                "temperature": self.temperature
            }
        });

        if let Some(tool_list) = tools {
            if !tool_list.is_empty() {
                body["tools"] = serde_json::Value::Array(tool_list.to_vec());
            }
        }

        let response = self
            .client
            .post(&url)
            .json(&body)
            .send()
            .await
            .map_err(|e| {
                PicoError::Inference(format!("Failed to connect to Ollama at {}: {:?}", url, e))
            })?;

        if !response.status().is_success() {
            let status = response.status();
            let err_text = response.text().await.unwrap_or_default();
            return Err(PicoError::Inference(format!(
                "Ollama responded with HTTP {}: {}",
                status, err_text
            )));
        }

        // Stream NDJSON chunks
        let byte_stream = response.bytes_stream();
        let token_stream = async_stream::try_stream! {
            let mut buffer = String::new();
            for await chunk_result in byte_stream {
                let bytes = chunk_result.map_err(|e| PicoError::Inference(e.to_string()))?;
                let text = String::from_utf8_lossy(&bytes);
                buffer.push_str(&text);

                while let Some(pos) = buffer.find('\n') {
                    let line = buffer[..pos].trim().to_string();
                    buffer = buffer[pos + 1..].to_string();

                    if !line.is_empty() {
                        if let Ok(val) = serde_json::from_str::<serde_json::Value>(&line) {
                            if let Some(content) = val.pointer("/message/content").and_then(|v| v.as_str()) {
                                if !content.is_empty() {
                                    yield content.to_string();
                                }
                            }
                        }
                    }
                }
            }
        };

        Ok(Box::pin(token_stream))
    }

    async fn generate_response(
        &self,
        messages: &[Message],
        system_prompt: Option<&str>,
        tools: Option<&[serde_json::Value]>,
    ) -> Result<InferenceResponse> {
        let url = format!("{}/api/chat", self.endpoint);
        let formatted_messages = self.format_messages(messages, system_prompt);

        let mut body = serde_json::json!({
            "model": self.model,
            "messages": formatted_messages,
            "stream": false,
            "options": {
                "temperature": self.temperature
            }
        });

        if let Some(tool_list) = tools {
            if !tool_list.is_empty() {
                body["tools"] = serde_json::Value::Array(tool_list.to_vec());
            }
        }

        let response = self
            .client
            .post(&url)
            .json(&body)
            .send()
            .await
            .map_err(|e| {
                PicoError::Inference(format!("Failed to connect to Ollama at {}: {:?}", url, e))
            })?;

        if !response.status().is_success() {
            let status = response.status();
            let err_text = response.text().await.unwrap_or_default();
            return Err(PicoError::Inference(format!(
                "Ollama error HTTP {}: {}",
                status, err_text
            )));
        }

        let resp_json: OllamaChatResponse = response.json().await.map_err(|e| {
            PicoError::Inference(format!("Failed to parse Ollama JSON response: {}", e))
        })?;

        let mut content = String::new();
        let mut tool_calls = Vec::new();

        if let Some(msg) = resp_json.message {
            content = msg.content;
            if let Some(native_calls) = msg.tool_calls {
                for tc in native_calls {
                    tool_calls.push(ToolCall {
                        id: format!("call_{}", uuid::Uuid::new_v4().simple()),
                        name: tc.function.name,
                        arguments: tc.function.arguments,
                        reason: None,
                    });
                }
            }
        }

        // If no native tool calls were returned, run structured parser over the generated content
        if tool_calls.is_empty() && !content.is_empty() {
            let parsed = StructuredDecisionParser::parse_tool_calls(&content);
            if !parsed.is_empty() {
                debug!(
                    count = parsed.len(),
                    "Extracted fallback tool calls from text response"
                );
                tool_calls = parsed;
            }
        }

        let usage = resp_json.eval_count;

        Ok(InferenceResponse {
            content,
            tool_calls,
            usage_tokens: usage,
        })
    }
}
