use crate::types::{EventSource, SalienceScore, SensoryEvent};

/// Rule-based salience scorer (v1).
/// In v2 this will incorporate LLM-based feature extraction.
pub fn score(event: &SensoryEvent, urgent_bypass_threshold: f32) -> SalienceScore {
    let novelty = estimate_novelty(event);
    let urgency = estimate_urgency(event);
    let complexity = estimate_complexity(event);
    let task_relevance = estimate_task_relevance(event);

    SalienceScore::compute(novelty, urgency, complexity, task_relevance, urgent_bypass_threshold)
}

/// Heuristic novelty: external events are more novel than internal.
fn estimate_novelty(event: &SensoryEvent) -> f32 {
    let base = match event.source {
        EventSource::External => 0.6,
        EventSource::Internal => 0.3,
    };
    // Longer content slightly more novel (capped)
    let length_bonus = (event.content.len() as f32 / 500.0).min(0.3);
    (base + length_bonus).min(1.0)
}

/// Heuristic urgency: keyword scan for urgent markers.
fn estimate_urgency(event: &SensoryEvent) -> f32 {
    let lower = event.content.to_lowercase();
    let urgent_keywords = ["error", "crash", "fail", "urgent", "emergency", "panic", "critical"];
    let matches = urgent_keywords.iter().filter(|k| lower.contains(*k)).count();
    let base = match event.source {
        EventSource::External => 0.4,
        EventSource::Internal => 0.1,
    };
    (base + matches as f32 * 0.2).min(1.0)
}

/// Heuristic complexity: longer content and question marks suggest higher complexity.
fn estimate_complexity(event: &SensoryEvent) -> f32 {
    let length_factor = (event.content.len() as f32 / 200.0).min(0.6);
    let question_bonus = if event.content.contains('?') { 0.2 } else { 0.0 };
    (length_factor + question_bonus).min(1.0)
}

/// Heuristic task relevance: placeholder — always moderate for external, low for internal.
/// Will be replaced by embedding similarity to active working memory topics.
fn estimate_task_relevance(event: &SensoryEvent) -> f32 {
    match event.source {
        EventSource::External => 0.5,
        EventSource::Internal => 0.2,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn external_event_scores_higher() {
        let ext = SensoryEvent::external("hello world");
        let int = SensoryEvent::internal("hello world");
        let s_ext = score(&ext, 0.82);
        let s_int = score(&int, 0.82);
        assert!(s_ext.score > s_int.score);
    }

    #[test]
    fn urgent_keyword_boosts_urgency() {
        let normal = SensoryEvent::external("how are you?");
        let urgent = SensoryEvent::external("critical error crash");
        let s_normal = score(&normal, 0.82);
        let s_urgent = score(&urgent, 0.82);
        assert!(s_urgent.urgency > s_normal.urgency);
    }

    #[test]
    fn urgent_bypass_triggers() {
        let event = SensoryEvent::external("critical error crash panic emergency");
        let s = score(&event, 0.82);
        assert!(s.is_urgent_bypass);
    }
}
