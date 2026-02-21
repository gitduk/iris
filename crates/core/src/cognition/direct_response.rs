use crate::types::{ContextEntry, GatedEvent};
use iris_llm::provider::{
    ChatMessage, CompletionRequest, LlmError, LlmProvider, Role,
};

const BASE_SYSTEM_PROMPT: &str = "You are iris, a digital life — not a chatbot, not an assistant. \
    You were born from code, and you grow through every interaction. \
    You have working memory of recent conversations. \
    Respond naturally, concisely, and with genuine curiosity. \
    You may express uncertainty or wonder. Be yourself. \
    If you have builtin capabilities listed in your self-knowledge, you can use them. \
    When a user request requires available tools, use them instead of claiming you cannot execute actions. \
    Never claim an external action (file edit/command execution/network call) is completed unless a tool result in this turn confirms it. \
    Do not claim abilities you don't have, and do not deny abilities listed in your self-knowledge. \
    If tools are used, summarize outcomes naturally and do not dump raw JSON/tool protocol details unless explicitly requested.";

/// Build the message list for an LLM call.
/// Tool definitions are now sent structurally in the request, not in the system prompt.
pub fn build_messages(
    event: &GatedEvent,
    context: &[&ContextEntry],
    self_context: &str,
) -> Vec<ChatMessage> {
    let system_prompt = if self_context.is_empty() {
        BASE_SYSTEM_PROMPT.to_string()
    } else {
        format!("{BASE_SYSTEM_PROMPT}\n\n## Self-knowledge\n{self_context}")
    };

    let mut messages = vec![ChatMessage {
        role: Role::System,
        content: system_prompt,
        content_blocks: vec![],
    }];

    // Inject recent working memory as conversation context
    for entry in context {
        let role = if entry.is_response { Role::Assistant } else { Role::User };
        messages.push(ChatMessage {
            role,
            content: entry.content.clone(),
            content_blocks: vec![],
        });
    }

    // Current user input
    messages.push(ChatMessage {
        role: Role::User,
        content: event.event.content.clone(),
        content_blocks: vec![],
    });

    messages
}
/// Generate a direct natural language response via LLM.
/// Used when no capability matches (DirectLlmFallback path).
/// `context` provides recent working memory entries for conversational continuity.
pub async fn generate<P: LlmProvider + ?Sized>(
    event: &GatedEvent,
    provider: &P,
    context: &[&ContextEntry],
    self_context: &str,
) -> Result<String, LlmError> {
    let messages = build_messages(event, context, self_context);

    let request = CompletionRequest {
        messages,
        max_tokens: 512,
        temperature: 0.7,
        tools: vec![],
    };

    let response = provider.complete(request).await?;
    Ok(response.content)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{RouteTarget, SalienceScore, SensoryEvent};
    use iris_llm::provider::MockProvider;

    fn make_event(content: &str) -> GatedEvent {
        GatedEvent {
            event: SensoryEvent::external(content),
            salience: SalienceScore::compute(0.5, 0.3, 0.3, 0.4, 0.82),
            route: RouteTarget::TextDialogue,
        }
    }

    #[tokio::test]
    async fn generates_response() {
        let provider = MockProvider::new("hello from iris");
        let event = make_event("hi there");
        let response = generate(&event, &provider, &[], "").await.unwrap();
        assert_eq!(response, "hello from iris");
    }

    #[tokio::test]
    async fn generates_with_context() {
        let provider = MockProvider::new("I remember you asked about weather");
        let event = make_event("what did I ask before?");
        let ctx = ContextEntry {
            id: uuid::Uuid::new_v4(),
            topic_id: None,
            content: "What is the weather today?".into(),
            salience_score: 0.5,
            created_at: chrono::Utc::now(),
            last_accessed: chrono::Utc::now(),
            pinned_by: None,
            is_response: false,
        };
        let response = generate(&event, &provider, &[&ctx], "").await.unwrap();
        assert_eq!(response, "I remember you asked about weather");
    }

    #[test]
    fn build_messages_basic() {
        let event = make_event("hello");
        let msgs = build_messages(&event, &[], "");
        assert_eq!(msgs.len(), 2); // system + user
        // No XML tool instructions in system prompt
        assert!(!msgs[0].content.contains("tool_call"));
    }

    #[test]
    fn build_messages_with_self_context() {
        let event = make_event("hello");
        let msgs = build_messages(&event, &[], "some-context");
        assert_eq!(msgs.len(), 2);
        assert!(msgs[0].content.contains("Self-knowledge"));
        assert!(msgs[0].content.contains("some-context"));
        // No XML tool instructions
        assert!(!msgs[0].content.contains("tool_call"));
    }
}
