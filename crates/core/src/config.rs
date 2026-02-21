use serde::{Deserialize, Serialize};
use sqlx::PgPool;
use std::collections::HashMap;

/// All iris system parameters. Loaded from `iris_config` table at startup.
/// First boot writes defaults; subsequent boots read existing values.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IrisCfg {
    // tick intervals (ms)
    pub tick_ms_normal: u64,
    pub tick_ms_idle: u64,
    pub tick_ms_rest: u64,

    // sensory gating
    pub noise_floor: f32,
    pub urgent_bypass: f32,
    pub slow_path_complexity: f32,

    // dialogue
    pub commit_window_ms: u64,

    // working memory
    pub working_memory_cap: usize,
    pub working_memory_ttl_secs: u64,

    // memory consolidation & replay
    pub replay_salience: f32,
    pub consolidation_interval_secs: u64,

    // codegen limits
    pub codegen_max_concurrent: usize,
    pub codegen_max_per_hour: usize,
    pub codegen_max_repair: usize,
    pub codegen_compile_timeout_secs: u64,

    // capability lifecycle
    pub candidate_observe_min_secs: u64,
    pub safe_mode_failures: usize,
    pub safe_mode_cooldown_secs: u64,
    pub safe_mode_recovery_ticks: u32,
    pub max_active_topics: usize,

    // shutdown
    pub shutdown_timeout_secs: u64,

    // LLM budget
    pub llm_tokens_per_min: u64,
    pub llm_calls_per_tick: usize,

    // embedding cache
    pub embedding_cache_cap: usize,
    pub embedding_cache_ttl_secs: u64,

    // episodic recall
    pub episodic_recall_threshold: usize,

    // resource
    pub ram_safety_margin_mb: u64,
    pub proactive_interval_secs: u64,
    pub narrative_interval_secs: u64,
}

impl Default for IrisCfg {
    fn default() -> Self {
        Self {
            tick_ms_normal: 100,
            tick_ms_idle: 500,
            tick_ms_rest: 2000,
            noise_floor: 0.20,
            urgent_bypass: 0.82,
            slow_path_complexity: 0.55,
            commit_window_ms: 600,
            working_memory_cap: 32,
            working_memory_ttl_secs: 1800,
            replay_salience: 0.45,
            consolidation_interval_secs: 1800,
            codegen_max_concurrent: 1,
            codegen_max_per_hour: 10,
            codegen_max_repair: 3,
            codegen_compile_timeout_secs: 120,
            candidate_observe_min_secs: 600,
            safe_mode_failures: 3,
            safe_mode_cooldown_secs: 300,
            safe_mode_recovery_ticks: 5,
            max_active_topics: 8,
            shutdown_timeout_secs: 15,
            llm_tokens_per_min: 10000,
            llm_calls_per_tick: 4,
            embedding_cache_cap: 1024,
            embedding_cache_ttl_secs: 300,
            episodic_recall_threshold: 3,
            ram_safety_margin_mb: 512,
            proactive_interval_secs: 300,
            narrative_interval_secs: 86400,
        }
    }
}

impl IrisCfg {
    /// Load config from `iris_config` table. If table is empty, seed with defaults.
    pub async fn load(pool: &PgPool) -> Result<Self, sqlx::Error> {
        let rows: Vec<(String, String)> =
            sqlx::query_as("SELECT key, value FROM iris_config")
                .fetch_all(pool)
                .await?;

        if rows.is_empty() {
            let cfg = Self::default();
            cfg.seed(pool).await?;
            return Ok(cfg);
        }

        let map: HashMap<String, String> = rows.into_iter().collect();
        Ok(Self::from_map(&map))
    }

    /// Write all default values into `iris_config` table.
    async fn seed(&self, pool: &PgPool) -> Result<(), sqlx::Error> {
        let entries = self.to_entries();
        for (key, value, desc) in &entries {
            sqlx::query(
                "INSERT INTO iris_config (key, value, description) VALUES ($1, $2, $3) \
                 ON CONFLICT (key) DO NOTHING",
            )
            .bind(key)
            .bind(value)
            .bind(desc)
            .execute(pool)
            .await?;
        }
        Ok(())
    }

