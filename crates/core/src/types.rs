use serde::{Deserialize, Serialize};
use uuid::Uuid;
use chrono::{DateTime, Utc};

/// Origin of a sensory event.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum EventSource {
    /// User input or external system event.
    External,
    /// Internal thought, replay, or spontaneous signal.
    Internal,
}

/// Raw input event entering the cognitive pipeline.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SensoryEvent {
    pub id: Uuid,
    pub source: EventSource,
    pub content: String,
    pub timestamp: DateTime<Utc>,
}

impl SensoryEvent {
    pub fn external(content: impl Into<String>) -> Self {
        Self {
            id: Uuid::new_v4(),
            source: EventSource::External,
            content: content.into(),
            timestamp: Utc::now(),
        }
    }

    pub fn internal(content: impl Into<String>) -> Self {
        Self {
            id: Uuid::new_v4(),
            source: EventSource::Internal,
            content: content.into(),
            timestamp: Utc::now(),
        }
    }
}

/// Four-dimensional salience score.
/// Total = novelty×0.35 + urgency×0.25 + complexity×0.25 + task_relevance×0.15
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct SalienceScore {
    pub score: f32,
    pub novelty: f32,
    pub urgency: f32,
    pub complexity: f32,
    pub task_relevance: f32,
    /// True when urgency >= urgent_bypass threshold, skips embedding and enters fast path directly.
    pub is_urgent_bypass: bool,
}

impl SalienceScore {
    pub fn compute(novelty: f32, urgency: f32, complexity: f32, task_relevance: f32, urgent_bypass_threshold: f32) -> Self {
        let score = novelty * 0.35 + urgency * 0.25 + complexity * 0.25 + task_relevance * 0.15;
        Self {
            score,
            novelty,
            urgency,
            complexity,
            task_relevance,
            is_urgent_bypass: urgency >= urgent_bypass_threshold,
        }
    }
}

/// Perceptual features extracted from a sensory event.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PerceptFeature {
    pub threat: f32,
    pub complexity_raw: f32,
    pub intent_tag: String,
    pub intent_confidence: f32,
}

/// Route target after thalamic routing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RouteTarget {
    /// External user dialogue — high priority.
    TextDialogue,
    /// Internal signal (replay, spontaneous thought) — low priority.
    InternalSignal,
    /// System event (resource alert, capability state change) — direct dispatch.
    SystemEvent,
}

/// A sensory event enriched with salience and routing info.
#[derive(Debug, Clone)]
pub struct GatedEvent {
    pub event: SensoryEvent,
    pub salience: SalienceScore,
    pub route: RouteTarget,
}

// ── Decision types ──────────────────────────────────────────────

/// Fast path action type.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReflexAction {
    /// Matched a confirmed capability — invoke it.
    InvokeCapability,
    /// No capability match — LLM generates response directly.
    DirectLlmFallback,
}

/// Fast path decision (< 50ms target).
#[derive(Debug, Clone)]
pub struct ReflexDecision {
    pub action: ReflexAction,
    pub capability_id: Option<Uuid>,
    pub confidence: f32,
    /// If true, async codegen is triggered for the unmatched gap.
    pub async_codegen: bool,
}

/// Slow path decision (async, LLM-assisted reasoning).
#[derive(Debug, Clone)]
pub struct DeliberateDecision {
    pub plan: ActionPlan,
    pub confidence: f32,
}

/// Which cognitive path produced the decision.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DecisionSource {
    Fast,
    Slow,
}

/// Fused decision after arbitration.
#[derive(Debug, Clone)]
pub struct Decision {
    pub source: DecisionSource,
    pub plan: ActionPlan,
    pub confidence: f32,
    /// If true, async codegen should be triggered for the unmatched capability gap.
    pub async_codegen: bool,
}

/// A concrete action to execute.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ActionPlan {
    pub id: Uuid,
    pub capability_id: Option<Uuid>,
    pub method: String,
    pub params: serde_json::Value,
    pub timeout_ms: u64,
}

