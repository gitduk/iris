use sqlx::PgPool;
use uuid::Uuid;

use crate::types::{
    CapabilityManifest, CapabilityRecord, CapabilityScore, CapabilityState,
};

/// Row type for sqlx deserialization from the `capability` table.
#[derive(sqlx::FromRow)]
struct CapabilityRow {
    id: Uuid,
    name: String,
    binary_path: String,
    manifest: serde_json::Value,
    state: String,
    lkg_version: Option<Uuid>,
    quarantine_count: i32,
    created_at: chrono::DateTime<chrono::Utc>,
    updated_at: chrono::DateTime<chrono::Utc>,
}

impl From<CapabilityRow> for CapabilityRecord {
    fn from(row: CapabilityRow) -> Self {
        let manifest: CapabilityManifest =
            serde_json::from_value(row.manifest).unwrap_or_else(|_| CapabilityManifest {
                name: row.name.clone(),
                binary_path: row.binary_path.clone(),
                permissions: vec![],
                resource_limits: serde_json::Value::Null,
                keywords: vec![],
            });
        Self {
            id: row.id,
            name: row.name,
            binary_path: row.binary_path,
            manifest,
            state: CapabilityState::from_db(&row.state).unwrap_or(CapabilityState::Quarantined),
            lkg_version: row.lkg_version,
            quarantine_count: row.quarantine_count,
            created_at: row.created_at,
            updated_at: row.updated_at,
        }
    }
}

/// Insert a new capability record.
pub async fn insert(pool: &PgPool, record: &CapabilityRecord) -> Result<(), sqlx::Error> {
    let manifest_json = serde_json::to_value(&record.manifest).unwrap_or_default();
    sqlx::query(
        "INSERT INTO capability (id, name, binary_path, manifest, state, lkg_version, quarantine_count, created_at, updated_at)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)"
    )
    .bind(record.id)
    .bind(&record.name)
    .bind(&record.binary_path)
    .bind(&manifest_json)
    .bind(record.state.as_db_str())
    .bind(record.lkg_version)
    .bind(record.quarantine_count)
    .bind(record.created_at)
    .bind(record.updated_at)
    .execute(pool)
    .await?;
    Ok(())
}

/// Fetch a capability by ID.
pub async fn fetch_by_id(pool: &PgPool, id: Uuid) -> Result<Option<CapabilityRecord>, sqlx::Error> {
    let row: Option<CapabilityRow> = sqlx::query_as(
        "SELECT id, name, binary_path, manifest, state, lkg_version, quarantine_count, created_at, updated_at
         FROM capability WHERE id = $1"
    )
    .bind(id)
    .fetch_optional(pool)
    .await?;
    Ok(row.map(Into::into))
}
/// Fetch a capability by name.
pub async fn fetch_by_name(pool: &PgPool, name: &str) -> Result<Option<CapabilityRecord>, sqlx::Error> {
    let row: Option<CapabilityRow> = sqlx::query_as(
        "SELECT id, name, binary_path, manifest, state, lkg_version, quarantine_count, created_at, updated_at
         FROM capability WHERE name = $1"
    )
    .bind(name)
    .fetch_optional(pool)
    .await?;
    Ok(row.map(Into::into))
}

/// Fetch all capabilities in a given state.
pub async fn fetch_by_state(pool: &PgPool, state: CapabilityState) -> Result<Vec<CapabilityRecord>, sqlx::Error> {
    let rows: Vec<CapabilityRow> = sqlx::query_as(
        "SELECT id, name, binary_path, manifest, state, lkg_version, quarantine_count, created_at, updated_at
         FROM capability WHERE state = $1 ORDER BY updated_at DESC"
    )
    .bind(state.as_db_str())
    .fetch_all(pool)
    .await?;
    Ok(rows.into_iter().map(Into::into).collect())
}

/// Update capability state and bump updated_at.
pub async fn update_state(pool: &PgPool, id: Uuid, new_state: CapabilityState) -> Result<(), sqlx::Error> {
    sqlx::query(
        "UPDATE capability SET state = $1, updated_at = now() WHERE id = $2"
    )
    .bind(new_state.as_db_str())
    .bind(id)
    .execute(pool)
    .await?;
    Ok(())
}

/// Update LKG version pointer (called when active_candidate → confirmed).
pub async fn update_lkg(pool: &PgPool, id: Uuid, lkg_version: Uuid) -> Result<(), sqlx::Error> {
    sqlx::query(
        "UPDATE capability SET lkg_version = $1, updated_at = now() WHERE id = $2"
    )
    .bind(lkg_version)
    .bind(id)
    .execute(pool)
    .await?;
    Ok(())
}
/// Increment quarantine count.
pub async fn increment_quarantine(pool: &PgPool, id: Uuid) -> Result<i32, sqlx::Error> {
    let row: (i32,) = sqlx::query_as(
        "UPDATE capability SET quarantine_count = quarantine_count + 1, updated_at = now()
         WHERE id = $1 RETURNING quarantine_count"
    )
    .bind(id)
    .fetch_one(pool)
    .await?;
    Ok(row.0)
}

// ── Capability Score operations ────────────────────────────────

/// Initialize a score row for a new capability.
pub async fn init_score(pool: &PgPool, capability_id: Uuid) -> Result<(), sqlx::Error> {
    sqlx::query(
        "INSERT INTO capability_score (capability_id) VALUES ($1) ON CONFLICT DO NOTHING"
    )
    .bind(capability_id)
    .execute(pool)
    .await?;
    Ok(())
}

/// Record a usage outcome (success or failure).
pub async fn record_outcome(pool: &PgPool, capability_id: Uuid, success: bool) -> Result<(), sqlx::Error> {
    if success {
        sqlx::query(
            "UPDATE capability_score SET usage_count = usage_count + 1, success_count = success_count + 1, updated_at = now()
             WHERE capability_id = $1"
        )
        .bind(capability_id)
        .execute(pool)
        .await?;
    } else {
        sqlx::query(
            "UPDATE capability_score SET usage_count = usage_count + 1, fail_count = fail_count + 1, updated_at = now()
             WHERE capability_id = $1"
        )
        .bind(capability_id)
        .execute(pool)
        .await?;
    }
    Ok(())
}

/// Fetch score for a capability.
pub async fn fetch_score(pool: &PgPool, capability_id: Uuid) -> Result<Option<CapabilityScore>, sqlx::Error> {
    let row: Option<ScoreRow> = sqlx::query_as(
        "SELECT capability_id, usage_count, success_count, fail_count, quarantine_count
         FROM capability_score WHERE capability_id = $1"
    )
    .bind(capability_id)
    .fetch_optional(pool)
    .await?;
    Ok(row.map(Into::into))
}

#[derive(sqlx::FromRow)]
struct ScoreRow {
    capability_id: Uuid,
    usage_count: i64,
    success_count: i64,
    fail_count: i64,
    quarantine_count: i32,
}

impl From<ScoreRow> for CapabilityScore {
    fn from(row: ScoreRow) -> Self {
        Self {
            capability_id: row.capability_id,
            usage_count: row.usage_count,
            success_count: row.success_count,
            fail_count: row.fail_count,
            quarantine_count: row.quarantine_count,
        }
    }
}
