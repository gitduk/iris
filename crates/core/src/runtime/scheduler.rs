use std::sync::Arc;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

use super::loop_control::{self, TickMode};
use super::rest_cycle::RestCycle;
use super::shutdown::ShutdownGuard;
use crate::boot::guardian::BootGuardian;
use crate::boot::safe_mode::SafeMode;
use crate::capability::builtin::BuiltinRegistry;
use crate::capability::process_manager::HealthEvent;
use crate::capability::{db as capability_db, lifecycle, process_manager::ProcessManager};
use crate::codegen::gap_generator;
use crate::cognition::arbitration::PressureState;
use crate::cognition::{response, tool_call};
use crate::config::IrisCfg;
use crate::dialogue::commit_window::CommitWindow;
use crate::dialogue::context_version::ContextVersion;
use crate::dialogue::feedback;
use crate::dialogue::interrupt::InterruptController;
use crate::dialogue::topic_tracking::TopicTracker;
use crate::environment::hardware::HardwareSnapshot;
use crate::environment::system::{CpuSampler, RamSnapshot};
use crate::environment::watcher::EnvironmentWatcher;
use crate::identity::affect::AffectActor;
use crate::identity::{core_identity, introspection, narrative, self_model};
use crate::io::output::{OutputMessage, OutputReceiver, OutputSender};
use crate::memory;
use crate::memory::working::WorkingMemory;
use crate::resource_space::budget::{self, BudgetSender, ResourceBudget};
use crate::resource_space::pressure::{self as res_pressure, ResourceSnapshot};
use crate::sensory::gating;
use crate::thalamus::router;
use crate::types::{
    ContextEntry, Episode, EventSource, FeedbackType, GapDescriptor, GapType, GatedEvent,
    NarrativeEventType, RuntimeStatus, SensoryEvent,
};
use iris_llm::provider::LlmProvider;

/// Core runtime that drives the iris tick loop.
pub struct Runtime {
    cfg: Arc<IrisCfg>,
    shutdown: ShutdownGuard,
    pool: Option<sqlx::PgPool>,
    /// Inbound event channel — external input, system events, spontaneous thoughts.
    event_rx: mpsc::Receiver<SensoryEvent>,
    /// Sender clone for re-injecting internal events (replay, spontaneous thoughts).
    event_tx: mpsc::Sender<SensoryEvent>,
    tick_count: u64,
    mode: TickMode,
    /// Pressure state machine for arbitration.
    pressure: PressureState,
    /// LLM provider for slow path + direct response (None if no LLM configured).
    llm: Option<Arc<dyn LlmProvider>>,
    /// Optional lightweight LLM used only to decide whether tool calls are needed.
    lite_llm: Option<Arc<dyn LlmProvider>>,
    /// In-process working memory.
    working_memory: WorkingMemory,
    /// Outbound response channel.
    output_tx: OutputSender,
    /// Affect state actor — drives energy, valence, arousal.
    affect: AffectActor,
    /// Conversation topic tracker.
    topics: TopicTracker,
    /// Boot guardian — tracks boot phases and failures.
    boot: BootGuardian,
    /// Safe mode — activated after consecutive boot failures.
    safe_mode: SafeMode,
    /// Commit window — batches rapid-fire dialogue inputs (reserved for v2).
    #[allow(dead_code)]
    commit_window: CommitWindow,
    /// Interrupt controller — cancels in-flight reasoning on new input.
    interrupt: InterruptController,
    /// Environment watcher — monitors CPU/battery and emits degradation signals.
    env_watcher: EnvironmentWatcher,
    /// CPU sampler — stateful, computes delta between ticks.
    cpu_sampler: CpuSampler,
    /// Resource budget sender — broadcasts recomputed budgets each tick.
    budget_tx: BudgetSender,
    /// Rest cycle — manages RestMode entry/exit.
    rest_cycle: RestCycle,
    /// Context version — increments on external input, detects stale reasoning.
    context_version: ContextVersion,
    /// Status watch channel — broadcasts runtime snapshot each tick for TUI.
    status_tx: tokio::sync::watch::Sender<RuntimeStatus>,
    /// Capability subprocess manager.
    process_manager: ProcessManager,
    /// Built-in capabilities (read_file, write_file, run_bash).
    builtin_registry: BuiltinRegistry,
}

