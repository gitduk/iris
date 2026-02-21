//! HTTP-based LLM providers.
//!
//! Supports OpenAI-compatible APIs (OpenAI, Google Gemini, DeepSeek, etc.)
//! and Anthropic's native Messages API.

use crate::provider::{
    CompletionRequest, CompletionResponse, ContentBlock, LlmError, LlmProvider, Role, StopReason,
    ToolDefinition,
};
use serde::{Deserialize, Serialize};
use std::future::Future;
use std::pin::Pin;

/// Inferred provider kind from model name.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProviderKind {
    OpenAi,
    Anthropic,
    Google,
    DeepSeek,
    /// Falls back to OpenAI-compatible format.
    Unknown,
}

impl ProviderKind {
    /// Infer provider from model name prefix.
    pub fn from_model(model: &str) -> Self {
        let m = model.to_lowercase();
        if m.starts_with("gpt-")
            || m.starts_with("o1-")
            || m.starts_with("o3-")
            || m.starts_with("o4-")
        {
            Self::OpenAi
        } else if m.starts_with("claude-") {
            Self::Anthropic
        } else if m.starts_with("gemini-") {
            Self::Google
        } else if m.starts_with("deepseek-") {
            Self::DeepSeek
        } else {
            Self::Unknown
        }
    }

    fn default_base_url(self) -> &'static str {
        match self {
            Self::OpenAi | Self::Unknown => "https://api.openai.com/v1",
            Self::Anthropic => "https://api.anthropic.com",
            Self::Google => "https://generativelanguage.googleapis.com/v1beta/openai",
            Self::DeepSeek => "https://api.deepseek.com",
        }
    }

    fn is_anthropic(self) -> bool {
        matches!(self, Self::Anthropic)
    }
}

// ── OpenAI-compatible request/response types ──

#[derive(Serialize)]
struct OaiRequest {
    model: String,
    messages: Vec<OaiMessage>,
    max_tokens: u32,
    temperature: f32,
}

#[derive(Serialize)]
struct OaiMessage {
    role: &'static str,
    content: String,
}

#[derive(Deserialize)]
struct OaiResponse {
    choices: Vec<OaiChoice>,
    usage: Option<OaiUsage>,
}

#[derive(Deserialize)]
struct OaiChoice {
    message: OaiChoiceMessage,
}

#[derive(Deserialize)]
struct OaiChoiceMessage {
    content: String,
}

#[derive(Deserialize)]
struct OaiUsage {
    prompt_tokens: u32,
    completion_tokens: u32,
}

// ── Anthropic Messages API types ──

#[derive(Serialize)]
struct AnthropicRequest {
    model: String,
    max_tokens: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    system: Option<String>,
    messages: Vec<AnthropicMessage>,
    temperature: f32,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    tools: Vec<AnthropicToolDef>,
}

#[derive(Serialize)]
struct AnthropicMessage {
    role: &'static str,
    content: AnthropicMessageContent,
}

/// Message content: either a plain string or an array of content blocks.
#[derive(Serialize)]
#[serde(untagged)]
enum AnthropicMessageContent {
    Text(String),
    Blocks(Vec<AnthropicBlock>),
}

/// A content block in an Anthropic message (request side).
#[derive(Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum AnthropicBlock {
    Text { text: String },
    ToolUse { id: String, name: String, input: serde_json::Value },
    ToolResult { tool_use_id: String, content: String, is_error: bool },
}

#[derive(Serialize)]
struct AnthropicToolDef {
    name: String,
    description: String,
    input_schema: serde_json::Value,
}

impl From<&ToolDefinition> for AnthropicToolDef {
    fn from(td: &ToolDefinition) -> Self {
        Self {
            name: td.name.clone(),
            description: td.description.clone(),
            input_schema: td.input_schema.clone(),
        }
    }
}

#[derive(Deserialize)]
struct AnthropicResponse {
    content: Vec<AnthropicResponseBlock>,
    usage: Option<AnthropicUsage>,
    stop_reason: Option<String>,
}

/// A content block in an Anthropic response.
#[derive(Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum AnthropicResponseBlock {
    Text { text: String },
    ToolUse { id: String, name: String, input: serde_json::Value },
}

#[derive(Deserialize)]
struct AnthropicUsage {
    input_tokens: u32,
    output_tokens: u32,
}

// ── Provider ──

/// HTTP-based LLM provider. Handles both OpenAI-compatible and Anthropic APIs.
pub struct HttpProvider {
    kind: ProviderKind,
    model: String,
    client: reqwest::Client,
    base_url: String,
    api_key: String,
}

impl HttpProvider {
    /// Build from model name + API key + optional base URL override.
    pub fn new(model: String, api_key: String, base_url: Option<String>) -> Self {
        let kind = ProviderKind::from_model(&model);
        let base = base_url.unwrap_or_else(|| kind.default_base_url().to_owned());
        Self {
            kind,
            model,
            client: reqwest::Client::new(),
            base_url: base.trim_end_matches('/').to_owned(),
            api_key,
        }
    }