impl ActionPlan {
    /// Create a direct LLM response plan (no capability).
    pub fn direct_llm(method: impl Into<String>, params: serde_json::Value) -> Self {
        Self {
            id: Uuid::new_v4(),
            capability_id: None,
            method: method.into(),
            params,
            timeout_ms: 5000,
        }
    }
}

/// Resource pressure level — affects fast/slow arbitration weights.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PressureLevel {
    Normal,
    High,
    Critical,
}

// ── Memory types ───────────────────────────────────────────────

/// Working memory entry (in-process ring buffer).
#[derive(Debug, Clone)]
pub struct ContextEntry {
    pub id: Uuid,
    pub topic_id: Option<Uuid>,
    pub content: String,
    pub salience_score: f32,
    pub created_at: DateTime<Utc>,
    pub last_accessed: DateTime<Utc>,
    /// If Some, this entry is pinned and won't be evicted.
    pub pinned_by: Option<String>,
    /// True if this entry is an iris response (assistant), false if user input.
    pub is_response: bool,
}

impl ContextEntry {
    /// Eviction score: higher = more likely to evict.
    /// Formula: (now - last_accessed) / TTL - 0.3 * salience
    pub fn evict_score(&self, now: DateTime<Utc>, ttl_secs: f64) -> f64 {
        let age = (now - self.last_accessed).num_milliseconds() as f64 / 1000.0;
        age / ttl_secs - 0.3 * self.salience_score as f64
    }
}

/// Episodic memory row (persisted in `episodes` table).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Episode {
    pub id: Uuid,
    pub topic_id: Option<Uuid>,
    pub content: String,
    pub embedding: Option<Vec<u8>>,
    pub salience: f32,
    pub is_consolidated: bool,
    pub created_at: DateTime<Utc>,
}

/// Semantic memory row (persisted in `knowledge` table).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Knowledge {
    pub id: Uuid,
    pub summary: String,
    pub embedding: Option<Vec<u8>>,
    pub source_episode_ids: Vec<Uuid>,
    pub created_at: DateTime<Utc>,
}

// ── Capability types ────────────────────────────────────────────

/// Capability lifecycle state machine.
/// staged → active_candidate → confirmed → retired
/// Any state can transition to quarantined on failure.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum CapabilityState {
    Staged,
    ActiveCandidate,
    Confirmed,
    Quarantined,
    Retired,
}

impl CapabilityState {
    /// Parse from DB string representation.
    pub fn from_db(s: &str) -> Option<Self> {
        match s {
            "staged" => Some(Self::Staged),
            "active_candidate" => Some(Self::ActiveCandidate),
            "confirmed" => Some(Self::Confirmed),
            "quarantined" => Some(Self::Quarantined),
            "retired" => Some(Self::Retired),
            _ => None,
        }
    }

    /// Convert to DB string representation.
    pub fn as_db_str(&self) -> &'static str {
        match self {
            Self::Staged => "staged",
            Self::ActiveCandidate => "active_candidate",
            Self::Confirmed => "confirmed",
            Self::Quarantined => "quarantined",
            Self::Retired => "retired",
        }
    }
}

/// Permissions a capability can request.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Permission {
    FileRead,
    FileWrite,
    NetworkRead,
    NetworkWrite,
    ProcessSpawn,
    SystemInfo,
}

/// Capability manifest — metadata describing a capability.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CapabilityManifest {
    pub name: String,
    pub binary_path: String,
    pub permissions: Vec<Permission>,
    pub resource_limits: serde_json::Value,
    pub keywords: Vec<String>,
}

/// A capability record as stored in the DB.
#[derive(Debug, Clone)]
pub struct CapabilityRecord {
    pub id: Uuid,
    pub name: String,
    pub binary_path: String,
    pub manifest: CapabilityManifest,
    pub state: CapabilityState,
    pub lkg_version: Option<Uuid>,
    pub quarantine_count: i32,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// IPC request sent to a capability subprocess.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CapabilityRequest {
    pub id: Uuid,
    pub method: String,
    pub params: serde_json::Value,
    pub version: u8,
}