impl Runtime {
    /// Create a new Runtime. Returns (Runtime, event_sender, output_receiver, status_receiver).
    /// Send `SensoryEvent`s into the returned sender to feed the tick loop.
    /// Consume `OutputMessage`s from the returned receiver to get iris responses.
    /// Watch `RuntimeStatus` from the returned receiver for TUI status bar.
    pub fn new(
        cfg: Arc<IrisCfg>,
        pool: Option<sqlx::PgPool>,
        llm: Option<Arc<dyn LlmProvider>>,
        lite_llm: Option<Arc<dyn LlmProvider>>,
    ) -> (
        Self,
        mpsc::Sender<SensoryEvent>,
        OutputReceiver,
        tokio::sync::watch::Receiver<RuntimeStatus>,
    ) {
        let shutdown = ShutdownGuard::new();
        let shutdown_token = shutdown.token();
        let working_memory_cap = cfg.working_memory_cap;
        let working_memory_ttl = cfg.working_memory_ttl_secs;
        let commit_window_ms = cfg.commit_window_ms;
        let max_active_topics = cfg.max_active_topics;
        let safe_mode_recovery = cfg.safe_mode_recovery_ticks;
        let safe_mode_cooldown = cfg.safe_mode_cooldown_secs;
        let (tx, rx) = mpsc::channel(256); // bounded, backpressure at 256
        let (output_tx, output_rx) = crate::io::output::channel(64);
        // affect_rx intentionally dropped — Runtime reads affect via affect.current() directly
        let (affect, _) = AffectActor::new();
        let (budget_tx, _budget_rx) = budget::watch_channel();
        let (status_tx, status_rx) = tokio::sync::watch::channel(RuntimeStatus::default());
        let runtime = Self {
            cfg,
            shutdown,
            pool,
            event_rx: rx,
            event_tx: tx.clone(),
            tick_count: 0,
            mode: TickMode::Idle,
            pressure: PressureState::new(),
            llm,
            lite_llm,
            working_memory: WorkingMemory::new(working_memory_cap, working_memory_ttl),
            output_tx,
            affect,
            topics: TopicTracker::with_max(max_active_topics),
            boot: BootGuardian::new(),
            safe_mode: SafeMode::with_params(safe_mode_recovery, safe_mode_cooldown),
            commit_window: CommitWindow::with_window_ms(commit_window_ms),
            interrupt: InterruptController::new(),
            env_watcher: EnvironmentWatcher::new(),
            cpu_sampler: CpuSampler::new(),
            budget_tx,
            rest_cycle: RestCycle::new(),
            context_version: ContextVersion::new(),
            status_tx,
            process_manager: ProcessManager::new(shutdown_token),
            builtin_registry: BuiltinRegistry::new(),
        };
        (runtime, tx, output_rx, status_rx)
    }

    /// Start the signal listener and enter the main tick loop.
    /// Returns when shutdown is complete.
    pub async fn run(&mut self) {
        self.shutdown.spawn_signal_listener();
        let token = self.shutdown.token();

        tracing::info!("iris runtime started");

        // Boot sequence: CoreInit → CapabilityLoad → EnvironmentSense → Ready
        self.boot.advance(); // → CapabilityLoad

        // Load confirmed capabilities from DB
        if let Some(pool) = &self.pool {
            match capability_db::fetch_by_state(pool, crate::types::CapabilityState::Confirmed)
                .await
            {
                Ok(caps) => {
                    tracing::info!(count = caps.len(), "capabilities loaded from DB");

                    // Spawn confirmed capability processes
                    for cap in &caps {
                        if let Err(e) = self.process_manager.spawn(cap) {
                            tracing::warn!(capability = %cap.name, error = %e, "failed to spawn confirmed capability");
                        }
                    }
                }
                Err(e) => {
                    self.boot.record_failure();
                    tracing::warn!(error = %e, "failed to load capabilities from DB");
                }
            }

            // Also spawn ActiveCandidate capabilities
            match capability_db::fetch_by_state(
                pool,
                crate::types::CapabilityState::ActiveCandidate,
            )
            .await
            {
                Ok(candidates) => {
                    for cap in &candidates {
                        if let Err(e) = self.process_manager.spawn(cap) {
                            tracing::warn!(capability = %cap.name, error = %e, "failed to spawn candidate capability");
                        }
                    }
                    if !candidates.is_empty() {
                        tracing::info!(count = candidates.len(), "active candidates spawned");
                    }
                }
                Err(e) => {
                    tracing::warn!(error = %e, "failed to load active candidates from DB");
                }
            }
        }

        self.boot.advance(); // → EnvironmentSense
        self.boot.advance(); // → Ready
        self.boot.record_success();
        tracing::info!(phase = %self.boot.current_phase(), "boot sequence complete");

        // Ensure core identity exists (DB required)
        if let Some(pool) = &self.pool {
            match core_identity::ensure(pool, "iris").await {
                Ok(id) => tracing::info!(name = %id.name, "core identity ensured"),
                Err(e) => {
                    self.boot.record_failure();
                    tracing::warn!(error = %e, "failed to ensure core identity");
                }
            }

            // Seed architectural self-knowledge (idempotent)
            if let Err(e) = self_model::seed_architecture(pool).await {
                tracing::warn!(error = %e, "failed to seed self-model");
            } else {
                tracing::info!("self-model architecture seeded");
            }

            // Record boot narrative event
            let evt = narrative::new_event(
                NarrativeEventType::MilestoneReached,
                "boot sequence completed successfully",
                0.8,
            );
            if let Err(e) = narrative::record(pool, &evt).await {
                tracing::warn!(error = %e, "failed to record boot narrative");
            }
        }

        // Check if boot failures warrant safe mode
        if self.boot.should_enter_safe_mode() {
            self.safe_mode.enter();
            tracing::warn!("entered safe mode due to consecutive boot failures");
        }

        // Spawn consolidation background task if LLM and DB are available
        if let (Some(llm), Some(pool)) = (&self.llm, &self.pool) {
            memory::consolidation::spawn(
                pool.clone(),
                Arc::clone(llm),
                self.cfg.consolidation_interval_secs,
                self.shutdown.token(),
            );
            tracing::info!("memory consolidation task spawned");
        }

        // Spawn memory replay background task if DB is available
        if let Some(pool) = &self.pool {
            memory::replay::spawn(
                pool.clone(),
                self.event_tx.clone(),
                self.cfg.replay_salience,
                self.cfg.consolidation_interval_secs, // reuse consolidation interval
                self.shutdown.token(),
            );
            tracing::info!("memory replay task spawned");
        }

        loop {
            let interval = self.mode.interval(&self.cfg);

            tokio::select! {
                _ = token.cancelled() => {
                    tracing::info!(tick_count = self.tick_count, "shutdown signal received, exiting tick loop");
                    break;
                },
                _ = tokio::time::sleep(interval) => {
                    self.tick().await;
                },
            }
        }

        self.process_manager
            .shutdown_all(std::time::Duration::from_secs(
                self.cfg.shutdown_timeout_secs,
            ))
            .await;
        tracing::info!("iris runtime stopped");
    }

