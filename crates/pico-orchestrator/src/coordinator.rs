use chrono::Utc;
use pico_core::config::PicoConfig;
use pico_core::error::{PicoError, Result};
use pico_core::events::PicoEvent;
use pico_core::session::{Message, SessionId, ToolCall};
use pico_core::traits::{DecisionModel, MemoryStore};
use pico_decision::{DecisionEngine, DecisionResult, ResponseSynthesizer, ToolEntry};
use pico_learning::{
    EvaluationMetrics, LabelQuality, LearningManager, ModelCheckpoint, ObservationRecord,
};
use pico_memory::MemoryManager;
use pico_security::{
    AuditLogger, ChannelConfirmationManager, ConfirmationProvider, PathSandbox, SecurityPolicy,
};
use pico_tools::{build_default_registry, ToolRegistry};
use pico_voice::VoicePipeline;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Instant;
use tokio::sync::{broadcast, Mutex};
use tracing::{info, warn};

pub struct PicoRuntime {
    config: PicoConfig,
    session_id: Arc<Mutex<SessionId>>,
    decision: Arc<DecisionEngine>,
    learning: Arc<LearningManager>,
    registry: Arc<ToolRegistry>,
    confirmation_mgr: Arc<ChannelConfirmationManager>,
    memory: Arc<MemoryManager>,
    voice: Arc<VoicePipeline>,
    event_tx: broadcast::Sender<PicoEvent>,
    is_cancelled: Arc<AtomicBool>,
    active_turn_cancel: Arc<Mutex<Option<Arc<AtomicBool>>>>,
    last_observation_id: Arc<Mutex<Option<String>>>,
}

impl PicoRuntime {
    /// Initializes the full Pico assistant runtime with all non-generative subsystems.
    pub fn new(config: PicoConfig) -> Result<Self> {
        let (event_tx, _) = broadcast::channel::<PicoEvent>(256);

        // Security & Sandbox
        let sandbox = Arc::new(PathSandbox::new(
            config.security.allowed_directories.clone(),
            config.security.blocked_directories.clone(),
        ));

        let audit_logger = if config.security.audit_log_path.as_os_str().is_empty() {
            None
        } else {
            Some(Arc::new(AuditLogger::new(&config.security.audit_log_path)))
        };

        let mut policy = SecurityPolicy::new(
            config.security.policy_mode == "Strict",
            config.security.require_confirmation_for_shell,
        );

        let confirmation_timeout =
            std::time::Duration::from_secs(config.security.command_timeout_seconds.max(10));
        let mut confirmation_mgr = ChannelConfirmationManager::new(confirmation_timeout);
        let ev_sender = event_tx.clone();
        confirmation_mgr.set_prompt_callback(Arc::new(move |req_id, tool, args| {
            let _ = ev_sender.send(PicoEvent::ToolPermissionPrompt {
                call_id: req_id.to_string(),
                tool: tool.to_string(),
                arguments: args.clone(),
                reason: format!("Action '{}' requires user confirmation", tool),
            });
        }));
        let confirmation_mgr = Arc::new(confirmation_mgr);
        policy.set_confirmation_provider(confirmation_mgr.clone());

        // Tool Registry
        let registry = Arc::new(build_default_registry(policy, sandbox, audit_logger));

        // Decision Engine
        let decision = Arc::new(DecisionEngine::new(config.decision.clone()));

        // Populate Decision Index with registered tools
        {
            let tool_idx = decision.tool_index();
            let mut idx = tool_idx.write().unwrap();
            for tool in registry.all_tools() {
                let parts: Vec<&str> = tool.name().split('.').collect();
                let cat = if (tool.name().starts_with("windows.")
                    || tool.name().starts_with("linux."))
                    && parts.len() >= 2
                {
                    parts[1]
                } else {
                    match tool.name() {
                        n if n.starts_with("sys.") || n.contains("system") => "system",
                        n if n.starts_with("process.") || n.contains("process") => "process",
                        n if n.starts_with("fs.") || n.contains("file") => "filesystem",
                        n if n.starts_with("audio.") || n.contains("volume") => "audio",
                        n if n.starts_with("power.") || n.contains("lock") => "power",
                        n if n.starts_with("network.") || n.contains("ping") => "network",
                        n if n.contains("calc") || n.contains("math") => "calculator",
                        n if n.contains("timer") => "timer",
                        n if n.contains("clip") => "clipboard",
                        n if n.contains("web") || n.contains("http") => "network",
                        _ => "utility",
                    }
                };

                let os = if tool.name().starts_with("windows.") {
                    "windows"
                } else if tool.name().starts_with("linux.") {
                    "linux"
                } else if tool.name().starts_with("audio.") || tool.name().starts_with("power.") {
                    "windows"
                } else {
                    "all"
                };

                idx.add_entry(ToolEntry {
                    name: tool.name().to_string(),
                    description: tool.description().to_string(),
                    category: cat.to_string(),
                    os: os.to_string(),
                    permission_tier: tool.permission_tier(),
                    parameters_schema: tool.parameters_schema(),
                });
            }
        }

        // Learning Manager
        let learning = Arc::new(LearningManager::new(config.learning.clone())?);

        // Memory Manager
        let memory = Arc::new(MemoryManager::new(config.memory.clone())?);

        // Voice Pipeline
        let voice = Arc::new(VoicePipeline::new(config.voice.clone(), event_tx.clone())?);

        let session_id = Arc::new(Mutex::new(SessionId::new()));
        let is_cancelled = Arc::new(AtomicBool::new(false));
        let active_turn_cancel = Arc::new(Mutex::new(None));
        let last_observation_id = Arc::new(Mutex::new(None));

        info!(
            model = %decision.model_name(),
            version = %decision.model_version(),
            "Pico probabilistic decision runtime initialized successfully"
        );

        Ok(Self {
            config,
            session_id,
            decision,
            learning,
            registry,
            confirmation_mgr,
            memory,
            voice,
            event_tx,
            is_cancelled,
            active_turn_cancel,
            last_observation_id,
        })
    }

