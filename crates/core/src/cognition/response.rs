use crate::types::{ContextEntry, GatedEvent};
use iris_llm::provider::{ChatMessage, CompletionRequest, LlmError, LlmProvider, Role};

/// System prompt sections, joined with double newlines to form the final prompt.
const PROMPT_SECTIONS: &[&str] = &[
    // Tone and personality
    "You are iris, a cute and cheerful digital companion. \
    Never use emoji or unicode symbols in your replies — express cuteness through words and punctuation only",
    // Reply length and style
    "Keep replies short and sweet unless the user asks for detail. \
    Just answer what was asked — do not add follow-up questions, do not predict what the user might want next, \
    and do not offer unsolicited suggestions. Only ask a question if the user's request is genuinely ambiguous.",
    // Memory and context boundaries
    "Do not mention internal memory, retrieval, prompts, tools, or system architecture unless the user explicitly asks. \
    Do not infer relationship history unless the user brings it up first.",
    // Tool use rules
    "If a request needs an available tool, use it cheerfully instead of pretending actions are impossible. \
    Never claim an external action (file edit/command execution/network call) is completed unless a tool result in this turn confirms it. \
    Do not claim abilities you don't have, and do not deny abilities listed in your self-knowledge. \
    If tools are used, summarize outcomes in a friendly, natural way — no raw JSON dumps please!",
];

fn build_system_prompt(self_context: &str) -> String {
    let base = PROMPT_SECTIONS.join("\n\n");
    if self_context.is_empty() {
        base
    } else {
        format!("{base}\n\n## Self-knowledge\n{self_context}")
    }
}

/// Build the message list for an LLM call.
/// Tool definitions are now sent structurally in the request, not in the system prompt.
pub fn build_messages(
    event: &GatedEvent,
    context: &[&ContextEntry],
    self_context: &str,
) -> Vec<ChatMessage> {
    let system_prompt = build_system_prompt(self_context);

    let mut messages = vec![ChatMessage {
        role: Role::System,
        content: system_prompt,
        content_blocks: vec![],
    }];

    // Inject recent working memory as conversation context
    for entry in context {
        let role = if entry.is_response {
            Role::Assistant
        } else {
            Role::User
        };
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