    /// Returns the cancellation token for spawning child tasks.
    pub fn token(&self) -> CancellationToken {
        self.shutdown.token()
    }

    /// Single tick: the 8-step cognitive cycle.
    async fn tick(&mut self) {
        self.tick_count += 1;
        let _span = tracing::info_span!("tick", n = self.tick_count, mode = ?self.mode).entered();

        // Step 1: Collect inputs — drain event channel
        let events = self.collect_inputs();

        // Commit window disabled in v1 — external events processed immediately.
        // Rapid-fire input merging deferred to v2 (see PLAN.md §11).

        // Step 2: Sensory gating — filter below noise_floor, score salience
        let gated = gating::gate(events, &self.cfg);

        // Step 3: Thalamic routing — sort into priority batches
        let batch = router::route(gated);

        if !batch.is_empty() {
            tracing::debug!(
                dialogue = batch.dialogue.len(),
                internal = batch.internal.len(),
                system = batch.system.len(),
                "routed events"
            );
        }

        let has_external_events = batch.has_external();

        // Step 4-6: Process dialogue events through fast/slow dual system
        let all_events: Vec<GatedEvent> =
            batch.dialogue.into_iter().chain(batch.internal).collect();

        // Interrupt: cancel previous reasoning if new external input arrives
        if has_external_events && self.interrupt.has_active_task() {
            self.interrupt.cancel_current();
            tracing::debug!("interrupted previous reasoning task");
        }

        // Bump context version on external input (used for stale reasoning detection)
        if has_external_events {
            let ver = self.context_version.bump();
            tracing::debug!(context_version = ver, "context version bumped");
        }

        for event in &all_events {
            self.process_event(event).await;
        }

        // Step 7: Learning update — write to working memory + track topics + detect feedback
        for event in &all_events {
            // Topic tracking: activate topic from content prefix (first 32 chars)
            let topic_id = if event.event.source == EventSource::External {
                let label: String = event.event.content.chars().take(32).collect();
                let tid = uuid::Uuid::new_v4();
                self.topics.activate(tid, label);
                Some(tid)
            } else {
                self.topics.current_topic()
            };

            // Feedback detection on external user input
            if event.event.source == EventSource::External {
                let fb = feedback::detect_keyword_feedback(&event.event.content);
                match fb {
                    FeedbackType::Positive => self.affect.on_capability_confirmed(),
                    FeedbackType::Negative => self.affect.on_error(),
                    FeedbackType::Neutral => {}
                }

                // Persist feedback preference to DB
                if fb != FeedbackType::Neutral
                    && let Some(pool) = &self.pool
                {
                    let request_type = self
                        .topics
                        .current_topic()
                        .map(|_| "dialogue")
                        .unwrap_or("unknown");
                    if let Err(e) = feedback::record_preference(pool, request_type, fb).await {
                        tracing::debug!(error = %e, "failed to persist feedback preference");
                    }
                }
            }

            let entry = ContextEntry {
                id: uuid::Uuid::new_v4(),
                topic_id,
                content: event.event.content.clone(),
                salience_score: event.salience.score,
                created_at: event.event.timestamp,
                last_accessed: chrono::Utc::now(),
                pinned_by: None,
                is_response: false,
            };
            self.working_memory.insert(entry);
        }

        // Step 8: Memory write — persist to episodes table (skip if no DB)
        if let Some(ref pool) = self.pool {
            for event in &all_events {
                let topic_id = self.topics.current_topic();
                let episode = Episode {
                    id: uuid::Uuid::new_v4(),
                    topic_id,
                    content: event.event.content.clone(),
                    embedding: Some(memory::embedding::generate(&event.event.content)),
                    salience: event.salience.score,
                    is_consolidated: false,
                    created_at: event.event.timestamp,
                };
                if let Err(e) = memory::episodic::write(pool, &episode).await {
                    tracing::warn!(error = %e, "failed to persist episode");
                }
            }
        }

        // Affect: arousal decay + idle recovery (when no events processed)
        self.affect.tick_decay();
        if all_events.is_empty() {
            self.affect.on_idle_tick();
        }

        // Safe mode tracking: record healthy/unhealthy ticks
        if self.safe_mode.is_active() {
            // In safe mode: skip slow path, only fast path
            if all_events.is_empty() {
                if self.safe_mode.record_healthy_tick() {
                    tracing::info!("safe mode exited after recovery");
                }
            } else {
                self.safe_mode.record_unhealthy_tick();
            }
        }

        // Environment monitoring: sample CPU and hardware each tick
        let cpu = self.cpu_sampler.sample();
        let hw = HardwareSnapshot::default();
        let signals = self.env_watcher.update(cpu, hw);
        for signal in &signals {
            tracing::info!(?signal, "environment degradation signal");
            self.affect.on_critical_event();
        }

        // Resource pressure evaluation — feeds into arbitration PressureState
        let ram = RamSnapshot::sample();
        let snap = ResourceSnapshot {
            ram_usage_ratio: ram.usage_ratio(),
            storage_usage_ratio: 0.0, // storage monitoring deferred to v2
        };
        let pressure_level = res_pressure::evaluate(&snap);
        self.pressure.update(pressure_level);

        // Recompute resource budget from pressure level
        let total_mb = if ram.total_mb > 0 { ram.total_mb } else { 512 };
        let new_budget = ResourceBudget::compute(total_mb, pressure_level);
        // watch::Sender::send only fails if all receivers dropped — benign
        let _ = self.budget_tx.send(new_budget);

        // Update tick mode for next iteration
        let has_pending_tasks = !self.event_rx.is_empty();
        let energy = self.affect.current().energy;
        self.mode = loop_control::next_mode(has_external_events, has_pending_tasks, energy);

        // Rest cycle management
        if self.mode == TickMode::Rest {
            self.rest_cycle.enter();
            self.rest_cycle.tick();
        }
        if self.rest_cycle.is_active() && self.rest_cycle.should_wake(energy, has_external_events) {
            self.rest_cycle.exit();
        }

        // Capability health check — detect crashes and confirm candidates
        let health_events = self.process_manager.health_check();
        for event in health_events {
            match event {
                HealthEvent::Crashed { cap_id, exit_code } => {
                    self.handle_capability_crash(cap_id, exit_code).await;
                }
                HealthEvent::ReadyToConfirm { cap_id } => {
                    self.maybe_confirm_candidate(cap_id).await;
                }
            }
        }

        // Broadcast runtime status snapshot for TUI
        let mode_str = match self.mode {
            TickMode::Normal => "Normal",
            TickMode::Idle => "Idle",
            TickMode::Rest => "Rest",
        };
        let _ = self.status_tx.send(RuntimeStatus {
            tick_count: self.tick_count,
            mode: mode_str,
            affect: self.affect.current(),
            pressure: pressure_level,
            is_fast_only: self.pressure.is_fast_only(),
            safe_mode_active: self.safe_mode.is_active(),
            topic_count: self.topics.active_count(),
            context_version: self.context_version.current(),
            rest_active: self.rest_cycle.is_active(),
        });
    }

