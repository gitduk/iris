use serde::{Deserialize, Serialize};
use std::future::Future;
use std::pin::Pin;

/// A single message in a conversation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatMessage {
    pub role: Role,
    /// Plain text content (convenience — concatenation of Text blocks).
    pub content: String,
    /// Structured content blocks (native tool use protocol).
    /// Empty means the message is plain text only.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub content_blocks: Vec<ContentBlock>,
}

impl ChatMessage {
    /// Build a message from structured content blocks.
    pub fn from_content_blocks(role: Role, blocks: Vec<ContentBlock>) -> Self {
        let text: String = blocks.iter().filter_map(|b| match b {
            ContentBlock::Text { text } => Some(text.as_str()),
            _ => None,
        }).collect::<Vec<_>>().join("");
        Self { role, content: text, content_blocks: blocks }
    }

    /// Build a User message carrying tool results.
    pub fn tool_results(results: Vec<ContentBlock>) -> Self {
        Self { role: Role::User, content: String::new(), content_blocks: results }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Role {
    System,
    User,
    Assistant,
}

// ── Tool use types ──

/// Tool definition sent in requests (name + description + JSON Schema).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolDefinition {
    pub name: String,
    pub description: String,
    pub input_schema: serde_json::Value,
}

/// A content block in a message — text, tool use, or tool result.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ContentBlock {
    Text { text: String },
    ToolUse { id: String, name: String, input: serde_json::Value },
    ToolResult { tool_use_id: String, content: String, is_error: bool },
}

/// Why the model stopped generating.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum StopReason {
    #[default]
    EndTurn,
    ToolUse,
    MaxTokens,
}

/// LLM completion request.
#[derive(Debug, Clone)]
pub struct CompletionRequest {
    pub messages: Vec<ChatMessage>,
    pub max_tokens: u32,
    pub temperature: f32,
    /// Tool definitions for native tool use (empty = no tools).
    pub tools: Vec<ToolDefinition>,
}

/// LLM completion response.
#[derive(Debug, Clone)]
pub struct CompletionResponse {
    /// Convenience: concatenation of all Text blocks.
    pub content: String,
    /// Structured content blocks from the model.
    pub content_blocks: Vec<ContentBlock>,
    /// Why the model stopped.
    pub stop_reason: StopReason,
    pub input_tokens: u32,
    pub output_tokens: u32,
}

/// Error type for LLM operations.
#[derive(Debug, thiserror::Error)]
pub enum LlmError {
    #[error("provider unavailable: {0}")]
    Unavailable(String),
    #[error("rate limited")]
    RateLimited,
    #[error("request failed: {0}")]
    RequestFailed(String),
    #[error("all providers exhausted")]
    AllProvidersExhausted,
}

/// Trait for LLM providers (OpenAI, Claude, Gemini, etc.)
pub trait LlmProvider: Send + Sync {
    fn name(&self) -> &str;

    fn complete(
        &self,
        request: CompletionRequest,
    ) -> Pin<Box<dyn Future<Output = Result<CompletionResponse, LlmError>> + Send + '_>>;
}

/// Mock provider for testing — returns a fixed response.
#[derive(Debug, Clone)]
pub struct MockProvider {
    pub response: String,
    pub response_blocks: Vec<ContentBlock>,
    pub stop_reason: StopReason,
}

impl MockProvider {
    pub fn new(response: impl Into<String>) -> Self {
        let text = response.into();
        Self {
            response: text.clone(),
            response_blocks: vec![ContentBlock::Text { text }],
            stop_reason: StopReason::EndTurn,
        }
    }

    /// Create a mock that returns specific content blocks and stop reason.
    pub fn with_blocks(blocks: Vec<ContentBlock>, stop_reason: StopReason) -> Self {
        let text: String = blocks.iter().filter_map(|b| match b {
            ContentBlock::Text { text } => Some(text.as_str()),
            _ => None,
        }).collect::<Vec<_>>().join("");
        Self { response: text, response_blocks: blocks, stop_reason }
    }
}

impl LlmProvider for MockProvider {
    fn name(&self) -> &str {
        "mock"
    }

    fn complete(
        &self,
        _request: CompletionRequest,
    ) -> Pin<Box<dyn Future<Output = Result<CompletionResponse, LlmError>> + Send + '_>> {
        let content = self.response.clone();
        let blocks = self.response_blocks.clone();
        let stop = self.stop_reason;
        Box::pin(async move {
            Ok(CompletionResponse {
                content,
                content_blocks: blocks,
                stop_reason: stop,
                input_tokens: 10,
                output_tokens: 20,
            })
        })
    }
}

/// LLM router — routes requests to available providers with fallback.
/// Tracks per-provider failure counts; 3 consecutive failures → unavailable.
pub struct LlmRouter {
    providers: Vec<Box<dyn LlmProvider>>,
    fail_counts: Vec<u32>,
}

impl LlmRouter {
    pub fn new(providers: Vec<Box<dyn LlmProvider>>) -> Self {
        let len = providers.len();
        Self {
            providers,
            fail_counts: vec![0; len],
        }
    }

    /// True if at least one provider is available.
    pub fn is_available(&self) -> bool {
        self.fail_counts.iter().any(|&c| c < 3)
    }

    /// Send a completion request, trying providers in priority order.
    pub async fn complete(&mut self, request: CompletionRequest) -> Result<CompletionResponse, LlmError> {
        for (i, provider) in self.providers.iter().enumerate() {
            if self.fail_counts[i] >= 3 {
                continue;
            }

            match provider.complete(request.clone()).await {
                Ok(response) => {
                    self.fail_counts[i] = 0;
                    return Ok(response);
                }
                Err(e) => {
                    self.fail_counts[i] += 1;
                    tracing::warn!(
                        provider = provider.name(),
                        fail_count = self.fail_counts[i],
                        error = %e,
                        "LLM provider failed"
                    );
                }
            }
        }

        Err(LlmError::AllProvidersExhausted)
    }

    /// Reset failure count for a provider (called by periodic health probe).
    pub fn reset_provider(&mut self, index: usize) {
        if let Some(count) = self.fail_counts.get_mut(index) {
            *count = 0;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn mock_provider_returns_response() {
        let mock = MockProvider::new("hello iris");
        let req = CompletionRequest {
            messages: vec![ChatMessage {
                role: Role::User,
                content: "hi".into(),
                content_blocks: vec![],
            }],
            max_tokens: 100,
            temperature: 0.7,
            tools: vec![],
        };
        let resp = mock.complete(req).await.unwrap();
        assert_eq!(resp.content, "hello iris");
        assert_eq!(resp.stop_reason, StopReason::EndTurn);
    }

    #[tokio::test]
    async fn router_falls_through_on_failure() {
        let providers: Vec<Box<dyn LlmProvider>> = vec![
            Box::new(MockProvider::new("from first")),
            Box::new(MockProvider::new("from second")),
        ];
        let mut router = LlmRouter::new(providers);
        assert!(router.is_available());

        let req = CompletionRequest {
            messages: vec![ChatMessage {
                role: Role::User,
                content: "test".into(),
                content_blocks: vec![],
            }],
            max_tokens: 50,
            temperature: 0.5,
            tools: vec![],
        };
        let resp = router.complete(req).await.unwrap();
        assert_eq!(resp.content, "from first");
    }
}
