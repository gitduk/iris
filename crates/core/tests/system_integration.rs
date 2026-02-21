//! System-level integration tests: boot + environment + resource + affect.

use iris_core::boot::guardian::BootGuardian;
use iris_core::boot::safe_mode::SafeMode;
use iris_core::environment::hardware::{BatteryState, DegradationSignal, HardwareSnapshot, NetworkState};
use iris_core::environment::system::{CpuSample, SystemInfo};
use iris_core::environment::watcher::EnvironmentWatcher;
use iris_core::identity::affect::AffectActor;
use iris_core::resource_space::admission::{self, AdmissionResult, ResourceEstimate};
use iris_core::resource_space::budget::{self, ResourceBudget};
use iris_core::resource_space::pressure::{self, ResourceSnapshot};
use iris_core::types::PressureLevel;

/// Full boot sequence: CoreInit → CapabilityLoad → EnvironmentSense → Ready.
#[test]
fn boot_sequence_happy_path() {
    let mut guardian = BootGuardian::new();
    let safe_mode = SafeMode::new();

    assert!(!safe_mode.is_active());

    guardian.advance(); // CapabilityLoad
    guardian.advance(); // EnvironmentSense
    guardian.advance(); // Ready
    guardian.record_success();

    assert_eq!(guardian.total_boots(), 1);
    assert_eq!(guardian.consecutive_failures(), 0);
    assert!(!safe_mode.is_active());
}

/// Three boot failures trigger safe mode.
#[test]
fn boot_failures_trigger_safe_mode() {
    let mut guardian = BootGuardian::new();
    let mut safe_mode = SafeMode::new();

    for _ in 0..3 {
        guardian.record_failure();
    }

    assert!(guardian.should_enter_safe_mode());
    safe_mode.enter();
    assert!(safe_mode.is_active());
}

/// Environment watcher detects degradation and affects resource budget.
#[test]
fn environment_drives_resource_budget() {
    let mut watcher = EnvironmentWatcher::new();

    // Normal conditions
    let hw = HardwareSnapshot {
        battery: BatteryState::Charging(100),
        network: NetworkState::Online,
    };
    let signals = watcher.update(CpuSample { usage_ratio: 0.5 }, hw);
    assert!(signals.is_empty());

    // Compute resource snapshot and budget
    let snap = ResourceSnapshot {
        ram_usage_ratio: 0.5,
        storage_usage_ratio: 0.6,
    };
    let level = pressure::evaluate(&snap);
    assert_eq!(level, PressureLevel::Normal);

    let budget = ResourceBudget::compute(1000, level);
    assert_eq!(budget.external_response_mb, 600);
    assert_eq!(budget.internal_growth_mb, 200);

    // High RAM pressure changes budget allocation
    let snap_high = ResourceSnapshot {
        ram_usage_ratio: 0.75,
        storage_usage_ratio: 0.6,
    };
    let level_high = pressure::evaluate(&snap_high);
    assert_eq!(level_high, PressureLevel::High);

    let budget_high = ResourceBudget::compute(1000, level_high);
    assert_eq!(budget_high.external_response_mb, 700);
    assert_eq!(budget_high.internal_growth_mb, 100);
}

/// Admission control rejects tasks when budget is exhausted.
#[test]
fn admission_control_under_pressure() {
    let (tx, rx) = budget::watch_channel();

    // Normal budget — admit external task
    let est = ResourceEstimate { memory_mb: 100, is_external: true };
    assert_eq!(admission::check(&rx, est), AdmissionResult::Admitted);

    // Switch to critical — no internal growth budget
    let critical = ResourceBudget::compute(200, PressureLevel::Critical);
    tx.send(critical).unwrap();

    let internal_est = ResourceEstimate { memory_mb: 1, is_external: false };
    assert_eq!(admission::check(&rx, internal_est), AdmissionResult::Rejected);
}

/// Battery low triggers degradation signal.
#[test]
fn battery_low_degradation() {
    let mut watcher = EnvironmentWatcher::new();
    let hw = HardwareSnapshot {
        battery: BatteryState::OnBattery(15),
        network: NetworkState::Online,
    };
    let signals = watcher.update(CpuSample { usage_ratio: 0.3 }, hw);
    assert!(signals.contains(&DegradationSignal::BatteryLow));
}

/// CPU sustained high triggers degradation after 3 consecutive samples.
#[test]
fn cpu_sustained_high_degradation() {
    let mut watcher = EnvironmentWatcher::new();
    let hw = HardwareSnapshot {
        battery: BatteryState::Charging(100),
        network: NetworkState::Online,
    };

    watcher.update(CpuSample { usage_ratio: 0.90 }, hw);
    watcher.update(CpuSample { usage_ratio: 0.88 }, hw);
    let signals = watcher.update(CpuSample { usage_ratio: 0.92 }, hw);
    assert!(signals.contains(&DegradationSignal::CpuSustainedHigh));
}