    /// Process a single event through the fast/slow cognitive pipeline.
    async fn process_event(&mut self, event: &GatedEvent) {
        // Build self-context once for both slow path and direct LLM fallback.
        // Builtin capability descriptions are no longer injected here — tools are
        // now sent structurally via the API `tools` parameter in the agentic loop.
        let self_context = if let Some(pool) = &self.pool {
            introspection::build_self_context(pool, &self.affect.current(), "").await
        } else {
            String::new()
        };

        // FastPath removed: all external/internal events now flow through the same
        // LLM + tool-routing path for consistent behavior.
        self.execute_direct_llm_fallback(event, &self_context).await;
    }

    /// Execute capability invocation: DB lookup, state validation, spawn if needed, IPC invoke.
    #[allow(dead_code)]
    async fn execute_capability_invocation(
        &mut self,
        event: &GatedEvent,
        cap_uuid: uuid::Uuid,
        self_context: &str,
    ) {
        // Built-in capability: execute in-process, then LLM-summarize the result
        if let Some(builtin) = self.builtin_registry.get(cap_uuid) {
            let builtin_name = builtin.name().to_string();
            let request = crate::types::CapabilityRequest {
                id: uuid::Uuid::new_v4(),
                method: event.event.content.clone(),
                params: serde_json::json!({"raw_input": event.event.content}),
                version: 1,
            };
            let resp = builtin.execute(request).await;
            let (tool_output, is_error) = if let Some(err) = &resp.error {
                (err.clone(), true)
            } else if let Some(result) = &resp.result {
                (result.to_string(), false)
            } else {
                ("ok".to_string(), false)
            };
            if is_error {
                self.affect.on_error();
            }
            self.execute_builtin_with_llm_summary(
                event,
                &builtin_name,
                &tool_output,
                is_error,
                self_context,
            )
            .await;
            return;
        }

        if let Some(pool) = &self.pool {
            match capability_db::fetch_by_id(pool, cap_uuid).await {
                Ok(Some(record)) => {
                    if let Err(e) = lifecycle::validate_transition(
                        record.state,
                        crate::types::CapabilityState::Confirmed,
                    ) {
                        if record.state == crate::types::CapabilityState::Quarantined {
                            if lifecycle::should_retire(record.quarantine_count) {
                                tracing::warn!(
                                    capability = %record.name,
                                    quarantine_count = record.quarantine_count,
                                    "capability should be retired"
                                );
                            }
                            self.send_response(&format!(
                                "[capability {}] quarantined, cannot invoke",
                                record.name
                            ));
                        } else {
                            tracing::debug!(error = %e, capability = %record.name, "invalid capability state for invocation");
                            self.send_response(&format!(
                                "[capability {}] state {:?} not invocable",
                                record.name, record.state
                            ));
                        }
                        return;
                    }

                    if record.state != crate::types::CapabilityState::Confirmed
                        && record.state != crate::types::CapabilityState::ActiveCandidate
                    {
                        return;
                    }

                    // Ensure process is running
                    if !self.process_manager.is_running(cap_uuid)
                        && let Err(e) = self.process_manager.spawn(&record)
                    {
                        tracing::warn!(capability = %record.name, error = %e, "failed to spawn capability for invocation");
                        self.send_response(&format!(
                            "[capability {}] spawn failed: {e}",
                            record.name
                        ));
                        if let Err(db_err) =
                            capability_db::record_outcome(pool, cap_uuid, false).await
                        {
                            tracing::warn!(error = %db_err, "failed to record capability outcome");
                        }
                        return;
                    }

                    // Build IPC request
                    let request = crate::types::CapabilityRequest {
                        id: uuid::Uuid::new_v4(),
                        method: event.event.content.clone(),
                        params: serde_json::json!({}),
                        version: 1,
                    };

                    let timeout = std::time::Duration::from_millis(
                        record
                            .manifest
                            .resource_limits
                            .get("timeout_ms")
                            .and_then(|v| v.as_u64())
                            .unwrap_or(5000),
                    );

                    tracing::info!(capability = %record.name, state = ?record.state, "invoking capability via IPC");

                    match self
                        .process_manager
                        .invoke(cap_uuid, request, timeout)
                        .await
                    {
                        Ok(resp) => {
                            let response = if let Some(err) = &resp.error {
                                format!("[capability {}] error: {err}", record.name)
                            } else if let Some(result) = &resp.result {
                                format!("[capability {}] {result}", record.name)
                            } else {
                                format!("[capability {}] ok (no result)", record.name)
                            };
                            self.send_response(&response);
                            if let Err(e) =
                                capability_db::record_outcome(pool, cap_uuid, resp.error.is_none())
                                    .await
                            {
                                tracing::warn!(error = %e, "failed to record capability outcome");
                            }
                            self.store_response(event, response).await;
                        }
                        Err(e) => {
                            tracing::warn!(capability = %record.name, error = %e, "capability invocation failed");
                            self.send_response(&format!(
                                "[capability {}] invoke error: {e}",
                                record.name
                            ));
                            if let Err(db_err) =
                                capability_db::record_outcome(pool, cap_uuid, false).await
                            {
                                tracing::warn!(error = %db_err, "failed to record capability outcome");
                            }
                        }
                    }
                }
                Ok(None) => {
                    tracing::warn!(capability_id = %cap_uuid, "capability not found in DB");
                    self.send_response(&format!("[capability {cap_uuid}] not found"));
                }
                Err(e) => {
                    tracing::warn!(error = %e, "failed to fetch capability");
                    self.send_response(&format!("[capability error] {e}"));
                }
            }
        } else {
            self.send_response(&format!("[capability {cap_uuid}] no DB configured"));
        }
    }

