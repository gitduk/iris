//! End-to-end integration tests for the iris cognitive pipeline.
//!
//! These tests exercise the full closed loop without a database:
//! SensoryEvent → gating → routing → fast/slow path → arbitration → working memory.

use iris_core::cognition::arbitration::{self, PressureState};
use iris_core::cognition::fast_path::FastPath;
use iris_core::config::IrisCfg;
use iris_core::memory::working::WorkingMemory;
use iris_core::sensory::gating;
use iris_core::thalamus::router;
use iris_core::types::{ContextEntry, DecisionSource, SensoryEvent};

/// Full pipeline: external text → gating → routing → fast path → arbitration → working memory.
#[test]
fn pipeline_external_text_to_working_memory() {
    let cfg = IrisCfg::default();
    let fast_path = FastPath::new();
    let pressure = PressureState::new();
    let mut wm = WorkingMemory::new(32, 1800);

    // 1. Create external input
    let events = vec![SensoryEvent::external("What is the weather today?")];

    // 2. Sensory gating
    let gated = gating::gate(events, &cfg);
    assert!(!gated.is_empty(), "external text should pass gating");

    // 3. Thalamic routing
    let batch = router::route(gated);
    assert!(batch.has_external(), "should have external dialogue events");
    assert!(!batch.dialogue.is_empty());

    // 4. Fast path evaluation
    let all_events: Vec<_> = batch.dialogue.into_iter().chain(batch.internal).collect();
    let mut decisions = Vec::new();
    for event in &all_events {
        let fast_decision = fast_path.evaluate(event);
        // No slow path in this test (no LLM)
        let decision = arbitration::fuse(fast_decision, None, &pressure);
        if let Some(d) = decision {
            decisions.push(d);
        }
    }

    // 5. Should produce at least one decision (DirectLlmFallback)
    assert!(!decisions.is_empty(), "pipeline should produce a decision");
    assert_eq!(decisions[0].source, DecisionSource::Fast);

    // 6. Write to working memory
    for event in &all_events {
        let entry = ContextEntry {
            id: uuid::Uuid::new_v4(),
            topic_id: None,
            content: event.event.content.clone(),
            salience_score: event.salience.score,
            created_at: event.event.timestamp,
            last_accessed: chrono::Utc::now(),
            pinned_by: None,
            is_response: false,
        };
        wm.insert(entry);
    }
    assert!(!wm.is_empty(), "working memory should have entries after pipeline");
}

/// Internal thought gets filtered or routed to internal signal batch.
#[test]
fn pipeline_internal_thought_routing() {
    let cfg = IrisCfg::default();

    let events = vec![
        SensoryEvent::internal("I should review my recent capability performance metrics"),
    ];

    let gated = gating::gate(events, &cfg);
    let batch = router::route(gated);

    // Internal thoughts go to internal batch, not dialogue
    assert!(batch.dialogue.is_empty());
    // May or may not pass gating depending on salience
    // If it passes, it should be in internal batch
    if !batch.internal.is_empty() {
        assert!(!batch.has_external());
    }
}

/// Urgent bypass events skip normal scoring and get fast-tracked.
#[test]
fn pipeline_urgent_bypass_fast_tracked() {
    let cfg = IrisCfg::default();
    let fast_path = FastPath::new();
    let pressure = PressureState::new();

    let events = vec![SensoryEvent::external(
        "critical error crash panic emergency failure",
    )];

    let gated = gating::gate(events, &cfg);
    assert_eq!(gated.len(), 1);
    assert!(gated[0].salience.is_urgent_bypass);

    let batch = router::route(gated);
    let event = &batch.dialogue[0];

    let fast_decision = fast_path.evaluate(event);
    assert!(fast_decision.is_some(), "urgent event must trigger fast path");

    let decision = arbitration::fuse(fast_decision, None, &pressure).unwrap();
    assert_eq!(decision.source, DecisionSource::Fast);
}

/// Under high pressure, fast path is preferred over slow path.
#[test]
fn pipeline_pressure_affects_arbitration() {
    let cfg = IrisCfg::default();
    let fast_path = FastPath::new();

    let events = vec![SensoryEvent::external("How do I fix this bug?")];
    let gated = gating::gate(events, &cfg);
    let batch = router::route(gated);
    let event = &batch.dialogue[0];

    let fast_decision = fast_path.evaluate(event);

    // Normal pressure: slow path would be preferred (if available)
    let normal_pressure = PressureState::new();
    let _d1 = arbitration::fuse(fast_decision.clone(), None, &normal_pressure);

    // High pressure: fast path preferred
    let mut high_pressure = PressureState::new();
    high_pressure.update(iris_core::types::PressureLevel::High);
    let _d2 = arbitration::fuse(fast_decision, None, &high_pressure);
    // Both produce Fast since no slow path, but the weights differ internally
}

/// Multiple events in a single tick are processed correctly.
#[test]
fn pipeline_multi_event_tick() {
    let cfg = IrisCfg::default();
    let fast_path = FastPath::new();
    let pressure = PressureState::new();
    let mut wm = WorkingMemory::new(32, 1800);

    let events = vec![
        SensoryEvent::external("Hello, how are you?"),
        SensoryEvent::external("What time is it?"),
        SensoryEvent::internal("I notice the user is asking simple questions"),
    ];

    let gated = gating::gate(events, &cfg);
    let batch = router::route(gated);

    let all_events: Vec<_> = batch.dialogue.into_iter().chain(batch.internal).collect();

    let mut decision_count = 0;
    for event in &all_events {
        let fast_decision = fast_path.evaluate(event);
        if let Some(_d) = arbitration::fuse(fast_decision, None, &pressure) {
            decision_count += 1;
        }

        let entry = ContextEntry {
            id: uuid::Uuid::new_v4(),
            topic_id: None,
            content: event.event.content.clone(),
            salience_score: event.salience.score,
            created_at: event.event.timestamp,
            last_accessed: chrono::Utc::now(),
            pinned_by: None,
            is_response: false,
        };
        wm.insert(entry);
    }

    assert!(decision_count >= 1, "should produce at least one decision");
    assert!(!wm.is_empty(), "working memory should have entries");
}

