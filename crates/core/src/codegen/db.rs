use sqlx::PgPool;

use crate::types::CodegenHistory;

/// Write a codegen history record.
pub async fn write_history(pool: &PgPool, history: &CodegenHistory) -> Result<(), sqlx::Error> {
    sqlx::query(
        "INSERT INTO codegen_history (id, gap_type, approach_summary, success, error_msg, consolidated_flag, created_at)
         VALUES ($1, $2, $3, $4, $5, $6, $7)",
    )
    .bind(history.id)
    .bind(&history.gap_type)
    .bind(&history.approach_summary)
    .bind(history.success)
    .bind(&history.error_msg)
    .bind(history.is_consolidated)
    .bind(history.created_at)
    .execute(pool)
    .await?;
    Ok(())
}

/// Fetch recent failure summaries for a given gap type.
pub async fn fetch_failure_summaries(
    pool: &PgPool,
    gap_type: &str,
    limit: i64,
) -> Result<Vec<String>, sqlx::Error> {
    let rows: Vec<(Option<String>,)> = sqlx::query_as(
        "SELECT error_msg FROM codegen_history
         WHERE gap_type = $1 AND NOT success AND error_msg IS NOT NULL
         ORDER BY created_at DESC LIMIT $2",
    )
    .bind(gap_type)
    .bind(limit)
    .fetch_all(pool)
    .await?;
    Ok(rows.into_iter().filter_map(|r| r.0).collect())
}
