use sqlx::PgPool;
use uuid::Uuid;

use crate::types::{NarrativeEvent, NarrativeEventType};

/// Record a narrative event.
pub async fn record(pool: &PgPool, event: &NarrativeEvent) -> Result<(), sqlx::Error> {
    sqlx::query(
        "INSERT INTO narrative_event (id, occurred_at, event_type, description, significance)
         VALUES ($1, $2, $3, $4, $5)",
    )
    .bind(event.id)
    .bind(event.occurred_at)
    .bind(event.event_type.as_str())
    .bind(&event.description)
    .bind(event.significance)
    .execute(pool)
    .await?;

    Ok(())
}

/// Fetch recent narrative events (most recent first).
pub async fn fetch_recent(pool: &PgPool, limit: i64) -> Result<Vec<NarrativeEvent>, sqlx::Error> {
    let rows = sqlx::query_as::<_, NarrativeRow>(
        "SELECT id, occurred_at, event_type, description, significance
         FROM narrative_event ORDER BY occurred_at DESC LIMIT $1",
    )
    .bind(limit)
    .fetch_all(pool)
    .await?;

    Ok(rows.into_iter().map(Into::into).collect())
}

/// Fetch narrative events by type.
pub async fn fetch_by_type(
    pool: &PgPool,
    event_type: NarrativeEventType,
    limit: i64,
) -> Result<Vec<NarrativeEvent>, sqlx::Error> {
    let rows = sqlx::query_as::<_, NarrativeRow>(
        "SELECT id, occurred_at, event_type, description, significance
         FROM narrative_event WHERE event_type = $1
         ORDER BY occurred_at DESC LIMIT $2",
    )
    .bind(event_type.as_str())
    .bind(limit)
    .fetch_all(pool)
    .await?;

    Ok(rows.into_iter().map(Into::into).collect())
}

/// Create a NarrativeEvent with sensible defaults.
pub fn new_event(
    event_type: NarrativeEventType,
    description: impl Into<String>,
    significance: f32,
) -> NarrativeEvent {
    NarrativeEvent {
        id: Uuid::new_v4(),
        occurred_at: chrono::Utc::now(),
        event_type,
        description: description.into(),
        significance: significance.clamp(0.0, 1.0),
    }
}

#[derive(sqlx::FromRow)]
struct NarrativeRow {
    id: Uuid,
    occurred_at: chrono::DateTime<chrono::Utc>,
    event_type: String,
    description: String,
    significance: f32,
}

impl From<NarrativeRow> for NarrativeEvent {
    fn from(r: NarrativeRow) -> Self {
        Self {
            id: r.id,
            occurred_at: r.occurred_at,
            event_type: NarrativeEventType::parse(&r.event_type),
            description: r.description,
            significance: r.significance,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_event_clamps_significance() {
        let e = new_event(NarrativeEventType::MilestoneReached, "test", 1.5);
        assert!((e.significance - 1.0).abs() < f32::EPSILON);

        let e = new_event(NarrativeEventType::Other, "test", -0.5);
        assert!(e.significance.abs() < f32::EPSILON);
    }
}
