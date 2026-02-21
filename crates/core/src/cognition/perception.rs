//! Perception module — rule-based PerceptFeature extraction.
//!
//! Extracts threat level, complexity, intent tag, and intent confidence
//! from a GatedEvent without any LLM calls (< 1ms target).

use crate::types::{GatedEvent, PerceptFeature};

/// Threat keywords and their implicit severity.
const THREAT_KEYWORDS: &[&str] = &[
    "error", "crash", "panic", "fail", "critical", "emergency", "attack",
];

/// Extract perceptual features from a gated event.
pub fn extract(event: &GatedEvent) -> PerceptFeature {
    let lower = event.event.content.to_lowercase();

    let threat = compute_threat(&lower);
    let complexity_raw = compute_complexity(&event.event.content);
    let (intent_tag, intent_confidence) = classify_intent(&lower);

    PerceptFeature {
        threat,
        complexity_raw,
        intent_tag,
        intent_confidence,
    }
}

/// Compute threat score from keyword density (0.0–1.0).
fn compute_threat(lower: &str) -> f32 {
    let count = THREAT_KEYWORDS
        .iter()
        .filter(|k| lower.contains(*k))
        .count();
    (count as f32 * 0.25).min(1.0)
}

/// Compute raw complexity from content length (0.0–1.0).
fn compute_complexity(content: &str) -> f32 {
    (content.len() as f32 / 200.0).min(1.0)
}

/// Rule-based intent classification.
/// Returns (intent_tag, confidence).
pub fn classify_intent(text: &str) -> (String, f32) {
    if text.contains('?')
        || text.starts_with("what")
        || text.starts_with("how")
        || text.starts_with("why")
        || text.starts_with("when")
        || text.starts_with("where")
        || text.starts_with("who")
    {
        ("question".into(), 0.7)
    } else if text.starts_with("do ")
        || text.starts_with("run ")
        || text.starts_with("create ")
        || text.starts_with("make ")
        || text.starts_with("delete ")
        || text.starts_with("stop ")
    {
        ("command".into(), 0.8)
    } else if text.contains("thanks") || text.contains("great") || text.contains("good") {
        ("feedback".into(), 0.65)
    } else if text.contains("help") || text.contains("please") {
        ("request".into(), 0.6)
    } else {
        ("statement".into(), 0.4)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{GatedEvent, RouteTarget, SalienceScore, SensoryEvent};

    fn make_event(content: &str) -> GatedEvent {
        GatedEvent {
            event: SensoryEvent::external(content),
            salience: SalienceScore::compute(0.5, 0.3, 0.3, 0.4, 0.82),
            route: RouteTarget::TextDialogue,
        }
    }

    #[test]
    fn threat_detection() {
        let event = make_event("critical error crash panic");
        let features = extract(&event);
        assert!(features.threat >= 0.75);
    }

    #[test]
    fn no_threat() {
        let event = make_event("hello, how are you?");
        let features = extract(&event);
        assert!(features.threat < 0.25);
    }

    #[test]
    fn complexity_scales_with_length() {
        let short = make_event("hi");
        let long = make_event(&"a".repeat(300));
        assert!(extract(&short).complexity_raw < extract(&long).complexity_raw);
    }

    #[test]
    fn intent_question() {
        assert_eq!(classify_intent("what is this?").0, "question");
        assert_eq!(classify_intent("how do I fix it?").0, "question");
        assert_eq!(classify_intent("why did it fail").0, "question");
        assert_eq!(classify_intent("when does it start?").0, "question");
    }

    #[test]
    fn intent_command() {
        assert_eq!(classify_intent("run the tests").0, "command");
        assert_eq!(classify_intent("create a new file").0, "command");
        assert_eq!(classify_intent("delete the old data").0, "command");
    }

    #[test]
    fn intent_request() {
        assert_eq!(classify_intent("help me with this").0, "request");
        assert_eq!(classify_intent("could you please check").0, "request");
    }

    #[test]
    fn intent_feedback() {
        assert_eq!(classify_intent("thanks for the help").0, "feedback");
        assert_eq!(classify_intent("that was great").0, "feedback");
    }

    #[test]
    fn intent_statement() {
        assert_eq!(classify_intent("the sky is blue").0, "statement");
    }
}
