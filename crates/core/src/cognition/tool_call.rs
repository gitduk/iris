use crate::capability::builtin::BuiltinRegistry;
use crate::types::{CapabilityRequest, CapabilityResponse};
use iris_llm::provider::{
    ChatMessage, CompletionRequest, ContentBlock, LlmError, LlmProvider, Role, StopReason,
    ToolDefinition,
};

/// Maximum number of tool-use iterations before forcing a text-only response.
const MAX_TOOL_ITERATIONS: usize = 5;
/// Default confidence when router output omits this field.
const DEFAULT_ROUTE_CONFIDENCE: f32 = 0.0;

/// Structured tool-routing decision produced by a lightweight gate model.
#[derive(Debug, Clone)]
pub struct ToolRouteDecision {
    pub use_tool: bool,
    pub tool_name: Option<String>,
    pub input: serde_json::Value,
    pub confidence: f32,
    pub is_valid: bool,
}

/// Ask a lightweight model to choose a specific tool and arguments.
///
/// The model must return strict JSON:
/// `{ "use_tool": bool, "tool_name": string|null, "input": object, "confidence": 0..1 }`
pub async fn route_tool_call(
    provider: &dyn LlmProvider,
    user_input: &str,
    tools: &[ToolDefinition],
) -> Result<ToolRouteDecision, LlmError> {
    tracing::debug!(
        provider = provider.name(),
        tools_count = tools.len(),
        user_input_len = user_input.len(),
        user_input_preview = %preview(user_input, 160),
        "tool router request started"
    );

    if tools.is_empty() {
        tracing::debug!("tool router short-circuit: no tools registered");
        return Ok(ToolRouteDecision {
            use_tool: false,
            tool_name: None,
            input: serde_json::json!({}),
            confidence: 1.0,
            is_valid: true,
        });
    }

    let tools_json = serde_json::to_string_pretty(tools).unwrap_or_else(|_| "[]".to_string());

    let request = CompletionRequest {
        messages: vec![
            ChatMessage {
                role: Role::System,
                content: "You are a strict tool router. Output ONLY valid JSON. No markdown, no explanation.".into(),
                content_blocks: vec![],
            },
            ChatMessage {
                role: Role::User,
                content: format!(
                    "Select the best action for the user request.\n\
                     Available tools (JSON):\n{}\n\n\
                     User request:\n{}\n\n\
                     Return exactly one JSON object with keys:\n\
                     - use_tool: boolean\n\
                     - tool_name: string or null\n\
                     - input: object (arguments)\n\
                     - confidence: number in [0,1]\n\
                     If no tool is needed, set use_tool=false, tool_name=null, input={{}}.",
                    tools_json, user_input
                ),
                content_blocks: vec![],
            },
        ],
        max_tokens: 200,
        temperature: 0.0,
        tools: vec![],
    };

    let response = provider.complete(request).await?;
    tracing::debug!(
        raw_response_len = response.content.len(),
        raw_response_preview = %preview(&response.content, 240),
        "tool router raw response received"
    );

    let parsed = match parse_router_json(&response.content) {
        Ok(v) => v,
        Err(e) => {
            tracing::debug!(
                error = %e,
                raw_response_preview = %preview(&response.content, 240),
                "tool router JSON parse failed"
            );
            return Err(LlmError::RequestFailed(e));
        }
    };

    let use_tool = parsed
        .get("use_tool")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    let tool_name = parsed
        .get("tool_name")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());
    let input = parsed
        .get("input")
        .cloned()
        .unwrap_or_else(|| serde_json::json!({}));
    let confidence = parsed
        .get("confidence")
        .and_then(|v| v.as_f64())
        .map(|n| (n as f32).clamp(0.0, 1.0))
        .unwrap_or(DEFAULT_ROUTE_CONFIDENCE);

    let is_valid = if !use_tool {
        true
    } else if let Some(name) = tool_name.as_deref() {
        if let Some(def) = tools.iter().find(|t| t.name == name) {
            validate_against_schema(&input, &def.input_schema)
        } else {
            false
        }
    } else {
        false
    };

    tracing::debug!(
        use_tool,
        tool_name = ?tool_name,
        confidence,
        is_valid,
        input_preview = %preview(&input.to_string(), 240),
        "tool router decision parsed"
    );

    Ok(ToolRouteDecision {
        use_tool,
        tool_name,
        input,
        confidence,
        is_valid,
    })
}

