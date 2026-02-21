use crate::types::{GatedEvent, RouteTarget};

/// Routed event batches, sorted by priority for the tick pipeline.
#[derive(Debug, Default)]
pub struct RoutedBatch {
    /// External dialogue events — processed first (high priority).
    pub dialogue: Vec<GatedEvent>,
    /// Internal signals (replay, spontaneous thought) — processed after dialogue.
    pub internal: Vec<GatedEvent>,
    /// System events (resource alerts, capability state) — dispatched directly.
    pub system: Vec<GatedEvent>,
}

impl RoutedBatch {
    /// True if no events in any category.
    pub fn is_empty(&self) -> bool {
        self.dialogue.is_empty() && self.internal.is_empty() && self.system.is_empty()
    }

    /// True if there are external dialogue events.
    pub fn has_external(&self) -> bool {
        !self.dialogue.is_empty()
    }

    /// Total event count across all categories.
    pub fn len(&self) -> usize {
        self.dialogue.len() + self.internal.len() + self.system.len()
    }
}

/// Route gated events into priority-sorted batches.
/// Within each batch, urgent-bypass events come first, then sorted by salience descending.
pub fn route(events: Vec<GatedEvent>) -> RoutedBatch {
    let mut batch = RoutedBatch::default();

    for event in events {
        match event.route {
            RouteTarget::TextDialogue => batch.dialogue.push(event),
            RouteTarget::InternalSignal => batch.internal.push(event),
            RouteTarget::SystemEvent => batch.system.push(event),
        }
    }

    // Sort each batch: urgent bypass first, then by salience descending
    let sort_fn = |a: &GatedEvent, b: &GatedEvent| {
        b.salience
            .is_urgent_bypass
            .cmp(&a.salience.is_urgent_bypass)
            .then(b.salience.score.partial_cmp(&a.salience.score).unwrap_or(std::cmp::Ordering::Equal))
    };

    batch.dialogue.sort_by(sort_fn);
    batch.internal.sort_by(sort_fn);
    batch.system.sort_by(sort_fn);

    batch
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{EventSource, SalienceScore, SensoryEvent};

    fn make_gated(source: EventSource, route: RouteTarget, score: f32, urgent: bool) -> GatedEvent {
        GatedEvent {
            event: match source {
                EventSource::External => SensoryEvent::external("test"),
                EventSource::Internal => SensoryEvent::internal("test"),
            },
            salience: SalienceScore {
                score,
                novelty: 0.5,
                urgency: if urgent { 0.9 } else { 0.3 },
                complexity: 0.3,
                task_relevance: 0.3,
                is_urgent_bypass: urgent,
            },
            route,
        }
    }

    #[test]
    fn routes_to_correct_batches() {
        let events = vec![
            make_gated(EventSource::External, RouteTarget::TextDialogue, 0.6, false),
            make_gated(EventSource::Internal, RouteTarget::InternalSignal, 0.4, false),
            make_gated(EventSource::External, RouteTarget::SystemEvent, 0.5, false),
        ];
        let batch = route(events);
        assert_eq!(batch.dialogue.len(), 1);
        assert_eq!(batch.internal.len(), 1);
        assert_eq!(batch.system.len(), 1);
        assert!(batch.has_external());
    }

    #[test]
    fn urgent_bypass_sorted_first() {
        let events = vec![
            make_gated(EventSource::External, RouteTarget::TextDialogue, 0.9, false),
            make_gated(EventSource::External, RouteTarget::TextDialogue, 0.5, true),
        ];
        let batch = route(events);
        assert!(batch.dialogue[0].salience.is_urgent_bypass);
        assert!(!batch.dialogue[1].salience.is_urgent_bypass);
    }

    #[test]
    fn empty_batch() {
        let batch = route(vec![]);
        assert!(batch.is_empty());
        assert_eq!(batch.len(), 0);
    }
}
