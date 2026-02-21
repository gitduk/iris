use crate::config::IrisCfg;
use crate::types::{SensoryEvent, GatedEvent, RouteTarget, EventSource};
use super::salience;

/// Sensory gating: scores events and filters below noise_floor.
/// Returns gated events that passed the filter, with route targets assigned.
pub fn gate(events: Vec<SensoryEvent>, cfg: &IrisCfg) -> Vec<GatedEvent> {
    events
        .into_iter()
        .filter_map(|event| {
            let score = salience::score(&event, cfg.urgent_bypass);

            // Below noise floor → discard
            if score.score < cfg.noise_floor {
                tracing::debug!(
                    event_id = %event.id,
                    score = score.score,
                    noise_floor = cfg.noise_floor,
                    "event below noise floor, discarded"
                );
                return None;
            }

            let route = route_target(&event);

            Some(GatedEvent {
                event,
                salience: score,
                route,
            })
        })
        .collect()
}

/// Determine route target based on event source.
fn route_target(event: &SensoryEvent) -> RouteTarget {
    match event.source {
        EventSource::External => RouteTarget::TextDialogue,
        EventSource::Internal => RouteTarget::InternalSignal,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn default_cfg() -> IrisCfg {
        IrisCfg::default()
    }

    #[test]
    fn gate_filters_low_salience() {
        let cfg = default_cfg();
        let events = vec![
            SensoryEvent::internal(""), // very short internal → low salience
            SensoryEvent::external("hello, how are you doing today?"),
        ];
        let gated = gate(events, &cfg);
        // The external event should pass; the empty internal may be filtered
        assert!(!gated.is_empty());
        assert!(gated.iter().all(|g| g.salience.score >= cfg.noise_floor));
    }

    #[test]
    fn gate_routes_correctly() {
        let cfg = default_cfg();
        let events = vec![
            SensoryEvent::external("test input"),
            SensoryEvent::internal("spontaneous thought about something interesting"),
        ];
        let gated = gate(events, &cfg);
        for g in &gated {
            match g.event.source {
                EventSource::External => assert_eq!(g.route, RouteTarget::TextDialogue),
                EventSource::Internal => assert_eq!(g.route, RouteTarget::InternalSignal),
            }
        }
    }

    #[test]
    fn gate_preserves_urgent_bypass() {
        let cfg = default_cfg();
        let events = vec![
            SensoryEvent::external("critical error crash panic emergency failure"),
        ];
        let gated = gate(events, &cfg);
        assert_eq!(gated.len(), 1);
        assert!(gated[0].salience.is_urgent_bypass);
    }
}