/// Use a lightweight classifier model to decide whether tools are needed.
/// Returns true for tool-required requests, false for text-only replies.
pub async fn should_use_tools(
    provider: &dyn LlmProvider,
    user_input: &str,
    tools: &[ToolDefinition],
) -> Result<bool, LlmError> {
    if tools.is_empty() {
        return Ok(false);
    }

    let tool_list = tools
        .iter()
        .map(|t| format!("- {}: {}", t.name, t.description))
        .collect::<Vec<_>>()
        .join("\n");

    let request = CompletionRequest {
        messages: vec![
            ChatMessage {
                role: Role::System,
                content: "You are a strict classifier. Decide whether the user request needs calling a tool. Reply with ONLY YES or NO.".into(),
                content_blocks: vec![],
            },
            ChatMessage {
                role: Role::User,
                content: format!(
                    "Available tools:\n{}\n\nUser request:\n{}\n\nNeed tool call?",
                    tool_list, user_input
                ),
                content_blocks: vec![],
            },
        ],
        max_tokens: 8,
        temperature: 0.0,
        tools: vec![],
    };

    let response = provider.complete(request).await?;
    let answer = response.content.trim().to_lowercase();

    if answer.starts_with("yes") || answer == "y" || answer.contains("是") {
        Ok(true)
    } else if answer.starts_with("no") || answer == "n" || answer.contains("否") {
        Ok(false)
    } else {
        // Unclear classifier answer: default to text-only to avoid unnecessary tool churn.
        Ok(false)
    }
}

fn parse_router_json(raw: &str) -> Result<serde_json::Value, String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err("router output is empty".into());
    }

    let candidate = if trimmed.starts_with("```") {
        let lines: Vec<&str> = trimmed.lines().collect();
        let start = if lines
            .first()
            .is_some_and(|l| l.trim_start().starts_with("```"))
        {
            1
        } else {
            0
        };
        let end = lines
            .iter()
            .rposition(|l| l.trim_start().starts_with("```"))
            .unwrap_or(lines.len());
        let end = end.max(start);
        lines[start..end].join("\n")
    } else if let (Some(start), Some(end)) = (trimmed.find('{'), trimmed.rfind('}')) {
        trimmed[start..=end].to_string()
    } else {
        trimmed.to_string()
    };

    serde_json::from_str::<serde_json::Value>(&candidate)
        .map_err(|e| format!("invalid router JSON: {e}; raw: {trimmed}"))
}

fn validate_against_schema(input: &serde_json::Value, schema: &serde_json::Value) -> bool {
    if !input.is_object() {
        tracing::debug!(
            input_preview = %preview(&input.to_string(), 160),
            "tool router schema validation failed: input is not an object"
        );
        return false;
    }

    // Required keys
    if let Some(required) = schema.get("required").and_then(|v| v.as_array()) {
        for key in required.iter().filter_map(|v| v.as_str()) {
            if input.get(key).is_none() {
                tracing::debug!(
                    missing_key = key,
                    input_preview = %preview(&input.to_string(), 160),
                    "tool router schema validation failed: required key missing"
                );
                return false;
            }
        }
    }

    // Shallow property type checks.
    if let Some(props) = schema.get("properties").and_then(|v| v.as_object()) {
        for (key, prop_schema) in props {
            if let Some(value) = input.get(key)
                && let Some(type_name) = prop_schema.get("type").and_then(|v| v.as_str())
                && !matches_json_type(value, type_name)
            {
                tracing::debug!(
                    key = key,
                    expected_type = type_name,
                    actual_value_preview = %preview(&value.to_string(), 120),
                    "tool router schema validation failed: type mismatch"
                );
                return false;
            }
        }
    }

    true
}

fn preview(s: &str, max: usize) -> String {
    let mut out: String = s.chars().take(max).collect();
    if s.chars().count() > max {
        out.push_str("...");
    }
    out
}

fn matches_json_type(value: &serde_json::Value, type_name: &str) -> bool {
    match type_name {
        "string" => value.is_string(),
        "number" => value.is_number(),
        "integer" => value.as_i64().is_some() || value.as_u64().is_some(),
        "boolean" => value.is_boolean(),
        "object" => value.is_object(),
        "array" => value.is_array(),
        "null" => value.is_null(),
        _ => true,
    }
}

/// Execute a single builtin tool by name with structured JSON input.
async fn execute_tool(
    registry: &BuiltinRegistry,
    tool_name: &str,
    input: &serde_json::Value,
) -> Result<String, String> {
    let cap = registry.get_by_name(tool_name).ok_or_else(|| {
        let available = registry.list_names().join(", ");
        format!("Unknown tool '{tool_name}'. Available: {available}")
    })?;

    let request = CapabilityRequest {
        id: uuid::Uuid::new_v4(),
        method: input.to_string(),
        params: input.clone(),
        version: 1,
    };

    let resp: CapabilityResponse = cap.execute(request).await;
    if let Some(err) = resp.error {
        Err(err)
    } else if let Some(result) = resp.result {
        Ok(result.to_string())
    } else {
        Ok("ok".to_string())
    }
}