    /// Submit async codegen for an unmatched capability gap.
    #[allow(dead_code)]
    fn submit_codegen_gap(&self, event: &GatedEvent) {
        if let (Some(pool), Some(llm)) = (&self.pool, &self.llm) {
            let gap = GapDescriptor {
                id: uuid::Uuid::new_v4(),
                gap_type: GapType::parse(&event.event.content),
                trigger_description: event.event.content.clone(),
                source: event.event.source,
                suggested_crates: Vec::new(),
                created_at: chrono::Utc::now(),
            };
            // Receiver intentionally dropped — codegen runs fire-and-forget in background
            let _rx = gap_generator::submit_async(
                gap,
                pool.clone(),
                Arc::clone(llm),
                self.shutdown.token(),
            );
            tracing::info!("async codegen submitted for capability gap");
        }
    }

    /// Execute DirectLlmFallback: generate response via LLM or placeholder.
    /// When builtin tools are available, uses the agentic tool-use loop.
    async fn execute_direct_llm_fallback(&mut self, event: &GatedEvent, self_context: &str) {
        if let Some(ref llm) = self.llm {
            self.affect.on_llm_call();
            let working = self.working_memory.recent(10);

            // Episodic recall: when working memory is thin, pull recent episodes from DB
            let mut episodic_entries = Vec::new();
            if working.len() < self.cfg.episodic_recall_threshold
                && let Some(pool) = &self.pool
            {
                match memory::episodic::search_recent(pool, 10).await {
                    Ok(episodes) => {
                        for ep in episodes {
                            // Skip episodes already present in working memory
                            if working.iter().any(|w| w.content == ep.content) {
                                continue;
                            }
                            episodic_entries.push(ContextEntry {
                                id: ep.id,
                                topic_id: ep.topic_id,
                                content: format!("[recall] {}", ep.content),
                                salience_score: ep.salience,
                                created_at: ep.created_at,
                                last_accessed: chrono::Utc::now(),
                                pinned_by: None,
                                is_response: false,
                            });
                        }
                    }
                    Err(e) => {
                        tracing::debug!(error = %e, "episodic recall failed");
                    }
                }
            }

            // Augment context with semantic memory (consolidated knowledge)
            let mut knowledge_entries = Vec::new();
            if let Some(pool) = &self.pool {
                match memory::semantic::recent_or_search(pool, &event.event.content, 3).await {
                    Ok(knowledge) => {
                        for k in knowledge {
                            knowledge_entries.push(ContextEntry {
                                id: uuid::Uuid::new_v4(),
                                topic_id: None,
                                content: format!("[knowledge] {}", k.summary),
                                salience_score: 0.7,
                                created_at: k.created_at,
                                last_accessed: chrono::Utc::now(),
                                pinned_by: None,
                                is_response: false,
                            });
                        }
                    }
                    Err(e) => {
                        tracing::debug!(error = %e, "semantic search failed, using working memory only");
                    }
                }
            }

            // Context order: episodic recall → working memory → semantic knowledge
            let mut context: Vec<&ContextEntry> = episodic_entries.iter().collect();
            context.extend(working);
            context.extend(knowledge_entries.iter());

            // Decide whether to execute a specific tool directly, run the full agentic loop,
            // or skip tools and generate a plain response.
            let tools = self.builtin_registry.tool_definitions();
            const TOOL_ROUTE_CONFIDENCE_THRESHOLD: f32 = 0.72;
            const TOOL_SKIP_CONFIDENCE_THRESHOLD: f32 = 0.90;

            enum ToolPlan {
                DirectResponse,
                RoutedTool {
                    name: String,
                    input: serde_json::Value,
                },
                AgenticLoop,
            }

            let plan = if tools.is_empty() {
                ToolPlan::DirectResponse
            } else {
                // Prefer lightweight router when configured, otherwise fall back to main model.
                let (router_llm, router_source) = if let Some(lite_llm) = &self.lite_llm {
                    (lite_llm.as_ref(), "lite")
                } else {
                    (llm.as_ref(), "main")
                };

                tracing::debug!(router_source, "tool routing provider selected");

                match tool_call::route_tool_call(router_llm, &event.event.content, &tools).await {
                    Ok(decision) => {
                        tracing::debug!(
                            use_tool = decision.use_tool,
                            confidence = decision.confidence,
                            is_valid = decision.is_valid,
                            tool_name = ?decision.tool_name,
                            router_source,
                            "tool route decision"
                        );

                        if !decision.use_tool {
                            if decision.confidence >= TOOL_SKIP_CONFIDENCE_THRESHOLD {
                                ToolPlan::DirectResponse
                            } else {
                                // Low-confidence "no tool": let main model decide via agentic loop.
                                ToolPlan::AgenticLoop
                            }
                        } else if decision.is_valid
                            && decision.confidence >= TOOL_ROUTE_CONFIDENCE_THRESHOLD
                        {
                            if let Some(name) = decision.tool_name {
                                ToolPlan::RoutedTool {
                                    name,
                                    input: decision.input,
                                }
                            } else {
                                ToolPlan::AgenticLoop
                            }
                        } else {
                            // Invalid or low-confidence route -> let main model decide via agentic loop.
                            ToolPlan::AgenticLoop
                        }
                    }
                    Err(e) => {
                        tracing::warn!(error = %e, router_source, "tool routing failed, falling back to agentic loop");
                        ToolPlan::AgenticLoop
                    }
                }
            };

            match plan {
                ToolPlan::RoutedTool { name, input } => {
                    match tool_call::execute_named_tool(&self.builtin_registry, &name, &input).await
                    {
                        Ok(result) => {
                            self.execute_builtin_with_llm_summary(
                                event,
                                &name,
                                &result,
                                false,
                                self_context,
                            )
                            .await;
                        }
                        Err(err) => {
                            self.affect.on_error();
                            self.execute_builtin_with_llm_summary(
                                event,
                                &name,
                                &err,
                                true,
                                self_context,
                            )
                            .await;
                        }
                    }
                }
                ToolPlan::AgenticLoop => {
                    let messages = response::build_messages(event, &context, self_context);
                    match tool_call::run_agentic_loop(
                        llm.as_ref(),
                        messages,
                        tools,
                        &self.builtin_registry,
                    )
                    .await
                    {
                        Ok(response) => {
                            tracing::info!(
                                response_len = response.len(),
                                "agentic loop response generated"
                            );
                            self.send_response(&response);
                            self.store_response(event, response).await;
                        }
                        Err(e) => {
                            self.affect.on_error();
                            tracing::warn!(error = %e, "agentic loop failed");
                            self.send_response(&format!("[LLM error] {e}"));
                        }
                    }
                }
                ToolPlan::DirectResponse => {
                    match response::generate(event, llm.as_ref(), &context, self_context)
                        .await
                    {
                        Ok(response) => {
                            tracing::info!(
                                response_len = response.len(),
                                "direct response generated (tool route: no tools)"
                            );
                            self.send_response(&response);
                            self.store_response(event, response).await;
                        }
                        Err(e) => {
                            self.affect.on_error();
                            tracing::warn!(error = %e, "direct response failed");
                            self.send_response(&format!("[LLM error] {e}"));
                        }
                    }
                }
            }
        } else {
            let placeholder = format!("[no LLM configured] received: {}", event.event.content);
            self.send_response(&placeholder);
            self.store_response(event, placeholder).await;
        }
    }

