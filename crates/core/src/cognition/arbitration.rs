use crate::types::{
    ActionPlan, Decision, DecisionSource, DeliberateDecision, PressureLevel, ReflexAction,
    ReflexDecision,
};

/// Pressure state machine for fast/slow arbitration.
/// Tracks consecutive Critical ticks to trigger fast-only mode.
#[derive(Debug)]
pub struct PressureState {
    pub level: PressureLevel,
    consecutive_critical: u32,
}

impl Default for PressureState {
    fn default() -> Self {
        Self {
            level: PressureLevel::Normal,
            consecutive_critical: 0,
        }
    }
}

impl PressureState {
    pub fn new() -> Self {
        Self::default()
    }

    /// Update pressure level. Tracks consecutive Critical ticks.
    pub fn update(&mut self, level: PressureLevel) {
        if level == PressureLevel::Critical {
            self.consecutive_critical += 1;
        } else {
            self.consecutive_critical = 0;
        }
        self.level = level;
    }

    /// True if 3+ consecutive Critical ticks → fast-only mode.
    pub fn is_fast_only(&self) -> bool {
        self.consecutive_critical >= 3
    }
}

/// Fuse fast and slow path decisions based on pressure state.
///
/// Weight table (from PLAN.md §3.2):
/// - Normal:   slow×0.6 vs fast×0.4
/// - High:     fast×0.7 vs slow×0.3
/// - Critical: fast×0.7 vs slow×0.3
/// - 3+ consecutive Critical: fast-only (ignore slow)
pub fn fuse(
    fast: Option<ReflexDecision>,
    slow: Option<DeliberateDecision>,
    pressure: &PressureState,
) -> Option<Decision> {
    // Fast-only mode: ignore slow path entirely
    if pressure.is_fast_only() {
        return fast.map(reflex_to_decision);
    }

    match (fast, slow) {
        (Some(f), Some(s)) => {
            let (fast_weight, slow_weight) = match pressure.level {
                PressureLevel::Normal => (0.4_f32, 0.6_f32),
                PressureLevel::High | PressureLevel::Critical => (0.7, 0.3),
            };

            let fast_score = f.confidence * fast_weight;
            let slow_score = s.confidence * slow_weight;

            if fast_score >= slow_score {
                Some(reflex_to_decision(f))
            } else {
                Some(deliberate_to_decision(s))
            }
        }
        (Some(f), None) => Some(reflex_to_decision(f)),
        (None, Some(s)) => Some(deliberate_to_decision(s)),
        (None, None) => None,
    }
}

fn reflex_to_decision(reflex: ReflexDecision) -> Decision {
    let async_codegen = reflex.async_codegen;
    let plan = match reflex.action {
        ReflexAction::InvokeCapability => ActionPlan {
            id: uuid::Uuid::new_v4(),
            capability_id: reflex.capability_id,
            method: "invoke_capability".into(),
            params: serde_json::json!({}),
            timeout_ms: 5000,
        },
        ReflexAction::DirectLlmFallback => {
            ActionPlan::direct_llm("direct_llm_fallback", serde_json::json!({}))
        }
    };

    Decision {
        source: DecisionSource::Fast,
        plan,
        confidence: reflex.confidence,
        async_codegen,
    }
}

fn deliberate_to_decision(deliberate: DeliberateDecision) -> Decision {
    Decision {
        source: DecisionSource::Slow,
        plan: deliberate.plan,
        confidence: deliberate.confidence,
        async_codegen: false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_fast(confidence: f32) -> ReflexDecision {
        ReflexDecision {
            action: ReflexAction::DirectLlmFallback,
            capability_id: None,
            confidence,
            async_codegen: true,
        }
    }

    fn make_slow(confidence: f32) -> DeliberateDecision {
        DeliberateDecision {
            plan: ActionPlan::direct_llm("slow_response", serde_json::json!({})),
            confidence,
        }
    }

    #[test]
    fn normal_pressure_prefers_slow() {
        let pressure = PressureState::new();
        let decision = fuse(Some(make_fast(0.6)), Some(make_slow(0.6)), &pressure).unwrap();
        // Equal confidence: Normal weights slow×0.6 > fast×0.4
        assert_eq!(decision.source, DecisionSource::Slow);
    }

    #[test]
    fn high_pressure_prefers_fast() {
        let mut pressure = PressureState::new();
        pressure.update(PressureLevel::High);
        let decision = fuse(Some(make_fast(0.6)), Some(make_slow(0.6)), &pressure).unwrap();
        // Equal confidence: High weights fast×0.7 > slow×0.3
        assert_eq!(decision.source, DecisionSource::Fast);
    }

    #[test]
    fn fast_only_after_three_critical() {
        let mut pressure = PressureState::new();
        pressure.update(PressureLevel::Critical);
        pressure.update(PressureLevel::Critical);
        pressure.update(PressureLevel::Critical);
        assert!(pressure.is_fast_only());

        let decision = fuse(Some(make_fast(0.3)), Some(make_slow(0.9)), &pressure).unwrap();
        // Fast-only mode: slow path ignored regardless of confidence
        assert_eq!(decision.source, DecisionSource::Fast);
    }

    #[test]
    fn consecutive_critical_resets_on_normal() {
        let mut pressure = PressureState::new();
        pressure.update(PressureLevel::Critical);
        pressure.update(PressureLevel::Critical);
        pressure.update(PressureLevel::Normal); // resets counter
        pressure.update(PressureLevel::Critical);
        assert!(!pressure.is_fast_only()); // only 1 consecutive
    }

    #[test]
    fn fast_only_returns_none_when_no_fast() {
        let mut pressure = PressureState::new();
        for _ in 0..3 {
            pressure.update(PressureLevel::Critical);
        }
        let decision = fuse(None, Some(make_slow(0.9)), &pressure);
        assert!(decision.is_none()); // fast-only but no fast decision
    }

    #[test]
    fn both_none_returns_none() {
        let pressure = PressureState::new();
        assert!(fuse(None, None, &pressure).is_none());
    }

    #[test]
    fn fast_only_fallback() {
        let pressure = PressureState::new();
        let decision = fuse(Some(make_fast(0.5)), None, &pressure).unwrap();
        assert_eq!(decision.source, DecisionSource::Fast);
    }

    #[test]
    fn slow_only_fallback() {
        let pressure = PressureState::new();
        let decision = fuse(None, Some(make_slow(0.7)), &pressure).unwrap();
        assert_eq!(decision.source, DecisionSource::Slow);
    }
}