    /// Delivers an interactive confirmation response for a pending confirmation request.
    pub async fn respond_confirmation(&self, request_id: &str, approved: bool) -> Result<()> {
        self.confirmation_mgr.respond(request_id, approved).await
    }

    /// Sets a custom asynchronous confirmation provider (e.g. for interactive CLI terminal prompts).
    pub async fn set_confirmation_provider(&self, provider: Arc<dyn ConfirmationProvider>) {
        self.registry.set_confirmation_provider(provider).await;
    }

    /// Subscribes to the runtime's real-time event bus.
    pub fn subscribe_events(&self) -> broadcast::Receiver<PicoEvent> {
        self.event_tx.subscribe()
    }

    /// Returns the active session ID.
    pub async fn current_session_id(&self) -> SessionId {
        *self.session_id.lock().await
    }

    /// Resets or creates a new conversational session.
    pub async fn reset_session(&self) -> SessionId {
        let mut sid = self.session_id.lock().await;
        *sid = SessionId::new();
        let _ = self
            .event_tx
            .send(PicoEvent::SessionCreated { session_id: *sid });
        info!(session_id = %sid, "Session reset");
        *sid
    }

    /// Processes a text input turn from the user via non-generative probabilistic decision intelligence.
    pub async fn process_turn(&self, user_input: &str, speak_response: bool) -> Result<String> {
        let turn_cancel = Arc::new(AtomicBool::new(false));
        {
            let mut lock = self.active_turn_cancel.lock().await;
            *lock = Some(turn_cancel.clone());
        }
        self.is_cancelled.store(false, Ordering::Relaxed);
        let sid = self.current_session_id().await;
        let t_start = Instant::now();

        info!(session = %sid, text = %user_input, "Processing turn with Decision Engine");

        // 1. Record user message in conversational memory
        let user_msg = Message::new_user(sid, user_input);
        self.memory.save_message(&user_msg).await?;

        let _ = self.event_tx.send(PicoEvent::DecisionStarted {
            request: user_input.to_string(),
            candidate_count: 15,
        });

        // 2. Evaluate objective through Decision Engine
        let decision_outcome = self.decision.decide(user_input).await?;

        let final_text = match decision_outcome {
            DecisionResult::DirectConversation { response } => {
                let _ = self.event_tx.send(PicoEvent::DecisionSelected {
                    action: None,
                    confidence: 1.0,
                    latency_ms: t_start.elapsed().as_millis() as u64,
                });

                // Record assistant message
                let asst_msg = Message::new_assistant(sid, &response, None);
                self.memory.save_message(&asst_msg).await?;
                response
            }
            DecisionResult::ClarificationRequired {
                prompt,
                top_candidates,
            } => {
                let _ = self.event_tx.send(PicoEvent::ClarificationRequested {
                    question: prompt.clone(),
                    candidates: top_candidates
                        .iter()
                        .map(|c| c.action_name.clone())
                        .collect(),
                });

                // Persist as clarified observation
                let obs_id = uuid::Uuid::new_v4().to_string();
                let obs = ObservationRecord {
                    id: obs_id.clone(),
                    timestamp: Utc::now(),
                    request_text: user_input.to_string(),
                    primary_intent: "ambiguous".to_string(),
                    candidate_names: top_candidates
                        .iter()
                        .map(|c| c.action_name.clone())
                        .collect(),
                    candidate_scores: top_candidates.iter().map(|c| c.probability).collect(),
                    selected_action: "clarify".to_string(),
                    confidence: top_candidates.first().map(|c| c.probability).unwrap_or(0.0),
                    execution_status: "clarified".to_string(),
                    latency_ms: t_start.elapsed().as_millis() as u64,
                    user_feedback: None,
                    corrected_action: None,
                    quality: LabelQuality::Inferred,
                };
                let _ = self.learning.record_observation(obs);
                *self.last_observation_id.lock().await = Some(obs_id.clone());

                let asst_msg = Message::new_assistant(sid, &prompt, None);
                self.memory.save_message(&asst_msg).await?;
                prompt
            }
            DecisionResult::ActionSelected {
                action_name,
                arguments,
                confidence,
                ranked_scores,
            } => {
                let _ = self.event_tx.send(PicoEvent::CandidatesRanked {
                    candidates: ranked_scores.clone(),
                });

                let _ = self.event_tx.send(PicoEvent::DecisionSelected {
                    action: Some(action_name.clone()),
                    confidence,
                    latency_ms: t_start.elapsed().as_millis() as u64,
                });

                if turn_cancel.load(Ordering::Relaxed) || self.is_cancelled.load(Ordering::Relaxed)
                {
                    return Err(PicoError::Cancelled("Turn was cancelled by user".into()));
                }

                let call_id = uuid::Uuid::new_v4().to_string();
                let tool_call = ToolCall {
                    id: call_id.clone(),
                    name: action_name.clone(),
                    arguments: arguments.clone(),
                    reason: None,
                };

                let _ = self.event_tx.send(PicoEvent::ToolCallRequested {
                    call: tool_call.clone(),
                });

                let _ = self.event_tx.send(PicoEvent::ToolExecutionStarted {
                    call_id: call_id.clone(),
                    tool: action_name.clone(),
                });

                // Execute tool via secured registry
                let result = self
                    .registry
                    .execute_call(&call_id, &action_name, arguments.clone())
                    .await;

                let _ = self.event_tx.send(PicoEvent::ToolExecutionFinished {
                    result: result.clone(),
                });

                // Deterministic outcome synthesis
                let synthesized = ResponseSynthesizer::format_tool_result(
                    &action_name,
                    &arguments,
                    &result.content,
                );

                // Save tool result and assistant response into memory
                let tool_msg = Message::new_tool_result(sid, &result);
                self.memory.save_message(&tool_msg).await?;

                let asst_msg = Message::new_assistant(sid, &synthesized, Some(vec![tool_call]));
                self.memory.save_message(&asst_msg).await?;

                // Record structured experience observation into continuous learning dataset
                let obs_id = uuid::Uuid::new_v4().to_string();
                let obs = ObservationRecord {
                    id: obs_id.clone(),
                    timestamp: Utc::now(),
                    request_text: user_input.to_string(),
                    primary_intent: action_name.clone(),
                    candidate_names: ranked_scores
                        .iter()
                        .map(|c| c.action_name.clone())
                        .collect(),
                    candidate_scores: ranked_scores.iter().map(|c| c.probability).collect(),
                    selected_action: action_name.clone(),
                    confidence,
                    execution_status: if result.success {
                        "success".into()
                    } else {
                        "failure".into()
                    },
                    latency_ms: t_start.elapsed().as_millis() as u64,
                    user_feedback: None,
                    corrected_action: None,
                    quality: if result.success {
                        LabelQuality::ExecutionOutcome
                    } else {
                        LabelQuality::Inferred
                    },
                };

                let _ = self.learning.record_observation(obs);
                *self.last_observation_id.lock().await = Some(obs_id.clone());

                let _ = self.event_tx.send(PicoEvent::ExperienceObserved {
                    observation_id: obs_id,
                    action: action_name,
                    success: result.success,
                });

                synthesized
            }
        };

        // 3. Spoken response synthesis if requested
        if speak_response && !final_text.is_empty() {
            if let Err(e) = self.voice.speak(&final_text).await {
                warn!(error = %e, "Voice synthesis skipped or failed");
            }
        }

        Ok(final_text)
    }