    /// Execute a built-in capability, then feed the result to LLM for a natural language summary.
    /// Falls back to a redacted, user-friendly text if no LLM is configured.
    #[allow(dead_code)]
    async fn execute_builtin_with_llm_summary(
        &mut self,
        event: &GatedEvent,
        tool_name: &str,
        tool_output: &str,
        is_error: bool,
        self_context: &str,
    ) {
        let tool_observation = Self::tool_observation_for_context(tool_name, tool_output, is_error);
        let fallback = Self::tool_fallback_message(tool_name, tool_output, is_error);

        // Never let model paraphrasing override concrete tool failures.
        // Return deterministic error text to avoid false success claims.
        if is_error {
            self.affect.on_error();
            self.send_response(&fallback);
            self.store_response(event, fallback).await;
            return;
        }

        // For shell execution, prefer deterministic fact-based reply to avoid
        // model-side contradiction (e.g., command succeeded but reply says "can't do that").
        if tool_name == "run_bash" {
            self.send_response(&fallback);
            self.store_response(event, fallback).await;
            return;
        }

        if let Some(ref llm) = self.llm {
            self.affect.on_llm_call();

            // Inject normalized tool observation as an assistant context entry so the LLM can
            // summarize naturally without leaking raw protocol payloads.
            let tool_entry = ContextEntry {
                id: uuid::Uuid::new_v4(),
                topic_id: self.topics.current_topic(),
                content: tool_observation,
                salience_score: 0.9,
                created_at: chrono::Utc::now(),
                last_accessed: chrono::Utc::now(),
                pinned_by: None,
                is_response: true,
            };

            // Build context from working memory + normalized tool observation for LLM
            let working = self.working_memory.recent(10);
            let mut context: Vec<&ContextEntry> = working.to_vec();
            context.push(&tool_entry);

            let llm_result =
                response::generate(event, llm.as_ref(), &context, self_context).await;

            // Persist normalized observation to working memory so it survives even if LLM summary is poor
            // (must happen after generate() to avoid borrow conflict on working_memory)
            self.working_memory.insert(tool_entry);

            match llm_result {
                Ok(response) => {
                    tracing::info!(
                        response_len = response.len(),
                        "builtin LLM summary generated"
                    );
                    self.send_response(&response);
                    self.store_response(event, response).await;
                }
                Err(e) => {
                    self.affect.on_error();
                    tracing::warn!(error = %e, tool_name, "builtin LLM summary failed, returning fallback message");
                    self.send_response(&fallback);
                    self.store_response(event, fallback).await;
                }
            }
        } else {
            self.send_response(&fallback);
            self.store_response(event, fallback).await;
        }
    }

