use tokio::sync::watch;

use crate::types::AffectState;

/// Energy cost of an LLM call.
const LLM_CALL_ENERGY_COST: f32 = 0.03;
/// Energy recovery per idle tick.
const IDLE_ENERGY_RECOVERY: f32 = 0.02;
/// Valence boost on capability confirmed.
const CONFIRMED_VALENCE_BOOST: f32 = 0.10;
/// Valence penalty on error.
const ERROR_VALENCE_PENALTY: f32 = 0.15;
/// Arousal spike on critical event.
const CRITICAL_AROUSAL_SPIKE: f32 = 0.30;

/// Affect actor — owns the AffectState and exposes a watch channel.
#[derive(Debug)]
pub struct AffectActor {
    state: AffectState,
    tx: watch::Sender<AffectState>,
}

impl AffectActor {
    /// Create a new affect actor with default state.
    /// Returns the actor and a watch receiver for subscribers.
    pub fn new() -> (Self, watch::Receiver<AffectState>) {
        let state = AffectState::default();
        let (tx, rx) = watch::channel(state);
        (Self { state, tx }, rx)
    }

    fn broadcast(&self) {
        // watch::Sender::send only fails if all receivers are dropped — benign
        let _ = self.tx.send(self.state);
    }

    /// Apply an LLM call energy cost.
    pub fn on_llm_call(&mut self) {
        self.state.energy -= LLM_CALL_ENERGY_COST;
        self.state.clamp();
        self.broadcast();
    }

    /// Apply idle tick recovery.
    pub fn on_idle_tick(&mut self) {
        self.state.energy += IDLE_ENERGY_RECOVERY;
        self.state.clamp();
        self.broadcast();
    }

    /// Apply capability confirmed boost.
    pub fn on_capability_confirmed(&mut self) {
        self.state.valence += CONFIRMED_VALENCE_BOOST;
        self.state.clamp();
        self.broadcast();
    }

    /// Apply error penalty.
    pub fn on_error(&mut self) {
        self.state.valence -= ERROR_VALENCE_PENALTY;
        self.state.clamp();
        self.broadcast();
    }

    /// Apply critical event arousal spike.
    pub fn on_critical_event(&mut self) {
        self.state.arousal += CRITICAL_AROUSAL_SPIKE;
        self.state.clamp();
        self.broadcast();
    }

    /// Per-tick arousal decay.
    pub fn tick_decay(&mut self) {
        self.state.decay_arousal();
        self.broadcast();
    }

    /// Get current state snapshot.
    pub fn current(&self) -> AffectState {
        self.state
    }
}

/// Shared handle for reading affect state from any module.
pub type AffectWatch = watch::Receiver<AffectState>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn affect_actor_llm_call_drains_energy() {
        let (mut actor, rx) = AffectActor::new();
        assert!((actor.current().energy - 1.0).abs() < f32::EPSILON);

        actor.on_llm_call();
        let state = *rx.borrow();
        assert!((state.energy - 0.97).abs() < 0.001);
    }

    #[test]
    fn affect_actor_error_lowers_valence() {
        let (mut actor, rx) = AffectActor::new();
        actor.on_error();
        let state = *rx.borrow();
        assert!((state.valence - 0.35).abs() < 0.001);
    }

    #[test]
    fn affect_actor_critical_event_spikes_arousal() {
        let (mut actor, rx) = AffectActor::new();
        actor.on_critical_event();
        let state = *rx.borrow();
        assert!((state.arousal - 0.60).abs() < 0.001);
    }

    #[test]
    fn affect_actor_tick_decay() {
        let (mut actor, rx) = AffectActor::new();
        actor.on_critical_event(); // arousal = 0.60
        actor.tick_decay(); // arousal = 0.60 * 0.95 = 0.57
        let state = *rx.borrow();
        assert!((state.arousal - 0.57).abs() < 0.01);
    }

    #[test]
    fn affect_actor_rest_mode_trigger() {
        let (mut actor, _rx) = AffectActor::new();
        // Drain energy with many LLM calls
        for _ in 0..30 {
            actor.on_llm_call(); // 30 * 0.03 = 0.90 drained
        }
        assert!(actor.current().should_rest());
    }
}
