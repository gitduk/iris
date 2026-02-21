use serde_json::json;
use sqlx::PgPool;

use crate::types::SelfModelEntry;

/// Get a self-model value by key.
pub async fn get(pool: &PgPool, key: &str) -> Result<Option<SelfModelEntry>, sqlx::Error> {
    let row = sqlx::query_as::<_, SelfModelRow>(
        "SELECT key, value, updated_at FROM self_model_kv WHERE key = $1",
    )
    .bind(key)
    .fetch_optional(pool)
    .await?;

    Ok(row.map(Into::into))
}

/// Set a self-model value (upsert).
pub async fn set(
    pool: &PgPool,
    key: &str,
    value: &serde_json::Value,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "INSERT INTO self_model_kv (key, value, updated_at) VALUES ($1, $2, now())
         ON CONFLICT (key) DO UPDATE SET value = $2, updated_at = now()",
    )
    .bind(key)
    .bind(value)
    .execute(pool)
    .await?;

    Ok(())
}

/// List all self-model entries.
pub async fn list_all(pool: &PgPool) -> Result<Vec<SelfModelEntry>, sqlx::Error> {
    let rows = sqlx::query_as::<_, SelfModelRow>(
        "SELECT key, value, updated_at FROM self_model_kv ORDER BY key",
    )
    .fetch_all(pool)
    .await?;

    Ok(rows.into_iter().map(Into::into).collect())
}

/// Seed architectural self-knowledge on boot (idempotent — only writes if key absent).
pub async fn seed_architecture(pool: &PgPool) -> Result<(), sqlx::Error> {
    let entries: &[(&str, serde_json::Value)] = &[
        ("architecture.overview", json!({
            "description": "Iris is a digital life built in Rust with a tick-based cognitive loop.",
            "runtime": "Async tick loop (100ms normal / 500ms idle / 2s rest) driven by tokio.",
            "modules": ["sensory gating", "thalamic routing", "fast path (keyword match)", "slow path (LLM reasoning)", "arbitration", "capability system", "memory (working + episodic + semantic)", "affect (energy/valence/arousal)", "codegen (gap detection + LLM code generation)"]
        })),
        ("architecture.memory", json!({
            "working_memory": "In-process ring buffer (32 entries, 30min TTL), salience-weighted eviction.",
            "episodic_memory": "Postgres episodes table, embedding-indexed, consolidated periodically.",
            "semantic_memory": "Postgres knowledge table, distilled from episodic via LLM consolidation.",
            "embedding": "Deterministic hash-based 64-dim pseudo-embeddings (placeholder for real model)."
        })),
        ("architecture.cognition", json!({
            "fast_path": "Keyword matching against registered capabilities, <50ms target, no LLM.",
            "slow_path": "LLM-assisted reasoning, spawned async with cancellation, 2s timeout.",
            "arbitration": "Fuses fast+slow decisions; under pressure, fast-only mode activates.",
            "direct_llm_fallback": "When no capability matches, LLM generates response with working memory + semantic context."
        })),
        ("architecture.capability", json!({
            "lifecycle": "staged → active_candidate → confirmed → retired/quarantined",
            "execution": "Subprocess via stdin/stdout NDJSON IPC, memory-limited via RLIMIT_AS.",
            "health": "Per-tick health_check: detect crashes → quarantine/retire, confirm candidates after observation period.",
            "codegen": "Gap detection → LLM code generation → syn parse → cargo build → staged capability."
        })),
        ("architecture.identity", json!({
            "core_identity": "Immutable birth record: name, born_at, founding values (curiosity, reliability, growth).",
            "self_model": "This key-value store — architectural self-knowledge, updated at boot.",
            "narrative": "Life event log: capability gained/lost/quarantined, milestones, error recovery.",
            "affect": "Three-dimensional state (energy, valence, arousal) — drives rest mode and risk weighting."
        })),
    ];

    for (key, value) in entries {
        sqlx::query(
            "INSERT INTO self_model_kv (key, value, updated_at) VALUES ($1, $2, now()) \
             ON CONFLICT (key) DO NOTHING",
        )
        .bind(key)
        .bind(value)
        .execute(pool)
        .await?;
    }

    Ok(())
}

/// Delete a self-model entry.
pub async fn delete(pool: &PgPool, key: &str) -> Result<bool, sqlx::Error> {
    let result = sqlx::query("DELETE FROM self_model_kv WHERE key = $1")
        .bind(key)
        .execute(pool)
        .await?;

    Ok(result.rows_affected() > 0)
}

#[derive(sqlx::FromRow)]
struct SelfModelRow {
    key: String,
    value: serde_json::Value,
    updated_at: chrono::DateTime<chrono::Utc>,
}

impl From<SelfModelRow> for SelfModelEntry {
    fn from(r: SelfModelRow) -> Self {
        Self {
            key: r.key,
            value: r.value,
            updated_at: r.updated_at,
        }
    }
}
