use sqlx::PgPool;
use uuid::Uuid;

use crate::types::CoreIdentity;

/// Ensure a core identity exists. If none, create one with defaults.
/// Returns the single identity row (there should only ever be one).
pub async fn ensure(pool: &PgPool, name: &str) -> Result<CoreIdentity, sqlx::Error> {
    // Try to fetch existing
    let row = sqlx::query_as::<_, IdentityRow>(
        "SELECT id, name, born_at, founding_values FROM iris_identity LIMIT 1",
    )
    .fetch_optional(pool)
    .await?;

    if let Some(row) = row {
        return Ok(row.into());
    }

    // First boot — create identity
    let id = Uuid::new_v4();
    let born_at = chrono::Utc::now();
    let founding_values = serde_json::json!({
        "curiosity": "explore and learn continuously",
        "reliability": "fulfill commitments accurately",
        "growth": "expand capabilities through experience",
    });

    sqlx::query(
        "INSERT INTO iris_identity (id, name, born_at, founding_values) VALUES ($1, $2, $3, $4)",
    )
    .bind(id)
    .bind(name)
    .bind(born_at)
    .bind(&founding_values)
    .execute(pool)
    .await?;

    Ok(CoreIdentity {
        id,
        name: name.to_string(),
        born_at,
        founding_values,
    })
}

/// Fetch the core identity (returns None if not yet initialized).
pub async fn fetch(pool: &PgPool) -> Result<Option<CoreIdentity>, sqlx::Error> {
    let row = sqlx::query_as::<_, IdentityRow>(
        "SELECT id, name, born_at, founding_values FROM iris_identity LIMIT 1",
    )
    .fetch_optional(pool)
    .await?;

    Ok(row.map(Into::into))
}

#[derive(sqlx::FromRow)]
struct IdentityRow {
    id: Uuid,
    name: String,
    born_at: chrono::DateTime<chrono::Utc>,
    founding_values: serde_json::Value,
}

impl From<IdentityRow> for CoreIdentity {
    fn from(r: IdentityRow) -> Self {
        Self {
            id: r.id,
            name: r.name,
            born_at: r.born_at,
            founding_values: r.founding_values,
        }
    }
}