/// IPC response from a capability subprocess.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CapabilityResponse {
    pub id: Uuid,
    pub result: Option<serde_json::Value>,
    pub error: Option<String>,
    pub metrics: Option<serde_json::Value>,
    pub side_effects: Vec<Permission>,
}

/// Capability scoring (usage/success/fail tracking).
#[derive(Debug, Clone)]
pub struct CapabilityScore {
    pub capability_id: Uuid,
    pub usage_count: i64,
    pub success_count: i64,
    pub fail_count: i64,
    pub quarantine_count: i32,
}

// ── Codegen types ──────────────────────────────────────────────

/// Type of capability gap detected.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum GapType {
    FileSystem,
    Network,
    DataProcessing,
    SystemInfo,
    ExternalAPI,
    Compute,
    Unknown,
}

impl GapType {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::FileSystem => "file_system",
            Self::Network => "network",
            Self::DataProcessing => "data_processing",
            Self::SystemInfo => "system_info",
            Self::ExternalAPI => "external_api",
            Self::Compute => "compute",
            Self::Unknown => "unknown",
        }
    }

    pub fn parse(s: &str) -> Self {
        match s {
            "file_system" => Self::FileSystem,
            "network" => Self::Network,
            "data_processing" => Self::DataProcessing,
            "system_info" => Self::SystemInfo,
            "external_api" => Self::ExternalAPI,
            "compute" => Self::Compute,
            _ => Self::Unknown,
        }
    }
}

/// Describes a capability gap that needs code generation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GapDescriptor {
    pub id: Uuid,
    pub gap_type: GapType,
    pub trigger_description: String,
    pub source: EventSource,
    pub suggested_crates: Vec<String>,
    pub created_at: DateTime<Utc>,
}

/// Record of a code generation attempt.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CodegenHistory {
    pub id: Uuid,
    pub gap_type: String,
    pub approach_summary: Option<String>,
    pub success: bool,
    pub error_msg: Option<String>,
    pub is_consolidated: bool,
    pub created_at: DateTime<Utc>,
}

// ── Identity types ────────────────────────────────────────────

/// Core identity — immutable after creation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CoreIdentity {
    pub id: Uuid,
    pub name: String,
    pub born_at: DateTime<Utc>,
    pub founding_values: serde_json::Value,
}

/// Self-model key-value entry.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SelfModelEntry {
    pub key: String,
    pub value: serde_json::Value,
    pub updated_at: DateTime<Utc>,
}

/// Narrative event type.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum NarrativeEventType {
    CapabilityGained,
    CapabilityLost,
    CapabilityQuarantined,
    GoalAchieved,
    MilestoneReached,
    ErrorRecovery,
    Other,
}

impl NarrativeEventType {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::CapabilityGained => "capability_gained",
            Self::CapabilityLost => "capability_lost",
            Self::CapabilityQuarantined => "capability_quarantined",
            Self::GoalAchieved => "goal_achieved",
            Self::MilestoneReached => "milestone_reached",
            Self::ErrorRecovery => "error_recovery",
            Self::Other => "other",
        }
    }

    pub fn parse(s: &str) -> Self {
        match s {
            "capability_gained" => Self::CapabilityGained,
            "capability_lost" => Self::CapabilityLost,
            "capability_quarantined" => Self::CapabilityQuarantined,
            "goal_achieved" => Self::GoalAchieved,
            "milestone_reached" => Self::MilestoneReached,
            "error_recovery" => Self::ErrorRecovery,
            _ => Self::Other,
        }
    }
}

/// A narrative event — key life milestone.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NarrativeEvent {
    pub id: Uuid,
    pub occurred_at: DateTime<Utc>,
    pub event_type: NarrativeEventType,
    pub description: String,
    pub significance: f32,
}