    fn endpoint(&self) -> String {
        if self.kind.is_anthropic() {
            format!("{}/v1/messages", self.base_url)
        } else {
            format!("{}/chat/completions", self.base_url)
        }
    }
}

fn role_str(role: &Role) -> &'static str {
    match role {
        Role::System => "system",
        Role::User => "user",
        Role::Assistant => "assistant",
    }
}

/// Parse error response, returning RateLimited for 429.
fn check_error(status: reqwest::StatusCode, body: String) -> LlmError {
    if status.as_u16() == 429 {
        LlmError::RateLimited
    } else {
        LlmError::RequestFailed(format!("{status}: {body}"))
    }
}

impl LlmProvider for HttpProvider {
    fn name(&self) -> &str {
        match self.kind {
            ProviderKind::OpenAi => "openai",
            ProviderKind::Anthropic => "anthropic",
            ProviderKind::Google => "google",
            ProviderKind::DeepSeek => "deepseek",
            ProviderKind::Unknown => "unknown",
        }
    }

    fn complete(
        &self,
        request: CompletionRequest,
    ) -> Pin<Box<dyn Future<Output = Result<CompletionResponse, LlmError>> + Send + '_>> {
        if self.kind.is_anthropic() {
            Box::pin(self.complete_anthropic(request))
        } else {
            Box::pin(self.complete_openai(request))
        }
    }
}

impl HttpProvider {
    /// OpenAI-compatible completion (OpenAI, Gemini, DeepSeek, Unknown).
    /// Tools not supported on this path — ignores request.tools.
    async fn complete_openai(&self, request: CompletionRequest) -> Result<CompletionResponse, LlmError> {
        let body = OaiRequest {
            model: self.model.clone(),
            messages: request.messages.iter().map(|m| OaiMessage {
                role: role_str(&m.role),
                content: m.content.clone(),
            }).collect(),
            max_tokens: request.max_tokens,
            temperature: request.temperature,
        };

        let resp = self.client
            .post(self.endpoint())
            .bearer_auth(&self.api_key)
            .json(&body)
            .send()
            .await
            .map_err(|e| LlmError::RequestFailed(e.to_string()))?;

        if !resp.status().is_success() {
            let status = resp.status();
            let text = resp.text().await.unwrap_or_default();
            return Err(check_error(status, text));
        }

        let api: OaiResponse = resp.json().await
            .map_err(|e| LlmError::RequestFailed(e.to_string()))?;

        let content = api.choices.into_iter().next()
            .map(|c| c.message.content).unwrap_or_default();
        let (input_tokens, output_tokens) = api.usage
            .map(|u| (u.prompt_tokens, u.completion_tokens)).unwrap_or((0, 0));

        let blocks = vec![ContentBlock::Text { text: content.clone() }];
        Ok(CompletionResponse { content, content_blocks: blocks, stop_reason: StopReason::EndTurn, input_tokens, output_tokens })
    }

    /// Anthropic Messages API completion with native tool use support.
    async fn complete_anthropic(&self, request: CompletionRequest) -> Result<CompletionResponse, LlmError> {
        // Extract system message separately (Anthropic puts it at top level).
        let mut system = None;
        let messages: Vec<AnthropicMessage> = request.messages.iter().filter_map(|m| {
            if m.role == Role::System {
                system = Some(m.content.clone());
                None
            } else if m.content_blocks.is_empty() {
                // Plain text message
                Some(AnthropicMessage {
                    role: role_str(&m.role),
                    content: AnthropicMessageContent::Text(m.content.clone()),
                })
            } else {
                // Structured content blocks (tool_use / tool_result)
                let blocks: Vec<AnthropicBlock> = m.content_blocks.iter().map(|b| match b {
                    ContentBlock::Text { text } => AnthropicBlock::Text { text: text.clone() },
                    ContentBlock::ToolUse { id, name, input } => AnthropicBlock::ToolUse {
                        id: id.clone(), name: name.clone(), input: input.clone(),
                    },
                    ContentBlock::ToolResult { tool_use_id, content, is_error } => AnthropicBlock::ToolResult {
                        tool_use_id: tool_use_id.clone(), content: content.clone(), is_error: *is_error,
                    },
                }).collect();
                Some(AnthropicMessage {
                    role: role_str(&m.role),
                    content: AnthropicMessageContent::Blocks(blocks),
                })
            }
        }).collect();

        let tools: Vec<AnthropicToolDef> = request.tools.iter().map(AnthropicToolDef::from).collect();

        let body = AnthropicRequest {
            model: self.model.clone(),
            max_tokens: request.max_tokens,
            system,
            messages,
            temperature: request.temperature,
            tools,
        };

        let resp = self.client
            .post(self.endpoint())
            .header("x-api-key", &self.api_key)
            .header("anthropic-version", "2023-06-01")
            .header("content-type", "application/json")
            .json(&body)
            .send()
            .await
            .map_err(|e| LlmError::RequestFailed(e.to_string()))?;

        if !resp.status().is_success() {
            let status = resp.status();
            let text = resp.text().await.unwrap_or_default();
            return Err(check_error(status, text));
        }

        let api: AnthropicResponse = resp.json().await
            .map_err(|e| LlmError::RequestFailed(e.to_string()))?;

        // Convert response blocks to our ContentBlock type
        let content_blocks: Vec<ContentBlock> = api.content.into_iter().map(|b| match b {
            AnthropicResponseBlock::Text { text } => ContentBlock::Text { text },
            AnthropicResponseBlock::ToolUse { id, name, input } => ContentBlock::ToolUse { id, name, input },
        }).collect();

        // Concatenate text blocks for convenience field
        let content: String = content_blocks.iter().filter_map(|b| match b {
            ContentBlock::Text { text } => Some(text.as_str()),
            _ => None,
        }).collect::<Vec<_>>().join("");

        let stop_reason = match api.stop_reason.as_deref() {
            Some("tool_use") => StopReason::ToolUse,
            Some("max_tokens") => StopReason::MaxTokens,
            _ => StopReason::EndTurn,
        };

        let (input_tokens, output_tokens) = api.usage
            .map(|u| (u.input_tokens, u.output_tokens)).unwrap_or((0, 0));

        Ok(CompletionResponse { content, content_blocks, stop_reason, input_tokens, output_tokens })
    }
}

