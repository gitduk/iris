//! Semantic memory: query the `knowledge` table for relevant context.

use sqlx::PgPool;
use uuid::Uuid;

use crate::types::Knowledge;

/// Search knowledge entries by keyword (simple ILIKE match).
pub async fn search(
    pool: &PgPool,
    query: &str,
    limit: i64,
) -> Result<Vec<Knowledge>, sqlx::Error> {
    let pattern = format!("%{query}%");
    let rows = sqlx::query_as::<_, KnowledgeRow>(
        "SELECT id, summary, embedding, source_episode_ids, created_at \
         FROM knowledge WHERE summary ILIKE $1 ORDER BY created_at DESC LIMIT $2",
    )
    .bind(&pattern)
    .bind(limit)
    .fetch_all(pool)
    .await?;

    Ok(rows.into_iter().map(Into::into).collect())
}

/// Fetch the most recent knowledge entries.
pub async fn recent(
    pool: &PgPool,
    limit: i64,
) -> Result<Vec<Knowledge>, sqlx::Error> {
    let rows = sqlx::query_as::<_, KnowledgeRow>(
        "SELECT id, summary, embedding, source_episode_ids, created_at \
         FROM knowledge ORDER BY created_at DESC LIMIT $1",
    )
    .bind(limit)
    .fetch_all(pool)
    .await?;

    Ok(rows.into_iter().map(Into::into).collect())
}

/// Search knowledge by keyword; if no results, fall back to most recent entries.
pub async fn recent_or_search(
    pool: &PgPool,
    query: &str,
    limit: i64,
) -> Result<Vec<Knowledge>, sqlx::Error> {
    let results = search(pool, query, limit).await?;
    if !results.is_empty() {
        return Ok(results);
    }
    recent(pool, limit).await
}

#[derive(sqlx::FromRow)]
struct KnowledgeRow {
    id: Uuid,
    summary: String,
    embedding: Option<Vec<u8>>,
    source_episode_ids: Vec<Uuid>,
    created_at: chrono::DateTime<chrono::Utc>,
}

impl From<KnowledgeRow> for Knowledge {
    fn from(row: KnowledgeRow) -> Self {
        Self {
            id: row.id,
            summary: row.summary,
            embedding: row.embedding,
            source_episode_ids: row.source_episode_ids,
            created_at: row.created_at,
        }
    }
}