/// Three-dimensional affect state (in-process, not persisted).
/// - energy: LLM call -0.03, idle +0.02; triggers RestMode when low
/// - valence: confirmed +0.10, error -0.15; sustained low affects risk weight
/// - arousal: Critical event +0.30, decay ×0.95
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct AffectState {
    pub energy: f32,
    pub valence: f32,
    pub arousal: f32,
}

impl Default for AffectState {
    fn default() -> Self {
        Self {
            energy: 1.0,
            valence: 0.5,
            arousal: 0.3,
        }
    }
}

impl AffectState {
    /// Clamp all dimensions to [0.0, 1.0].
    pub fn clamp(&mut self) {
        self.energy = self.energy.clamp(0.0, 1.0);
        self.valence = self.valence.clamp(0.0, 1.0);
        self.arousal = self.arousal.clamp(0.0, 1.0);
    }

    /// Apply per-tick arousal decay (×0.95).
    pub fn decay_arousal(&mut self) {
        self.arousal *= 0.95;
        self.clamp();
    }

    /// Whether energy is low enough to enter RestMode.
    pub fn should_rest(&self) -> bool {
        self.energy < 0.15
    }
}

// ── Dialogue / Feedback types ─────────────────────────────────

/// Feedback sentiment from user.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum FeedbackType {
    Positive,
    Negative,
    Neutral,
}

impl FeedbackType {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Positive => "positive",
            Self::Negative => "negative",
            Self::Neutral => "neutral",
        }
    }

    pub fn parse(s: &str) -> Self {
        match s {
            "positive" => Self::Positive,
            "negative" => Self::Negative,
            _ => Self::Neutral,
        }
    }
}

// ── Runtime status (for TUI status bar) ──────────────────────

/// Snapshot of runtime state, broadcast each tick via watch channel.
#[derive(Debug, Clone, Copy)]
pub struct RuntimeStatus {
    pub tick_count: u64,
    pub mode: &'static str,
    pub affect: AffectState,
    pub pressure: PressureLevel,
    pub is_fast_only: bool,
    pub safe_mode_active: bool,
    pub topic_count: usize,
    pub context_version: u64,
    pub rest_active: bool,
}