    fn tool_observation_for_context(tool_name: &str, tool_output: &str, is_error: bool) -> String {
        if tool_name == "run_bash" {
            if is_error {
                return format!("run_bash failed: {}", Self::short_text(tool_output, 240));
            }

            if let Ok(v) = serde_json::from_str::<serde_json::Value>(tool_output) {
                let code = v.get("exit_code").and_then(|x| x.as_i64()).unwrap_or(0);
                let stdout = v
                    .get("stdout")
                    .and_then(|x| x.as_str())
                    .unwrap_or("")
                    .trim();
                let stderr = v
                    .get("stderr")
                    .and_then(|x| x.as_str())
                    .unwrap_or("")
                    .trim();

                let out = if stdout.is_empty() { "(empty)" } else { stdout };
                let err = if stderr.is_empty() { "(empty)" } else { stderr };
                return format!(
                    "run_bash finished with exit_code={code}. stdout: {} ; stderr: {}",
                    Self::short_text(out, 500),
                    Self::short_text(err, 500)
                );
            }
        }

        if is_error {
            format!("{tool_name} failed: {}", Self::short_text(tool_output, 240))
        } else {
            format!("{tool_name} result: {}", Self::short_text(tool_output, 600))
        }
    }

    fn tool_fallback_message(tool_name: &str, tool_output: &str, is_error: bool) -> String {
        if tool_name == "run_bash" {
            if is_error {
                return format!("执行命令时失败：{}", Self::short_text(tool_output, 180));
            }

            if let Ok(v) = serde_json::from_str::<serde_json::Value>(tool_output) {
                let code = v.get("exit_code").and_then(|x| x.as_i64()).unwrap_or(0);
                let stdout = v
                    .get("stdout")
                    .and_then(|x| x.as_str())
                    .unwrap_or("")
                    .trim();
                let stderr = v
                    .get("stderr")
                    .and_then(|x| x.as_str())
                    .unwrap_or("")
                    .trim();

                if code == 0 {
                    if stdout.is_empty() && stderr.is_empty() {
                        return "命令已执行完成，没有输出。".to_string();
                    }
                    if !stdout.is_empty() {
                        return format!("命令已执行完成。输出：{}", Self::short_text(stdout, 280));
                    }
                    return format!("命令已执行完成。提示：{}", Self::short_text(stderr, 280));
                }

                let brief = if !stderr.is_empty() { stderr } else { stdout };
                return format!(
                    "执行命令失败（exit code {code}）：{}",
                    Self::short_text(brief, 240)
                );
            }
        }

        if is_error {
            format!(
                "执行 {tool_name} 时失败：{}",
                Self::short_text(tool_output, 180)
            )
        } else {
            format!("{tool_name} 已执行完成。")
        }
    }

    fn short_text(input: &str, max_chars: usize) -> String {
        let trimmed = input.trim();
        let mut out: String = trimmed.chars().take(max_chars).collect();
        if trimmed.chars().count() > max_chars {
            out.push_str("...");
        }
        out
    }

