use super::perception;
use crate::types::{
    GatedEvent, ReflexAction, ReflexDecision,
};
use uuid::Uuid;

/// Fast path threshold: threat level that triggers immediate reflex.
const THREAT_THRESHOLD: f32 = 0.75;

/// A registered capability entry for keyword-based matching.
#[derive(Debug)]
struct CapabilityEntry {
    id: Uuid,
    keywords: Vec<String>,
}

/// Fast path processor — capability matching or DirectLlmFallback.
/// Target latency: < 50ms (no LLM calls, pure rule matching).
#[derive(Debug, Default)]
pub struct FastPath {
    registry: Vec<CapabilityEntry>,
}

impl FastPath {
    pub fn new() -> Self {
        Self::default()
    }

    /// Register a capability with keywords for fast matching.
    pub fn register(&mut self, id: Uuid, keywords: Vec<String>) {
        self.registry.push(CapabilityEntry { id, keywords });
    }

    /// Unregister a capability (e.g. on crash or retirement).
    pub fn unregister(&mut self, id: Uuid) {
        self.registry.retain(|entry| entry.id != id);
    }

    /// Evaluate a gated event through the fast path.
    /// Returns `Some(ReflexDecision)` if fast path should handle it,
    /// `None` if the event doesn't qualify for fast processing.
    pub fn evaluate(&self, event: &GatedEvent) -> Option<ReflexDecision> {
        // Fast path triggers:
        // 1. urgent_bypass (already flagged by sensory gating)
        // 2. threat >= 0.75
        // 3. Any external dialogue event (always attempt fast match first)
        let features = perception::extract(event);

        if !event.salience.is_urgent_bypass
            && features.threat < THREAT_THRESHOLD
            && !is_dialogue(event)
        {
            return None;
        }

        // Attempt capability matching
        match self.match_capability(event) {
            Some(capability_id) => Some(ReflexDecision {
                action: ReflexAction::InvokeCapability,
                capability_id: Some(capability_id),
                confidence: features.intent_confidence,
                async_codegen: false,
            }),
            None => {
                // No capability match → DirectLlmFallback
                // Also trigger async codegen for the gap
                Some(ReflexDecision {
                    action: ReflexAction::DirectLlmFallback,
                    capability_id: None,
                    confidence: 0.5, // moderate confidence for LLM fallback
                    async_codegen: true,
                })
            }
        }
    }

    /// Try to match event against known capabilities by keyword overlap.
    /// Returns the capability with the highest keyword hit count.
    fn match_capability(&self, event: &GatedEvent) -> Option<Uuid> {
        if self.registry.is_empty() {
            return None;
        }
        let lower = event.event.content.to_lowercase();
        let mut best: Option<(Uuid, usize)> = None;
        for entry in &self.registry {
            let hits = entry.keywords.iter().filter(|k| lower.contains(k.as_str())).count();
            if hits > 0 && best.is_none_or(|(_, prev)| hits > prev) {
                best = Some((entry.id, hits));
            }
        }
        best.map(|(id, _)| id)
    }
}

fn is_dialogue(event: &GatedEvent) -> bool {
    event.route == crate::types::RouteTarget::TextDialogue
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{GatedEvent, RouteTarget, SalienceScore, SensoryEvent};

    fn make_dialogue_event(content: &str) -> GatedEvent {
        GatedEvent {
            event: SensoryEvent::external(content),
            salience: SalienceScore::compute(0.6, 0.4, 0.3, 0.5, 0.82),
            route: RouteTarget::TextDialogue,
        }
    }

    fn make_urgent_event(content: &str) -> GatedEvent {
        GatedEvent {
            event: SensoryEvent::external(content),
            salience: SalienceScore::compute(0.6, 0.9, 0.3, 0.5, 0.82),
            route: RouteTarget::TextDialogue,
        }
    }

    #[test]
    fn dialogue_event_triggers_fast_path() {
        let fp = FastPath::new();
        let event = make_dialogue_event("hello, how are you?");
        let decision = fp.evaluate(&event);
        assert!(decision.is_some());
        let d = decision.unwrap();
        // No capabilities registered → DirectLlmFallback
        assert_eq!(d.action, ReflexAction::DirectLlmFallback);
        assert!(d.async_codegen);
    }

    #[test]
    fn urgent_event_triggers_fast_path() {
        let fp = FastPath::new();
        let event = make_urgent_event("critical error crash");
        let decision = fp.evaluate(&event);
        assert!(decision.is_some());
    }

    #[test]
    fn internal_low_threat_skips_fast_path() {
        let fp = FastPath::new();
        let event = GatedEvent {
            event: SensoryEvent::internal("idle thought"),
            salience: SalienceScore::compute(0.3, 0.1, 0.2, 0.1, 0.82),
            route: RouteTarget::InternalSignal,
        };
        let decision = fp.evaluate(&event);
        assert!(decision.is_none());
    }

    #[test]
    fn feature_extraction_detects_threat() {
        let event = make_dialogue_event("critical error crash panic");
        let features = perception::extract(&event);
        assert!(features.threat >= 0.5);
    }

    #[test]
    fn intent_classification() {
        assert_eq!(perception::classify_intent("what is this?").0, "question");
        assert_eq!(perception::classify_intent("run the tests").0, "command");
        assert_eq!(perception::classify_intent("help me please").0, "request");
        assert_eq!(perception::classify_intent("the sky is blue").0, "statement");
    }

    #[test]
    fn registered_capability_matches() {
        let mut fp = FastPath::new();
        let cap_id = Uuid::new_v4();
        fp.register(cap_id, vec!["weather".into(), "forecast".into()]);
        let event = make_dialogue_event("what's the weather today?");
        let d = fp.evaluate(&event).unwrap();
        assert_eq!(d.action, ReflexAction::InvokeCapability);
        assert_eq!(d.capability_id, Some(cap_id));
        assert!(!d.async_codegen);
    }

    #[test]
    fn best_keyword_match_wins() {
        let mut fp = FastPath::new();
        let cap_a = Uuid::new_v4();
        let cap_b = Uuid::new_v4();
        fp.register(cap_a, vec!["file".into()]);
        fp.register(cap_b, vec!["file".into(), "read".into(), "open".into()]);
        let event = make_dialogue_event("read and open the file");
        let d = fp.evaluate(&event).unwrap();
        assert_eq!(d.capability_id, Some(cap_b));
    }

    #[test]
    fn no_match_falls_back_to_llm() {
        let mut fp = FastPath::new();
        fp.register(Uuid::new_v4(), vec!["weather".into()]);
        let event = make_dialogue_event("hello, how are you?");
        let d = fp.evaluate(&event).unwrap();
        assert_eq!(d.action, ReflexAction::DirectLlmFallback);
        assert!(d.async_codegen);
    }
}
