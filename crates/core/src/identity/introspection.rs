use sqlx::PgPool;

use crate::identity::{narrative, self_model};
use crate::types::AffectState;

/// Assemble a self-knowledge context string for LLM system prompt injection.
///
/// Sections:
/// 1. Self-model KV entries (architectural knowledge)
/// 2. Recent narrative events (life history)
/// 3. Current affect state (energy/valence/arousal)
///
/// Returns empty string on any DB failure (graceful degradation).
pub async fn build_self_context(pool: &PgPool, affect: &AffectState, builtin_desc: &str) -> String {
    let mut parts = Vec::new();

    // Self-model entries
    if let Ok(entries) = self_model::list_all(pool).await {
        for entry in entries {
            parts.push(format!("[self-knowledge:{}] {}", entry.key, entry.value));
        }
    }

    // Builtin capabilities (no DB dependency)
    if !builtin_desc.is_empty() {
        parts.push(format!("[builtin-capabilities]\n{builtin_desc}"));
    }

    // Recent narrative events
    if let Ok(events) = narrative::fetch_recent(pool, 5).await {
        for evt in events {
            parts.push(format!(
                "[narrative] {}: {}",
                evt.event_type.as_str(),
                evt.description
            ));
        }
    }

    // Current affect
    parts.push(format!(
        "[affect] energy={:.2}, valence={:.2}, arousal={:.2}",
        affect.energy, affect.valence, affect.arousal
    ));

    parts.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn affect_line_format() {
        let affect = AffectState {
            energy: 0.85,
            valence: 0.60,
            arousal: 0.25,
        };
        let line = format!(
            "[affect] energy={:.2}, valence={:.2}, arousal={:.2}",
            affect.energy, affect.valence, affect.arousal
        );
        assert_eq!(line, "[affect] energy=0.85, valence=0.60, arousal=0.25");
    }
}