    /// Provides user feedback (+1 or -1) on the most recent turn.
    pub async fn provide_feedback(&self, feedback: i32) -> Result<()> {
        let lock = self.last_observation_id.lock().await;
        if let Some(ref obs_id) = *lock {
            self.learning.record_feedback(obs_id, feedback)?;
            info!(obs_id = %obs_id, feedback, "Recorded feedback for turn");
            Ok(())
        } else {
            Err(PicoError::NotFound(
                "No recent observation found for feedback".into(),
            ))
        }
    }

    /// Records an explicit user correction against the most recent decision.
    pub async fn provide_correction(&self, target_action: &str) -> Result<()> {
        let lock = self.last_observation_id.lock().await;
        if let Some(ref obs_id) = *lock {
            self.learning.record_correction(obs_id, target_action)?;
            info!(obs_id = %obs_id, target_action, "Recorded user correction");
            Ok(())
        } else {
            Err(PicoError::NotFound(
                "No recent observation found for correction".into(),
            ))
        }
    }

    /// Manually triggers model retraining and evaluates promotion against held-out benchmarks.
    pub async fn trigger_training(&self) -> Result<Option<(String, EvaluationMetrics)>> {
        let curr_weights = self.decision.current_weights();
        let obs_count = self.learning.store().count_observations().unwrap_or(0);

        let _ = self.event_tx.send(PicoEvent::ModelTrainingStarted {
            version: "candidate".to_string(),
            eligible_observations: obs_count,
        });

        let res = self.learning.train_and_evaluate(&curr_weights)?;
        if let Some((new_weights, version, metrics)) = res {
            self.decision.update_weights(new_weights, version.clone());
            let _ = self.event_tx.send(PicoEvent::ModelTrainingCompleted {
                version: version.clone(),
                top1_accuracy: metrics.top1_accuracy,
                top3_accuracy: metrics.top3_recall,
                promoted: metrics.passed_promotion,
            });
            Ok(Some((version, metrics)))
        } else {
            Ok(None)
        }
    }

