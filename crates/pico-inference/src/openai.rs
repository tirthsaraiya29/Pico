use crate::structured::StructuredDecisionParser;
use async_trait::async_trait;
use eventsource_stream::Eventsource;
use futures::StreamExt;
use pico_core::error::{PicoError, Result};
use pico_core::session::{Message, Role, ToolCall};
use pico_core::traits::{InferenceProvider, InferenceResponse, TokenStream};
use serde::Deserialize;
use std::time::Duration;

pub struct OpenAiCompatibleProvider {
    endpoint: String,
    model: String,
    api_key: Option<String>,
    client: reqwest::Client,
    temperature: f32,
    system_prompt: Option<String>,
}

impl OpenAiCompatibleProvider {
    pub fn new(
        endpoint: impl Into<String>,
        model: impl Into<String>,
        api_key: Option<String>,
        temperature: f32,
        system_prompt: Option<String>,
    ) -> Self {
        let client = reqwest::Client::builder()
            .no_proxy()
            .timeout(Duration::from_secs(90))
            .build()
            .unwrap_or_default();

        Self {
            endpoint: endpoint.into().trim_end_matches('/').to_string(),
            model: model.into(),
            api_key,
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
                                "arguments": tc.arguments.to_string()
                            }
                        })
                    })
                    .collect();
                obj["tool_calls"] = serde_json::Value::Array(tc_json);
            }

            if let Some(ref call_id) = msg.tool_call_id {
                obj["tool_call_id"] = serde_json::Value::String(call_id.clone());
            }

            list.push(obj);
        }

        list
    }

    fn apply_auth(&self, req: reqwest::RequestBuilder) -> reqwest::RequestBuilder {
        if let Some(ref key) = self.api_key {
            req.header("Authorization", format!("Bearer {}", key))
        } else {
            req
        }
    }
}

#[derive(Deserialize)]
struct ChatCompletionResponse {
    choices: Vec<ChatChoice>,
    usage: Option<Usage>,
}

#[derive(Deserialize)]
struct ChatChoice {
    message: ChatResponseMessage,
}

#[derive(Deserialize)]
struct ChatResponseMessage {
    content: Option<String>,
    tool_calls: Option<Vec<OpenAiToolCall>>,
}

#[derive(Deserialize)]
struct OpenAiToolCall {
    id: String,
    function: OpenAiFunctionCall,
}

#[derive(Deserialize)]
struct OpenAiFunctionCall {
    name: String,
    arguments: String,
}

#[derive(Deserialize)]
struct Usage {
    total_tokens: usize,
}

#[async_trait]
impl InferenceProvider for OpenAiCompatibleProvider {
    fn provider_name(&self) -> &str {
        "openai"
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
        let url = format!("{}/v1/chat/completions", self.endpoint);
        let formatted_messages = self.format_messages(messages, system_prompt);

        let mut body = serde_json::json!({
            "model": self.model,
            "messages": formatted_messages,
            "temperature": self.temperature,
            "stream": true
        });

        if let Some(tool_list) = tools {
            if !tool_list.is_empty() {
                body["tools"] = serde_json::Value::Array(tool_list.to_vec());
            }
        }

        let mut req = self.client.post(&url).json(&body);
        req = self.apply_auth(req);

        let response = req.send().await.map_err(|e| {
            PicoError::Inference(format!("Failed to connect to API at {}: {}", url, e))
        })?;

        if !response.status().is_success() {
            let status = response.status();
            let err_text = response.text().await.unwrap_or_default();
            return Err(PicoError::Inference(format!(
                "Inference API error {}: {}",
                status, err_text
            )));
        }

        let mut event_stream = response.bytes_stream().eventsource();
        let token_stream = async_stream::try_stream! {
            while let Some(event) = event_stream.next().await {
                let event = event.map_err(|e| PicoError::Inference(e.to_string()))?;
                if event.data == "[DONE]" {
                    break;
                }
                if let Ok(val) = serde_json::from_str::<serde_json::Value>(&event.data) {
                    if let Some(delta) = val.pointer("/choices/0/delta/content").and_then(|v| v.as_str()) {
                        if !delta.is_empty() {
                            yield delta.to_string();
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
        let url = format!("{}/v1/chat/completions", self.endpoint);
        let formatted_messages = self.format_messages(messages, system_prompt);

        let mut body = serde_json::json!({
            "model": self.model,
            "messages": formatted_messages,
            "temperature": self.temperature,
            "stream": false
        });

        if let Some(tool_list) = tools {
            if !tool_list.is_empty() {
                body["tools"] = serde_json::Value::Array(tool_list.to_vec());
            }
        }

        let mut req = self.client.post(&url).json(&body);
        req = self.apply_auth(req);

        let response = req.send().await.map_err(|e| {
            PicoError::Inference(format!("Failed to connect to API at {}: {}", url, e))
        })?;

        if !response.status().is_success() {
            let status = response.status();
            let err_text = response.text().await.unwrap_or_default();
            return Err(PicoError::Inference(format!(
                "Inference API error {}: {}",
                status, err_text
            )));
        }

        let resp: ChatCompletionResponse = response
            .json()
            .await
            .map_err(|e| PicoError::Inference(format!("Failed to parse completion JSON: {}", e)))?;

        let choice = resp
            .choices
            .into_iter()
            .next()
            .ok_or_else(|| PicoError::Inference("No choices returned from model".into()))?;

        let content = choice.message.content.unwrap_or_default();
        let mut tool_calls = Vec::new();

        if let Some(native_calls) = choice.message.tool_calls {
            for tc in native_calls {
                let parsed_args: serde_json::Value =
                    serde_json::from_str(&tc.function.arguments).unwrap_or(serde_json::json!({}));
                tool_calls.push(ToolCall {
                    id: tc.id,
                    name: tc.function.name,
                    arguments: parsed_args,
                    reason: None,
                });
            }
        }

        // Fallback to structured parsing if content has embedded tool call
        if tool_calls.is_empty() && !content.is_empty() {
            let parsed = StructuredDecisionParser::parse_tool_calls(&content);
            if !parsed.is_empty() {
                tool_calls = parsed;
            }
        }

        let usage_tokens = resp.usage.map(|u| u.total_tokens);

        Ok(InferenceResponse {
            content,
            tool_calls,
            usage_tokens,
        })
    }
}
