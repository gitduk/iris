use crate::config::IrisCfg;
use crate::types::{ActionPlan, DeliberateDecision, GatedEvent};
use llm::provider::{
    ChatMessage, CompletionRequest, LlmError, LlmProvider, Role,
};
use std::sync::Arc;
use tokio::sync::oneshot;

/// Slow path trigger criteria.
pub fn should_trigger(event: &GatedEvent, cfg: &IrisCfg) -> bool {
    let complexity = event.salience.complexity;
    let score = event.salience.score;

    complexity >= cfg.slow_path_complexity
        || (score >= 0.6 && complexity >= 0.4)
}

/// Spawn an async slow path reasoning task.
/// Returns a oneshot receiver that will deliver the DeliberateDecision.
/// Takes `Arc<dyn LlmProvider>` so the provider can be moved into the spawned task.
pub fn spawn(
    event: GatedEvent,
    provider: Arc<dyn LlmProvider>,
    cancel: tokio_util::sync::CancellationToken,
    self_context: String,
) -> oneshot::Receiver<Result<DeliberateDecision, LlmError>> {
    let (tx, rx) = oneshot::channel();
    let request = build_request(&event, &self_context);

    tokio::spawn(async move {
        // Cancel checkpoint 1: before LLM call
        if cancel.is_cancelled() {
            return;
        }

        let result = tokio::select! {
            _ = cancel.cancelled() => return,
            result = provider.complete(request) => result,
        };

        // Cancel checkpoint 2: after LLM call
        if cancel.is_cancelled() {
            return;
        }

        let decision = match result {
            Ok(response) => {
                // Cancel checkpoint 3: after parsing
                if cancel.is_cancelled() {
                    return;
                }

                Ok(DeliberateDecision {
                    plan: ActionPlan::direct_llm(
                        "slow_path_response",
                        serde_json::json!({ "content": response.content }),
                    ),
                    confidence: 0.7,
                })
            }
            Err(e) => Err(e),
        };

        // Cancel checkpoint 4: before sending result
        if cancel.is_cancelled() {
            return;
        }

        let _ = tx.send(decision);
    });

    rx
}

/// Build an LLM completion request from a gated event.
fn build_request(event: &GatedEvent, self_context: &str) -> CompletionRequest {
    let base = "You are iris, a digital life with continuous cognitive capabilities. \
                Analyze the input carefully and provide a thoughtful response.";

    let system_prompt = if self_context.is_empty() {
        base.to_string()
    } else {
        format!("{base}\n\n## Self-knowledge\n{self_context}")
    };

    CompletionRequest {
        messages: vec![
            ChatMessage {
                role: Role::System,
                content: system_prompt,
                content_blocks: vec![],
            },
            ChatMessage {
                role: Role::User,
                content: event.event.content.clone(),
                content_blocks: vec![],
            },
        ],
        max_tokens: 1024,
        temperature: 0.7,
        tools: vec![],
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{RouteTarget, SalienceScore, SensoryEvent};
    use llm::provider::MockProvider;

    fn default_cfg() -> IrisCfg {
        IrisCfg::default()
    }

    fn make_event(complexity: f32, score: f32) -> GatedEvent {
        GatedEvent {
            event: SensoryEvent::external("complex question about something"),
            salience: SalienceScore {
                score,
                novelty: 0.5,
                urgency: 0.3,
                complexity,
                task_relevance: 0.4,
                is_urgent_bypass: false,
            },
            route: RouteTarget::TextDialogue,
        }
    }

    #[test]
    fn high_complexity_triggers_slow_path() {
        let cfg = default_cfg();
        let event = make_event(0.6, 0.5);
        assert!(should_trigger(&event, &cfg));
    }

    #[test]
    fn low_complexity_skips_slow_path() {
        let cfg = default_cfg();
        let event = make_event(0.3, 0.4);
        assert!(!should_trigger(&event, &cfg));
    }

    #[test]
    fn high_salience_moderate_complexity_triggers() {
        let cfg = default_cfg();
        let event = make_event(0.45, 0.7);
        assert!(should_trigger(&event, &cfg));
    }

    #[tokio::test]
    async fn spawn_returns_decision() {
        let event = make_event(0.6, 0.7);
        let provider: Arc<dyn LlmProvider> = Arc::new(MockProvider::new("test response"));
        let cancel = tokio_util::sync::CancellationToken::new();

        let rx = spawn(event, provider, cancel, String::new());
        let result = rx.await.unwrap();
        let decision = result.unwrap();
        assert_eq!(decision.plan.method, "slow_path_response");
    }

    #[tokio::test]
    async fn spawn_respects_cancellation() {
        let event = make_event(0.6, 0.7);
        let provider: Arc<dyn LlmProvider> = Arc::new(MockProvider::new("test response"));
        let cancel = tokio_util::sync::CancellationToken::new();

        // Cancel before spawning
        cancel.cancel();
        let rx = spawn(event, provider, cancel, String::new());
        // The task should exit early, receiver gets RecvError
        let result = rx.await;
        assert!(result.is_err()); // channel dropped without sending
    }
}