/// Execute one explicitly selected tool with validated JSON input.
pub async fn execute_named_tool(
    registry: &BuiltinRegistry,
    tool_name: &str,
    input: &serde_json::Value,
) -> Result<String, String> {
    execute_tool(registry, tool_name, input).await
}

/// Run the agentic tool-use loop using Claude's native tool use protocol.
///
/// Each iteration: call LLM with tool definitions → check stop_reason →
/// if ToolUse: execute tools, send tool_result blocks → repeat.
/// Stops on EndTurn/MaxTokens or after MAX_TOOL_ITERATIONS rounds.
pub async fn run_agentic_loop(
    provider: &dyn LlmProvider,
    initial_messages: Vec<ChatMessage>,
    tools: Vec<ToolDefinition>,
    registry: &BuiltinRegistry,
) -> Result<String, LlmError> {
    let mut messages = initial_messages;
    let mut final_text = String::new();

    for iteration in 0..MAX_TOOL_ITERATIONS {
        let request = CompletionRequest {
            messages: messages.clone(),
            max_tokens: 4096,
            temperature: 0.7,
            tools: tools.clone(),
        };

        let response = provider.complete(request).await?;

        match response.stop_reason {
            StopReason::EndTurn | StopReason::MaxTokens => {
                final_text = response.content;
                break;
            }
            StopReason::ToolUse => {
                // Append assistant message with all content blocks
                messages.push(ChatMessage::from_content_blocks(
                    Role::Assistant,
                    response.content_blocks.clone(),
                ));

                // Collect tool_use blocks and execute them
                let tool_uses: Vec<_> = response
                    .content_blocks
                    .iter()
                    .filter_map(|b| {
                        if let ContentBlock::ToolUse { id, name, input } = b {
                            Some((id.clone(), name.clone(), input.clone()))
                        } else {
                            None
                        }
                    })
                    .collect();

                let mut result_blocks = Vec::new();
                for (id, name, input) in &tool_uses {
                    tracing::info!(
                        tool = %name,
                        iteration = iteration,
                        "agentic loop: executing tool"
                    );

                    let (content, is_error) = match execute_tool(registry, name, input).await {
                        Ok(result) => (result, false),
                        Err(err) => (err, true),
                    };

                    result_blocks.push(ContentBlock::ToolResult {
                        tool_use_id: id.clone(),
                        content,
                        is_error,
                    });
                }

                // Append user message with tool results
                messages.push(ChatMessage::tool_results(result_blocks));

                // If last iteration, do one final call without tools
                if iteration == MAX_TOOL_ITERATIONS - 1 {
                    tracing::warn!("agentic loop: max iterations reached, forcing final response");
                    let request = CompletionRequest {
                        messages: messages.clone(),
                        max_tokens: 4096,
                        temperature: 0.7,
                        tools: vec![],
                    };
                    let response = provider.complete(request).await?;
                    final_text = response.content;
                }
            }
        }
    }

    Ok(final_text)
}

#[cfg(test)]
mod tests {
    use super::*;
    use iris_llm::provider::MockProvider;

    #[tokio::test]
    async fn router_returns_valid_tool_decision() {
        let provider = MockProvider::new(
            r#"{"use_tool":true,"tool_name":"run_bash","input":{"command":"echo hi"},"confidence":0.91}"#,
        );
        let tools = vec![ToolDefinition {
            name: "run_bash".into(),
            description: "Execute shell command".into(),
            input_schema: serde_json::json!({
                "type":"object",
                "properties":{"command":{"type":"string"}},
                "required":["command"]
            }),
        }];

        let decision = route_tool_call(&provider, "run echo hi", &tools)
            .await
            .unwrap();
        assert!(decision.use_tool);
        assert_eq!(decision.tool_name.as_deref(), Some("run_bash"));
        assert!(decision.is_valid);
        assert!(decision.confidence > 0.9);
    }

    #[tokio::test]
    async fn router_invalid_when_required_arg_missing() {
        let provider = MockProvider::new(
            r#"{"use_tool":true,"tool_name":"run_bash","input":{},"confidence":0.95}"#,
        );
        let tools = vec![ToolDefinition {
            name: "run_bash".into(),
            description: "Execute shell command".into(),
            input_schema: serde_json::json!({
                "type":"object",
                "properties":{"command":{"type":"string"}},
                "required":["command"]
            }),
        }];

        let decision = route_tool_call(&provider, "run echo hi", &tools)
            .await
            .unwrap();
        assert!(decision.use_tool);
        assert!(!decision.is_valid);
    }