    fn from_map(m: &HashMap<String, String>) -> Self {
        let d = Self::default();
        Self {
            tick_ms_normal: get_or(m, "tick_ms_normal", d.tick_ms_normal),
            tick_ms_idle: get_or(m, "tick_ms_idle", d.tick_ms_idle),
            tick_ms_rest: get_or(m, "tick_ms_rest", d.tick_ms_rest),
            noise_floor: get_or(m, "noise_floor", d.noise_floor),
            urgent_bypass: get_or(m, "urgent_bypass", d.urgent_bypass),
            slow_path_complexity: get_or(m, "slow_path_complexity", d.slow_path_complexity),
            commit_window_ms: get_or(m, "commit_window_ms", d.commit_window_ms),
            working_memory_cap: get_or(m, "working_memory_cap", d.working_memory_cap),
            working_memory_ttl_secs: get_or(m, "working_memory_ttl_secs", d.working_memory_ttl_secs),
            replay_salience: get_or(m, "replay_salience", d.replay_salience),
            consolidation_interval_secs: get_or(m, "consolidation_interval_secs", d.consolidation_interval_secs),
            codegen_max_concurrent: get_or(m, "codegen_max_concurrent", d.codegen_max_concurrent),
            codegen_max_per_hour: get_or(m, "codegen_max_per_hour", d.codegen_max_per_hour),
            codegen_max_repair: get_or(m, "codegen_max_repair", d.codegen_max_repair),
            codegen_compile_timeout_secs: get_or(m, "codegen_compile_timeout_secs", d.codegen_compile_timeout_secs),
            candidate_observe_min_secs: get_or(m, "candidate_observe_min_secs", d.candidate_observe_min_secs),
            safe_mode_failures: get_or(m, "safe_mode_failures", d.safe_mode_failures),
            safe_mode_cooldown_secs: get_or(m, "safe_mode_cooldown_secs", d.safe_mode_cooldown_secs),
            safe_mode_recovery_ticks: get_or(m, "safe_mode_recovery_ticks", d.safe_mode_recovery_ticks),
            max_active_topics: get_or(m, "max_active_topics", d.max_active_topics),
            shutdown_timeout_secs: get_or(m, "shutdown_timeout_secs", d.shutdown_timeout_secs),
            llm_tokens_per_min: get_or(m, "llm_tokens_per_min", d.llm_tokens_per_min),
            llm_calls_per_tick: get_or(m, "llm_calls_per_tick", d.llm_calls_per_tick),
            embedding_cache_cap: get_or(m, "embedding_cache_cap", d.embedding_cache_cap),
            embedding_cache_ttl_secs: get_or(m, "embedding_cache_ttl_secs", d.embedding_cache_ttl_secs),
            episodic_recall_threshold: get_or(m, "episodic_recall_threshold", d.episodic_recall_threshold),
            ram_safety_margin_mb: get_or(m, "ram_safety_margin_mb", d.ram_safety_margin_mb),
            proactive_interval_secs: get_or(m, "proactive_interval_secs", d.proactive_interval_secs),
            narrative_interval_secs: get_or(m, "narrative_interval_secs", d.narrative_interval_secs),
        }
    }

    fn to_entries(&self) -> Vec<(&str, String, &str)> {
        vec![
            ("tick_ms_normal", self.tick_ms_normal.to_string(), "Normal tick interval ms"),
            ("tick_ms_idle", self.tick_ms_idle.to_string(), "Idle tick interval ms"),
            ("tick_ms_rest", self.tick_ms_rest.to_string(), "Rest tick interval ms"),
            ("noise_floor", self.noise_floor.to_string(), "Salience filter threshold"),
            ("urgent_bypass", self.urgent_bypass.to_string(), "Urgent bypass threshold"),
            ("slow_path_complexity", self.slow_path_complexity.to_string(), "Slow path trigger threshold"),
            ("commit_window_ms", self.commit_window_ms.to_string(), "Silent commit window ms"),
            ("working_memory_cap", self.working_memory_cap.to_string(), "Working memory max entries"),
            ("working_memory_ttl_secs", self.working_memory_ttl_secs.to_string(), "Working memory TTL seconds"),
            ("replay_salience", self.replay_salience.to_string(), "Replay trigger threshold"),
            ("consolidation_interval_secs", self.consolidation_interval_secs.to_string(), "Consolidation interval seconds"),
            ("codegen_max_concurrent", self.codegen_max_concurrent.to_string(), "Max concurrent codegen tasks"),
            ("codegen_max_per_hour", self.codegen_max_per_hour.to_string(), "Max codegen per hour"),
            ("codegen_max_repair", self.codegen_max_repair.to_string(), "Max repair iterations"),
            ("codegen_compile_timeout_secs", self.codegen_compile_timeout_secs.to_string(), "Cargo build timeout seconds"),
            ("candidate_observe_min_secs", self.candidate_observe_min_secs.to_string(), "Active candidate observation period"),
            ("safe_mode_failures", self.safe_mode_failures.to_string(), "Consecutive failures to trigger safe mode"),
            ("safe_mode_cooldown_secs", self.safe_mode_cooldown_secs.to_string(), "Safe mode cooldown before exit"),
            ("safe_mode_recovery_ticks", self.safe_mode_recovery_ticks.to_string(), "Healthy ticks to exit safe mode"),
            ("max_active_topics", self.max_active_topics.to_string(), "Max active conversation topics"),
            ("shutdown_timeout_secs", self.shutdown_timeout_secs.to_string(), "Graceful shutdown timeout seconds"),
            ("llm_tokens_per_min", self.llm_tokens_per_min.to_string(), "LLM token budget per minute"),
            ("llm_calls_per_tick", self.llm_calls_per_tick.to_string(), "Max LLM calls per tick"),
            ("embedding_cache_cap", self.embedding_cache_cap.to_string(), "Embedding cache capacity"),
            ("embedding_cache_ttl_secs", self.embedding_cache_ttl_secs.to_string(), "Embedding cache TTL seconds"),
            ("episodic_recall_threshold", self.episodic_recall_threshold.to_string(), "Working memory count below which episodic recall activates"),
            ("ram_safety_margin_mb", self.ram_safety_margin_mb.to_string(), "RAM safety margin MB"),
            ("proactive_interval_secs", self.proactive_interval_secs.to_string(), "Proactive output min interval"),
            ("narrative_interval_secs", self.narrative_interval_secs.to_string(), "Narrative synthesis interval"),
        ]
    }
}

fn get_or<T: std::str::FromStr>(map: &HashMap<String, String>, key: &str, default: T) -> T {
    map.get(key)
        .and_then(|v| v.parse().ok())
        .unwrap_or(default)
}