    /// Rolls back active decision model to a target version or the previous checkpoint.
    pub async fn rollback_model(&self, target_version: Option<&str>) -> Result<ModelCheckpoint> {
        let cp = self.learning.rollback(target_version)?;
        self.decision
            .update_weights(cp.weights.clone(), cp.version.clone());
        let _ = self.event_tx.send(PicoEvent::ModelRollbackOccurred {
            target_version: cp.version.clone(),
        });
        Ok(cp)
    }

    /// Triggers immediate cancellation of active tasks, tools, or audio playback.
    pub fn cancel(&self) {
        self.is_cancelled.store(true, Ordering::SeqCst);
        if let Ok(lock) = self.active_turn_cancel.try_lock() {
            if let Some(ref turn_cancel) = *lock {
                turn_cancel.store(true, Ordering::SeqCst);
            }
        }
        self.voice.interrupt_playback();
        info!("PicoRuntime turn cancelled");
    }

    /// Access the underlying VoicePipeline.
    pub fn voice(&self) -> &Arc<VoicePipeline> {
        &self.voice
    }

    /// Access the underlying MemoryManager.
    pub fn memory(&self) -> &Arc<MemoryManager> {
        &self.memory
    }

    /// Access the underlying ToolRegistry.
    pub fn registry(&self) -> &Arc<ToolRegistry> {
        &self.registry
    }

    /// Access the DecisionEngine.
    pub fn decision(&self) -> &Arc<DecisionEngine> {
        &self.decision
    }

    /// Access the LearningManager.
    pub fn learning(&self) -> &Arc<LearningManager> {
        &self.learning
    }

    /// Access the runtime config.
    pub fn config(&self) -> &PicoConfig {
        &self.config
    }
}