    /// Send a response to the output channel, logging if full.
    fn send_response(&self, content: &str) {
        if self
            .output_tx
            .try_send(OutputMessage::complete(content.to_owned()))
            .is_err()
        {
            tracing::warn!("output channel full, response dropped");
        }
    }

    /// Store an iris response in working memory and episodes table.
    async fn store_response(&mut self, event: &GatedEvent, content: String) {
        let now = chrono::Utc::now();
        let topic_id = self.topics.current_topic();

        // Persist response to episodes for cross-session recall
        if let Some(pool) = &self.pool {
            let episode = Episode {
                id: uuid::Uuid::new_v4(),
                topic_id,
                content: content.clone(),
                embedding: Some(memory::embedding::generate(&content)),
                salience: event.salience.score,
                is_consolidated: false,
                created_at: now,
            };
            if let Err(e) = memory::episodic::write(pool, &episode).await {
                tracing::warn!(error = %e, "failed to persist response episode");
            }
        }

        self.working_memory.insert(ContextEntry {
            id: uuid::Uuid::new_v4(),
            topic_id,
            content,
            salience_score: event.salience.score,
            created_at: now,
            last_accessed: now,
            pinned_by: None,
            is_response: true,
        });
    }

    /// Drain all pending events from the channel.
    fn collect_inputs(&mut self) -> Vec<SensoryEvent> {
        let mut events = Vec::new();
        while let Ok(event) = self.event_rx.try_recv() {
            events.push(event);
        }
        events
    }

    /// Handle a crashed capability: quarantine or retire, attempt LKG rollback.
    async fn handle_capability_crash(&mut self, cap_id: uuid::Uuid, exit_code: Option<i32>) {
        let Some(pool) = &self.pool else { return };

        tracing::warn!(capability_id = %cap_id, ?exit_code, "capability process crashed");

        let count = match capability_db::increment_quarantine(pool, cap_id).await {
            Ok(c) => c,
            Err(e) => {
                tracing::warn!(error = %e, "failed to increment quarantine count");
                return;
            }
        };

        if lifecycle::should_retire(count) {
            // Retire the capability
            if let Err(e) =
                capability_db::update_state(pool, cap_id, crate::types::CapabilityState::Retired)
                    .await
            {
                tracing::warn!(error = %e, "failed to retire capability");
            } else {
                tracing::info!(capability_id = %cap_id, "capability retired after repeated crashes");
            }

            // Narrative: capability lost
            let evt = narrative::new_event(
                NarrativeEventType::CapabilityLost,
                format!("capability {cap_id} retired after {count} quarantines"),
                0.7,
            );
            let _ = narrative::record(pool, &evt).await;
        } else {
            // Quarantine
            if let Err(e) = capability_db::update_state(
                pool,
                cap_id,
                crate::types::CapabilityState::Quarantined,
            )
            .await
            {
                tracing::warn!(error = %e, "failed to quarantine capability");
            }

            // Narrative: capability quarantined
            let evt = narrative::new_event(
                NarrativeEventType::CapabilityQuarantined,
                format!("capability {cap_id} quarantined (exit code: {exit_code:?})"),
                0.5,
            );
            let _ = narrative::record(pool, &evt).await;

            // Attempt LKG rollback
            match capability_db::fetch_by_id(pool, cap_id).await {
                Ok(Some(record)) if record.lkg_version.is_some() => {
                    // Fetch the LKG version and try to spawn it
                    let lkg_id = record.lkg_version.unwrap();
                    match capability_db::fetch_by_id(pool, lkg_id).await {
                        Ok(Some(lkg_record)) => {
                            if let Err(e) = self.process_manager.spawn(&lkg_record) {
                                tracing::warn!(error = %e, "failed to spawn LKG rollback");
                            } else {
                                tracing::info!(capability_id = %cap_id, lkg = %lkg_id, "rolled back to LKG version");
                            }
                        }
                        _ => {
                            tracing::debug!(lkg = %lkg_id, "LKG version not found in DB");
                        }
                    }
                }
                _ => {}
            }
        }
    }

    /// Confirm an ActiveCandidate if it has been running long enough.
    async fn maybe_confirm_candidate(&mut self, cap_id: uuid::Uuid) {
        let observe_dur = std::time::Duration::from_secs(self.cfg.candidate_observe_min_secs);
        if !self
            .process_manager
            .has_been_running_for(cap_id, observe_dur)
        {
            return;
        }

        let Some(pool) = &self.pool else { return };

        if let Err(e) =
            capability_db::update_state(pool, cap_id, crate::types::CapabilityState::Confirmed)
                .await
        {
            tracing::warn!(error = %e, "failed to confirm candidate");
            return;
        }

        // Set current version as LKG
        if let Err(e) = capability_db::update_lkg(pool, cap_id, cap_id).await {
            tracing::warn!(error = %e, "failed to update LKG after confirmation");
        }

        tracing::info!(capability_id = %cap_id, "active candidate confirmed after observation period");

        // Narrative: capability gained
        let evt = narrative::new_event(
            NarrativeEventType::CapabilityGained,
            format!("capability {cap_id} confirmed after observation period"),
            0.8,
        );
        let _ = narrative::record(pool, &evt).await;
    }
}