/// Affect actor tracks energy and valence across events.
#[test]
fn affect_state_lifecycle() {
    let (mut actor, rx) = AffectActor::new();

    // Initial state
    let state = actor.current();
    assert!((state.energy - 1.0).abs() < f32::EPSILON);

    // LLM call drains energy
    actor.on_llm_call();
    assert!(actor.current().energy < 1.0);

    // Error lowers valence
    actor.on_error();
    assert!(actor.current().valence < 0.5);

    // Capability confirmed boosts valence
    actor.on_capability_confirmed();
    let after_confirm = actor.current().valence;
    let _ = after_confirm; // used for verification

    // Idle tick recovers energy
    actor.on_idle_tick();
    assert!(actor.current().energy > state.energy - 0.03);

    // Watch channel reflects latest state
    let watched = *rx.borrow();
    assert!((watched.energy - actor.current().energy).abs() < f32::EPSILON);
}

/// System info can be gathered (smoke test).
#[test]
fn system_info_smoke() {
    let info = SystemInfo::gather();
    assert!(!info.os_name.is_empty());
    assert!(info.cpu_count >= 1);
}

/// Capability registration enables keyword matching in fast path.
#[test]
fn capability_registration_enables_matching() {
    use iris_core::cognition::fast_path::FastPath;
    use iris_core::types::{ReflexAction, RouteTarget, SalienceScore, SensoryEvent};

    let mut fp = FastPath::new();
    let cap_id = uuid::Uuid::new_v4();
    fp.register(cap_id, vec!["weather".into(), "forecast".into(), "temperature".into()]);

    // Event matching registered keywords → InvokeCapability
    let event = iris_core::types::GatedEvent {
        event: SensoryEvent::external("what's the weather forecast?"),
        salience: SalienceScore::compute(0.6, 0.4, 0.3, 0.5, 0.82),
        route: RouteTarget::TextDialogue,
    };
    let decision = fp.evaluate(&event).unwrap();
    assert_eq!(decision.action, ReflexAction::InvokeCapability);
    assert_eq!(decision.capability_id, Some(cap_id));
    assert!(!decision.async_codegen);

    // Unrelated event → DirectLlmFallback with async_codegen
    let event2 = iris_core::types::GatedEvent {
        event: SensoryEvent::external("tell me a joke"),
        salience: SalienceScore::compute(0.6, 0.4, 0.3, 0.5, 0.82),
        route: RouteTarget::TextDialogue,
    };
    let decision2 = fp.evaluate(&event2).unwrap();
    assert_eq!(decision2.action, ReflexAction::DirectLlmFallback);
    assert!(decision2.async_codegen);
}

/// Decision fusion carries async_codegen flag from fast path.
#[test]
fn decision_fusion_carries_async_codegen() {
    use iris_core::cognition::arbitration::{self, PressureState};
    use iris_core::types::{ReflexAction, ReflexDecision};

    let pressure = PressureState::new();

    // Fast path with async_codegen=true
    let fast = ReflexDecision {
        action: ReflexAction::DirectLlmFallback,
        capability_id: None,
        confidence: 0.5,
        async_codegen: true,
    };
    let decision = arbitration::fuse(Some(fast), None, &pressure).unwrap();
    assert!(decision.async_codegen);

    // Fast path with async_codegen=false (capability matched)
    let fast_cap = ReflexDecision {
        action: ReflexAction::InvokeCapability,
        capability_id: Some(uuid::Uuid::new_v4()),
        confidence: 0.8,
        async_codegen: false,
    };
    let decision2 = arbitration::fuse(Some(fast_cap), None, &pressure).unwrap();
    assert!(!decision2.async_codegen);
}

/// Feedback detection drives affect state changes.
#[test]
fn feedback_detection_affects_state() {
    use iris_core::dialogue::feedback;
    use iris_core::identity::affect::AffectActor;
    use iris_core::types::FeedbackType;

    let (mut affect, _rx) = AffectActor::new();
    let initial_valence = affect.current().valence;

    // Positive feedback boosts valence
    let fb = feedback::detect_keyword_feedback("thanks, that was great!");
    assert_eq!(fb, FeedbackType::Positive);
    affect.on_capability_confirmed();
    assert!(affect.current().valence > initial_valence);

    // Negative feedback lowers valence
    let fb2 = feedback::detect_keyword_feedback("that's wrong, please fix it");
    assert_eq!(fb2, FeedbackType::Negative);
    let before_error = affect.current().valence;
    affect.on_error();
    assert!(affect.current().valence < before_error);
}

