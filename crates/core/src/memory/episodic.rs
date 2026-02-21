use crate::types::Episode;
use sqlx::PgPool;
use uuid::Uuid;

/// Write an episode to the `episodes` table.
pub async fn write(pool: &PgPool, episode: &Episode) -> Result<(), sqlx::Error> {
    sqlx::query(
        "INSERT INTO episodes (id, topic_id, content, embedding, salience, is_consolidated, created_at) \
         VALUES ($1, $2, $3, $4, $5, $6, $7)",
    )
    .bind(episode.id)
    .bind(episode.topic_id)
    .bind(&episode.content)
    .bind(&episode.embedding)
    .bind(episode.salience)
    .bind(episode.is_consolidated)
    .bind(episode.created_at)
    .execute(pool)
    .await?;
    Ok(())
}

/// Fetch unconsolidated episodes, ordered by creation time.
pub async fn fetch_unconsolidated(
    pool: &PgPool,
    limit: i64,
) -> Result<Vec<Episode>, sqlx::Error> {
    let rows = sqlx::query_as::<_, EpisodeRow>(
        "SELECT id, topic_id, content, embedding, salience, is_consolidated, created_at \
         FROM episodes WHERE NOT is_consolidated ORDER BY created_at ASC LIMIT $1",
    )
    .bind(limit)
    .fetch_all(pool)
    .await?;

    Ok(rows.into_iter().map(Into::into).collect())
}

/// Fetch episodes with salience above threshold for replay.
pub async fn fetch_for_replay(
    pool: &PgPool,
    min_salience: f32,
    limit: i64,
) -> Result<Vec<Episode>, sqlx::Error> {
    let rows = sqlx::query_as::<_, EpisodeRow>(
        "SELECT id, topic_id, content, embedding, salience, is_consolidated, created_at \
         FROM episodes WHERE salience >= $1 ORDER BY salience DESC LIMIT $2",
    )
    .bind(min_salience)
    .bind(limit)
    .fetch_all(pool)
    .await?;

    Ok(rows.into_iter().map(Into::into).collect())
}

/// Mark episodes as consolidated.
pub async fn mark_consolidated(
    pool: &PgPool,
    ids: &[Uuid],
) -> Result<(), sqlx::Error> {
    sqlx::query("UPDATE episodes SET is_consolidated = TRUE WHERE id = ANY($1)")
        .bind(ids)
        .execute(pool)
        .await?;
    Ok(())
}

/// Write a knowledge entry to the `knowledge` table.
pub async fn write_knowledge(
    pool: &PgPool,
    knowledge: &crate::types::Knowledge,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "INSERT INTO knowledge (id, summary, embedding, source_episode_ids, created_at) \
         VALUES ($1, $2, $3, $4, $5)",
    )
    .bind(knowledge.id)
    .bind(&knowledge.summary)
    .bind(&knowledge.embedding)
    .bind(&knowledge.source_episode_ids)
    .bind(knowledge.created_at)
    .execute(pool)
    .await?;
    Ok(())
}

/// Fetch the most recent episodes (newest first), up to `limit`.
/// Used for cross-session recall when working memory is empty.
pub async fn search_recent(
    pool: &PgPool,
    limit: i64,
) -> Result<Vec<Episode>, sqlx::Error> {
    let rows = sqlx::query_as::<_, EpisodeRow>(
        "SELECT id, topic_id, content, embedding, salience, is_consolidated, created_at \
         FROM episodes ORDER BY created_at DESC LIMIT $1",
    )
    .bind(limit)
    .fetch_all(pool)
    .await?;

    Ok(rows.into_iter().map(Into::into).collect())
}

/// Internal row type for sqlx deserialization.
#[derive(sqlx::FromRow)]
struct EpisodeRow {
    id: Uuid,
    topic_id: Option<Uuid>,
    content: String,
    embedding: Option<Vec<u8>>,
    salience: f32,
    is_consolidated: bool,
    created_at: chrono::DateTime<chrono::Utc>,
}

impl From<EpisodeRow> for Episode {
    fn from(row: EpisodeRow) -> Self {
        Self {
            id: row.id,
            topic_id: row.topic_id,
            content: row.content,
            embedding: row.embedding,
            salience: row.salience,
            is_consolidated: row.is_consolidated,
            created_at: row.created_at,
        }
    }
}

