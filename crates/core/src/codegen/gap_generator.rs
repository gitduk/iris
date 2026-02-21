use std::sync::Arc;

use sqlx::PgPool;
use tokio::sync::oneshot;
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

use crate::types::{CodegenHistory, GapDescriptor};
use iris_llm::provider::LlmProvider;

use super::{crate_permit, db, prompt, repair_loop};

/// Submit a gap for async code generation.
/// Returns a oneshot receiver that will contain the result.
pub fn submit_async(
    gap: GapDescriptor,
    pool: PgPool,
    llm: Arc<dyn LlmProvider>,
    cancel: CancellationToken,
) -> oneshot::Receiver<Result<repair_loop::RepairResult, Box<dyn std::error::Error + Send + Sync>>>
{
    let (tx, rx) = oneshot::channel();

    tokio::spawn(async move {
        let result = tokio::select! {
            _ = cancel.cancelled() => {
                Err("codegen cancelled".into())
            }
            result = generate_inner(&gap, &pool, &*llm) => result,
        };
        // oneshot send fails only if receiver was dropped (fire-and-forget) — benign
        let _ = tx.send(result);
    });

    rx
}

/// Synchronous (blocking-async) code generation.
pub async fn generate(
    gap: &GapDescriptor,
    pool: &PgPool,
    llm: &dyn LlmProvider,
) -> Result<repair_loop::RepairResult, Box<dyn std::error::Error + Send + Sync>> {
    generate_inner(gap, pool, llm).await
}
async fn generate_inner(
    gap: &GapDescriptor,
    pool: &PgPool,
    llm: &dyn LlmProvider,
) -> Result<repair_loop::RepairResult, Box<dyn std::error::Error + Send + Sync>> {
    // Check which suggested crates are approved
    let approved: Vec<String> = {
        let mut approved = Vec::new();
        for c in &gap.suggested_crates {
            if crate_permit::is_approved(pool, c).await? {
                approved.push(c.clone());
            }
        }
        approved
    };

    // Fetch past failure summaries for this gap type
    let failures = db::fetch_failure_summaries(pool, gap.gap_type.as_str(), 3).await?;

    // Build prompt
    let codegen_prompt = prompt::build_codegen_prompt(gap, &approved, &failures);

    // Run repair loop
    let result = repair_loop::run(llm, &codegen_prompt).await?;

    // Record history
    let history = CodegenHistory {
        id: Uuid::new_v4(),
        gap_type: gap.gap_type.as_str().to_string(),
        approach_summary: Some(gap.trigger_description.clone()),
        success: result.success,
        error_msg: result.last_error.clone(),
        is_consolidated: false,
        created_at: chrono::Utc::now(),
    };
    if let Err(e) = db::write_history(pool, &history).await {
        tracing::warn!(error = %e, "failed to write codegen history");
    }

    Ok(result)
}