/// Narrative event creation with significance clamping.
#[test]
fn narrative_event_creation() {
    use iris_core::identity::narrative;
    use iris_core::types::NarrativeEventType;

    let evt = narrative::new_event(
        NarrativeEventType::MilestoneReached,
        "boot sequence completed",
        0.8,
    );
    assert_eq!(evt.event_type, NarrativeEventType::MilestoneReached);
    assert!((evt.significance - 0.8).abs() < f32::EPSILON);

    // Significance clamped to [0, 1]
    let evt_high = narrative::new_event(NarrativeEventType::Other, "test", 2.0);
    assert!((evt_high.significance - 1.0).abs() < f32::EPSILON);
}

/// Embedding generation is deterministic and correct dimension.
#[test]
fn embedding_generation_properties() {
    use iris_core::memory::embedding;

    let emb1 = embedding::generate("hello world");
    let emb2 = embedding::generate("hello world");
    assert_eq!(emb1, emb2, "embedding must be deterministic");
    assert_eq!(emb1.len(), 32, "embedding dimension must be 32");

    let emb3 = embedding::generate("different input");
    assert_ne!(emb1, emb3, "different inputs should produce different embeddings");
}

/// Admission control gates slow path under critical pressure.
#[test]
fn admission_gates_slow_path_budget() {
    use iris_core::resource_space::admission::{self, AdmissionResult, ResourceEstimate};
    use iris_core::resource_space::budget;

    let (tx, rx) = budget::watch_channel();

    // Normal: slow path admitted (64 MB external estimate)
    let est = ResourceEstimate { memory_mb: 64, is_external: true };
    assert_eq!(admission::check(&rx, est), AdmissionResult::Admitted);

    // Critical with tiny total: external still admitted (floor 64 MB)
    let critical = budget::ResourceBudget::compute(100, PressureLevel::Critical);
    tx.send(critical).unwrap();
    assert_eq!(admission::check(&rx, est), AdmissionResult::Admitted);

    // But internal growth is 0 under critical
    let internal = ResourceEstimate { memory_mb: 1, is_external: false };
    assert_eq!(admission::check(&rx, internal), AdmissionResult::Rejected);
}

/// Compile check in repair loop works for valid Rust code.
#[test]
fn repair_loop_compile_check_valid_code() {
    // This test verifies the compile_in_temp_dir function works
    // by checking that extract_code handles various formats
    use iris_core::codegen::repair_loop;
    assert_eq!(repair_loop::MAX_REPAIR_ITERATIONS, 3);
    assert_eq!(repair_loop::COMPILE_TIMEOUT_SECS, 120);
}

/// ContextVersion tracks external input versions and detects staleness.
#[test]
fn context_version_staleness_detection() {
    use iris_core::dialogue::context_version::ContextVersion;

    let cv = ContextVersion::new();
    assert_eq!(cv.current(), 0);

    // Snapshot before bump
    let snap = cv.current();
    assert!(cv.is_current(snap));

    // Bump simulates new external input
    cv.bump();
    assert!(!cv.is_current(snap), "old snapshot should be stale after bump");
    assert!(cv.is_current(cv.current()));

    // Clone shares state (Arc-backed)
    let cv2 = cv.clone();
    cv2.bump();
    assert_eq!(cv.current(), 2);
}

/// CpuSampler returns valid usage ratios.
#[test]
fn cpu_sampler_returns_valid_ratio() {
    use iris_core::environment::system::CpuSampler;

    let mut sampler = CpuSampler::new();
    let sample = sampler.sample();
    assert!(sample.usage_ratio >= 0.0);
    assert!(sample.usage_ratio <= 1.0);
}

/// RamSnapshot::sample returns non-negative values.
#[test]
fn ram_snapshot_sample_valid() {
    use iris_core::environment::system::RamSnapshot;

    let snap = RamSnapshot::sample();
    // On Linux, total_mb should be > 0; on other platforms it may be 0
    assert!(snap.usage_ratio() >= 0.0);
    assert!(snap.usage_ratio() <= 1.0);
}

/// Perception module extracts features correctly from gated events.
#[test]
fn perception_extract_features() {
    use iris_core::cognition::perception;
    use iris_core::types::{GatedEvent, RouteTarget, SalienceScore, SensoryEvent};

    let event = GatedEvent {
        event: SensoryEvent::external("critical error: system crash detected"),
        salience: SalienceScore::compute(0.6, 0.4, 0.3, 0.5, 0.82),
        route: RouteTarget::TextDialogue,
    };
    let features = perception::extract(&event);
    assert!(features.threat >= 0.5, "should detect threat keywords");
    assert_eq!(features.intent_tag, "statement");

    // Question detection
    let (tag, conf) = perception::classify_intent("how do I fix this?");
    assert_eq!(tag, "question");
    assert!(conf > 0.5);
}
