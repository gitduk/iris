use std::sync::Arc;
use sqlx::PgPool;
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

use crate::memory::episodic;
use crate::types::Knowledge;
use llm::provider::{ChatMessage, CompletionRequest, LlmProvider, Role};

/// Maximum consecutive failures before skipping a consolidation cycle.
const MAX_CONSECUTIVE_FAILURES: u32 = 3;

/// Spawn the consolidation background task.
/// Runs every `interval_secs`, scans unconsolidated episodes, LLM-summarizes them
/// into knowledge entries.
pub fn spawn(
    pool: PgPool,
    llm: Arc<dyn LlmProvider>,
    interval_secs: u64,
    cancel: CancellationToken,
) {
    tokio::spawn(async move {
        let interval = std::time::Duration::from_secs(interval_secs);
        let mut consecutive_failures: u32 = 0;

        loop {
            tokio::select! {
                _ = cancel.cancelled() => {
                    tracing::info!("consolidation task shutting down");
                    return;
                }
                _ = tokio::time::sleep(interval) => {}
            }

            if cancel.is_cancelled() {
                return;
            }

            match run_cycle(&pool, &*llm).await {
                Ok(count) => {
                    consecutive_failures = 0;
                    if count > 0 {
                        tracing::info!(consolidated = count, "consolidation cycle complete");
                    }
                }
                Err(e) => {
                    consecutive_failures += 1;
                    tracing::warn!(
                        error = %e,
                        consecutive_failures,
                        "consolidation cycle failed"
                    );
                    if consecutive_failures >= MAX_CONSECUTIVE_FAILURES {
                        tracing::error!(
                            "consolidation: {} consecutive failures, skipping cycle",
                            MAX_CONSECUTIVE_FAILURES
                        );
                        consecutive_failures = 0;
                    }
                }
            }
        }
    });
}

/// Run one consolidation cycle. Returns number of episodes consolidated.
async fn run_cycle(
    pool: &PgPool,
    llm: &dyn LlmProvider,
) -> Result<usize, Box<dyn std::error::Error + Send + Sync>> {
    let episodes = episodic::fetch_unconsolidated(pool, 10).await?;
    if episodes.is_empty() {
        return Ok(0);
    }

    // Build a combined text for LLM summarization
    let combined: String = episodes
        .iter()
        .map(|e| format!("- {}", e.content))
        .collect::<Vec<_>>()
        .join("\n");

    let request = CompletionRequest {
        messages: vec![
            ChatMessage {
                role: Role::System,
                content: "You are a memory consolidation system. Summarize the following \
                          episodic memories into a concise knowledge entry. Extract key facts, \
                          patterns, and insights. Be brief and factual."
                    .into(),
                content_blocks: vec![],
            },
            ChatMessage {
                role: Role::User,
                content: combined,
                content_blocks: vec![],
            },
        ],
        max_tokens: 512,
        temperature: 0.3,
        tools: vec![],
    };

    let response = llm.complete(request).await?;

    let episode_ids: Vec<Uuid> = episodes.iter().map(|e| e.id).collect();

    let emb = crate::memory::embedding::generate(&response.content);
    let knowledge = Knowledge {
        id: Uuid::new_v4(),
        summary: response.content,
        embedding: Some(emb),
        source_episode_ids: episode_ids.clone(),
        created_at: chrono::Utc::now(),
    };

    episodic::write_knowledge(pool, &knowledge).await?;
    episodic::mark_consolidated(pool, &episode_ids).await?;

    Ok(episodes.len())
}