/// Build an LlmProvider from environment variables.
/// Reads `<model_env_key>`, `IRIS_LLM_API_KEY`, optionally `IRIS_LLM_BASE_URL`.
/// Returns `None` if model or key is not set.
pub fn from_env_with_model_var(model_env_key: &str) -> Option<HttpProvider> {
    let model = std::env::var(model_env_key).ok()?;
    let api_key = std::env::var("IRIS_LLM_API_KEY").ok()?;
    let base_url = std::env::var("IRIS_LLM_BASE_URL").ok();
    Some(HttpProvider::new(model, api_key, base_url))
}

/// Build an LlmProvider from the default environment variables.
/// Reads `IRIS_LLM_MODEL`, `IRIS_LLM_API_KEY`, optionally `IRIS_LLM_BASE_URL`.
/// Returns `None` if model or key is not set.
pub fn from_env() -> Option<HttpProvider> {
    from_env_with_model_var("IRIS_LLM_MODEL")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn infer_openai_models() {
        assert_eq!(ProviderKind::from_model("gpt-4o"), ProviderKind::OpenAi);
        assert_eq!(ProviderKind::from_model("gpt-3.5-turbo"), ProviderKind::OpenAi);
        assert_eq!(ProviderKind::from_model("o1-preview"), ProviderKind::OpenAi);
        assert_eq!(ProviderKind::from_model("o3-mini"), ProviderKind::OpenAi);
        assert_eq!(ProviderKind::from_model("o4-mini"), ProviderKind::OpenAi);
    }

    #[test]
    fn infer_anthropic_models() {
        assert_eq!(ProviderKind::from_model("claude-3-opus"), ProviderKind::Anthropic);
        assert_eq!(ProviderKind::from_model("claude-sonnet-4-6"), ProviderKind::Anthropic);
    }

    #[test]
    fn infer_google_models() {
        assert_eq!(ProviderKind::from_model("gemini-2.0-flash"), ProviderKind::Google);
        assert_eq!(ProviderKind::from_model("gemini-pro"), ProviderKind::Google);
    }

    #[test]
    fn infer_deepseek_models() {
        assert_eq!(ProviderKind::from_model("deepseek-chat"), ProviderKind::DeepSeek);
        assert_eq!(ProviderKind::from_model("deepseek-reasoner"), ProviderKind::DeepSeek);
    }

    #[test]
    fn infer_unknown_falls_back() {
        assert_eq!(ProviderKind::from_model("llama-3"), ProviderKind::Unknown);
        assert_eq!(ProviderKind::from_model("qwen-72b"), ProviderKind::Unknown);
    }

    #[test]
    fn case_insensitive() {
        assert_eq!(ProviderKind::from_model("GPT-4o"), ProviderKind::OpenAi);
        assert_eq!(ProviderKind::from_model("Claude-3-opus"), ProviderKind::Anthropic);
    }

    #[test]
    fn openai_endpoint() {
        let p = HttpProvider::new("gpt-4o".into(), "sk-test".into(), None);
        assert_eq!(p.endpoint(), "https://api.openai.com/v1/chat/completions");
        assert_eq!(p.name(), "openai");
    }

    #[test]
    fn anthropic_endpoint() {
        let p = HttpProvider::new("claude-sonnet-4-6".into(), "sk-ant-test".into(), None);
        assert_eq!(p.endpoint(), "https://api.anthropic.com/v1/messages");
        assert_eq!(p.name(), "anthropic");
    }

    #[test]
    fn deepseek_endpoint() {
        let p = HttpProvider::new("deepseek-chat".into(), "sk-test".into(), None);
        assert_eq!(p.endpoint(), "https://api.deepseek.com/chat/completions");
        assert_eq!(p.name(), "deepseek");
    }

    #[test]
    fn custom_base_url_override() {
        let p = HttpProvider::new(
            "gpt-4o".into(),
            "sk-test".into(),
            Some("https://my-proxy.com/v1".into()),
        );
        assert_eq!(p.endpoint(), "https://my-proxy.com/v1/chat/completions");
    }
}