impl Default for RuntimeStatus {
    fn default() -> Self {
        Self {
            tick_count: 0,
            mode: "Idle",
            affect: AffectState::default(),
            pressure: PressureLevel::Normal,
            is_fast_only: false,
            safe_mode_active: false,
            topic_count: 0,
            context_version: 0,
            rest_active: false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn salience_score_weights_sum_to_one() {
        let s = SalienceScore::compute(1.0, 1.0, 1.0, 1.0, 0.82);
        assert!((s.score - 1.0).abs() < f32::EPSILON);
    }

    #[test]
    fn salience_urgent_bypass() {
        let s = SalienceScore::compute(0.5, 0.9, 0.3, 0.2, 0.82);
        assert!(s.is_urgent_bypass);

        let s = SalienceScore::compute(0.5, 0.5, 0.3, 0.2, 0.82);
        assert!(!s.is_urgent_bypass);
    }

    #[test]
    fn sensory_event_constructors() {
        let ext = SensoryEvent::external("hello");
        assert_eq!(ext.source, EventSource::External);

        let int = SensoryEvent::internal("thought");
        assert_eq!(int.source, EventSource::Internal);
    }

    #[test]
    fn context_entry_evict_score() {
        let now = Utc::now();
        let old = now - chrono::Duration::seconds(900); // 15 min ago
        let entry = ContextEntry {
            id: Uuid::new_v4(),
            topic_id: None,
            content: "test".into(),
            salience_score: 0.8,
            created_at: old,
            last_accessed: old,
            pinned_by: None,
            is_response: false,
        };
        let ttl = 1800.0; // 30 min
        let score = entry.evict_score(now, ttl);
        // age/ttl = 900/1800 = 0.5, salience penalty = 0.3*0.8 = 0.24
        // evict_score ≈ 0.5 - 0.24 = 0.26
        assert!((score - 0.26).abs() < 0.01);
    }

    #[test]
    fn pinned_entry_not_evictable() {
        let entry = ContextEntry {
            id: Uuid::new_v4(),
            topic_id: None,
            content: "pinned".into(),
            salience_score: 0.1,
            created_at: Utc::now(),
            last_accessed: Utc::now(),
            pinned_by: Some("system".into()),
            is_response: false,
        };
        assert!(entry.pinned_by.is_some());
    }

    #[test]
    fn capability_state_db_roundtrip() {
        let states = [
            (CapabilityState::Staged, "staged"),
            (CapabilityState::ActiveCandidate, "active_candidate"),
            (CapabilityState::Confirmed, "confirmed"),
            (CapabilityState::Quarantined, "quarantined"),
            (CapabilityState::Retired, "retired"),
        ];
        for (state, expected_str) in &states {
            assert_eq!(state.as_db_str(), *expected_str);
            assert_eq!(CapabilityState::from_db(expected_str), Some(*state));
        }
        assert_eq!(CapabilityState::from_db("unknown"), None);
    }

    #[test]
    fn action_plan_direct_llm() {
        let plan = ActionPlan::direct_llm("test_method", serde_json::json!({"key": "val"}));
        assert!(plan.capability_id.is_none());
        assert_eq!(plan.method, "test_method");
        assert_eq!(plan.timeout_ms, 5000);
    }

    #[test]
    fn gap_type_roundtrip() {
        let types = [
            GapType::FileSystem,
            GapType::Network,
            GapType::DataProcessing,
            GapType::SystemInfo,
            GapType::ExternalAPI,
            GapType::Compute,
            GapType::Unknown,
        ];
        for gt in &types {
            assert_eq!(GapType::parse(gt.as_str()), *gt);
        }
        assert_eq!(GapType::parse("nonsense"), GapType::Unknown);
    }

    #[test]
    fn narrative_event_type_roundtrip() {
        let types = [
            NarrativeEventType::CapabilityGained,
            NarrativeEventType::CapabilityLost,
            NarrativeEventType::CapabilityQuarantined,
            NarrativeEventType::GoalAchieved,
            NarrativeEventType::MilestoneReached,
            NarrativeEventType::ErrorRecovery,
            NarrativeEventType::Other,
        ];
        for nt in &types {
            assert_eq!(NarrativeEventType::parse(nt.as_str()), *nt);
        }
        assert_eq!(NarrativeEventType::parse("unknown"), NarrativeEventType::Other);
    }

    #[test]
    fn affect_state_defaults() {
        let a = AffectState::default();
        assert!((a.energy - 1.0).abs() < f32::EPSILON);
        assert!((a.valence - 0.5).abs() < f32::EPSILON);
        assert!((a.arousal - 0.3).abs() < f32::EPSILON);
    }

    #[test]
    fn affect_state_decay_and_clamp() {
        let mut a = AffectState { energy: 0.5, valence: 0.5, arousal: 1.0 };
        a.decay_arousal();
        assert!((a.arousal - 0.95).abs() < 0.001);

        a.energy = -0.5;
        a.valence = 1.5;
        a.clamp();
        assert!((a.energy).abs() < f32::EPSILON);
        assert!((a.valence - 1.0).abs() < f32::EPSILON);
    }

    #[test]
    fn affect_state_should_rest() {
        let low = AffectState { energy: 0.10, valence: 0.5, arousal: 0.3 };
        assert!(low.should_rest());

        let ok = AffectState { energy: 0.20, valence: 0.5, arousal: 0.3 };
        assert!(!ok.should_rest());
    }

    #[test]
    fn feedback_type_roundtrip() {
        assert_eq!(FeedbackType::parse("positive"), FeedbackType::Positive);
        assert_eq!(FeedbackType::parse("negative"), FeedbackType::Negative);
        assert_eq!(FeedbackType::parse("neutral"), FeedbackType::Neutral);
        assert_eq!(FeedbackType::parse("unknown"), FeedbackType::Neutral);
        assert_eq!(FeedbackType::Positive.as_str(), "positive");
    }
}