    #[tokio::test]
    async fn router_invalid_when_tool_unknown() {
        let provider = MockProvider::new(
            r#"{"use_tool":true,"tool_name":"unknown_tool","input":{},"confidence":0.9}"#,
        );
        let tools = vec![ToolDefinition {
            name: "run_bash".into(),
            description: "Execute shell command".into(),
            input_schema: serde_json::json!({
                "type":"object",
                "properties":{"command":{"type":"string"}},
                "required":["command"]
            }),
        }];

        let decision = route_tool_call(&provider, "run echo hi", &tools)
            .await
            .unwrap();
        assert!(decision.use_tool);
        assert!(!decision.is_valid);
    }

    #[tokio::test]
    async fn classifier_yes_means_use_tools() {
        let provider = MockProvider::new("YES");
        let tools = vec![ToolDefinition {
            name: "run_bash".into(),
            description: "Execute shell command".into(),
            input_schema: serde_json::json!({"type":"object"}),
        }];

        let use_tools = should_use_tools(&provider, "run ls", &tools).await.unwrap();
        assert!(use_tools);
    }

    #[tokio::test]
    async fn classifier_no_means_no_tools() {
        let provider = MockProvider::new("NO");
        let tools = vec![ToolDefinition {
            name: "run_bash".into(),
            description: "Execute shell command".into(),
            input_schema: serde_json::json!({"type":"object"}),
        }];

        let use_tools = should_use_tools(&provider, "hello", &tools).await.unwrap();
        assert!(!use_tools);
    }

    #[tokio::test]
    async fn classifier_unclear_defaults_to_no_tools() {
        let provider = MockProvider::new("maybe");
        let tools = vec![ToolDefinition {
            name: "run_bash".into(),
            description: "Execute shell command".into(),
            input_schema: serde_json::json!({"type":"object"}),
        }];

        let use_tools = should_use_tools(&provider, "hello", &tools).await.unwrap();
        assert!(!use_tools);
    }

    #[tokio::test]
    async fn agentic_loop_no_tool_call() {
        // LLM returns plain text with EndTurn → loop exits immediately
        let provider = MockProvider::new("just a normal answer");
        let registry = BuiltinRegistry::new();

        let messages = vec![ChatMessage {
            role: Role::User,
            content: "hello".into(),
            content_blocks: vec![],
        }];

        let result = run_agentic_loop(&provider, messages, vec![], &registry)
            .await
            .unwrap();
        assert_eq!(result, "just a normal answer");
    }

    #[tokio::test]
    async fn agentic_loop_with_tool_use() {
        // First call returns ToolUse, second call returns EndTurn
        use iris_llm::provider::{CompletionResponse, LlmError};
        use std::sync::atomic::{AtomicUsize, Ordering};

        struct TwoStepProvider {
            call_count: AtomicUsize,
        }

        impl LlmProvider for TwoStepProvider {
            fn name(&self) -> &str {
                "two-step"
            }

            fn complete(
                &self,
                _request: CompletionRequest,
            ) -> std::pin::Pin<
                Box<
                    dyn std::future::Future<Output = Result<CompletionResponse, LlmError>>
                        + Send
                        + '_,
                >,
            > {
                let n = self.call_count.fetch_add(1, Ordering::SeqCst);
                Box::pin(async move {
                    if n == 0 {
                        // First call: tool use
                        let blocks = vec![
                            ContentBlock::Text {
                                text: "Let me check.".into(),
                            },
                            ContentBlock::ToolUse {
                                id: "tu_1".into(),
                                name: "run_bash".into(),
                                input: serde_json::json!({"command": "echo hello"}),
                            },
                        ];
                        Ok(CompletionResponse {
                            content: "Let me check.".into(),
                            content_blocks: blocks,
                            stop_reason: StopReason::ToolUse,
                            input_tokens: 10,
                            output_tokens: 20,
                        })
                    } else {
                        // Second call: final answer
                        Ok(CompletionResponse {
                            content: "The command output: hello".into(),
                            content_blocks: vec![ContentBlock::Text {
                                text: "The command output: hello".into(),
                            }],
                            stop_reason: StopReason::EndTurn,
                            input_tokens: 10,
                            output_tokens: 20,
                        })
                    }
                })
            }
        }

        let provider = TwoStepProvider {
            call_count: AtomicUsize::new(0),
        };
        let registry = BuiltinRegistry::new();

        let tools = registry.tool_definitions();
        let messages = vec![ChatMessage {
            role: Role::User,
            content: "run echo hello".into(),
            content_blocks: vec![],
        }];

        let result = run_agentic_loop(&provider, messages, tools, &registry)
            .await
            .unwrap();
        assert_eq!(result, "The command output: hello");
        assert_eq!(provider.call_count.load(Ordering::SeqCst), 2);
    }
}
