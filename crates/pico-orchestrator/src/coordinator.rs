use chrono::Utc;
use pico_core::config::PicoConfig;
use pico_core::error::{PicoError, Result};
use pico_core::events::PicoEvent;
use pico_core::session::{ClarificationState, Message, SessionId, ToolCall};
use pico_core::traits::{CandidateScore, MemoryStore};
use pico_decision::{
    resolve_followup, ClarificationOutcome, DecisionEngine, DecisionResult, ResponseSynthesizer,
    ToolEntry,
};
use pico_learning::{
    EvaluationMetrics, LabelQuality, LearningManager, ModelCheckpoint, ObservationRecord,
};
use pico_memory::MemoryManager;
use pico_security::{
    AuditLogger, ChannelConfirmationManager, ConfirmationProvider, PathSandbox, SecurityPolicy,
};
use pico_tools::{build_default_registry, ToolRegistry};
use pico_voice::VoicePipeline;
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Instant;
use tokio::sync::{broadcast, Mutex};
use tracing::{info, warn};

type ClarificationStore = Arc<Mutex<HashMap<SessionId, ClarificationState>>>;

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
    clarifications: ClarificationStore,
}

impl PicoRuntime {
    /// Initializes the full Pico assistant runtime with all non-generative subsystems.
    pub fn new(config: PicoConfig) -> Result<Self> {
        let (event_tx, _) = broadcast::channel::<PicoEvent>(256);

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

        let registry = Arc::new(build_default_registry(policy, sandbox, audit_logger));
        let decision = Arc::new(DecisionEngine::new(config.decision.clone()));

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
                        "open_application" | "close_application" | "app_launch" => "application",
                        n if n.starts_with("app.") || n.contains("application") => "application",
                        n if n.starts_with("sys.") || n.contains("system") => "system",
                        n if n.starts_with("process.") || n.contains("process") => "process",
                        n if n.starts_with("fs.") || n.contains("file") => "filesystem",
                        n if n.starts_with("audio.") || n.contains("volume") => "audio",
                        n if n.starts_with("display.") || n.contains("brightness") => "display",
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

        let learning = Arc::new(LearningManager::new(config.learning.clone())?);
        let memory = Arc::new(MemoryManager::new(config.memory.clone())?);

        decision.set_history_provider(learning.clone());
        decision.set_memory_store(memory.clone());

        if let Ok(Some(active_chk)) = learning.store().get_active_checkpoint() {
            decision.update_weights(active_chk.weights, active_chk.version);
        }

        let voice = Arc::new(VoicePipeline::new(config.voice.clone(), event_tx.clone())?);

        Ok(Self {
            config,
            session_id: Arc::new(Mutex::new(SessionId::new())),
            decision,
            learning,
            registry,
            confirmation_mgr,
            memory,
            voice,
            event_tx,
            is_cancelled: Arc::new(AtomicBool::new(false)),
            active_turn_cancel: Arc::new(Mutex::new(None)),
            last_observation_id: Arc::new(Mutex::new(None)),
            clarifications: Arc::new(Mutex::new(HashMap::new())),
        })
    }

    pub async fn respond_confirmation(&self, request_id: &str, approved: bool) -> Result<()> {
        self.confirmation_mgr.respond(request_id, approved).await
    }

    pub async fn set_confirmation_provider(&self, provider: Arc<dyn ConfirmationProvider>) {
        self.registry.set_confirmation_provider(provider).await;
    }

    pub fn subscribe_events(&self) -> broadcast::Receiver<PicoEvent> {
        self.event_tx.subscribe()
    }

    pub async fn current_session_id(&self) -> SessionId {
        *self.session_id.lock().await
    }

    pub async fn reset_session(&self) -> SessionId {
        let mut sid = self.session_id.lock().await;
        *sid = SessionId::new();
        let _ = self
            .event_tx
            .send(PicoEvent::SessionCreated { session_id: *sid });
        info!(session_id = %sid, "Session reset");
        *sid
    }

    /// Returns whether the specified session currently holds an active clarification state.
    pub async fn has_pending_clarification(&self, session_id: SessionId) -> bool {
        let map = self.clarifications.lock().await;
        map.get(&session_id).is_some_and(|st| !st.is_expired())
    }

    /// Allows tests and diagnostics to inspect or adjust clarification state for a session.
    pub async fn set_session_clarification(
        &self,
        session_id: SessionId,
        state: Option<ClarificationState>,
    ) {
        let mut map = self.clarifications.lock().await;
        if let Some(st) = state {
            map.insert(session_id, st);
        } else {
            map.remove(&session_id);
        }
    }

    /// Processes a turn using the active default runtime session.
    pub async fn process_turn(&self, user_input: &str, speak_response: bool) -> Result<String> {
        let sid = self.current_session_id().await;
        self.process_turn_in_session(sid, user_input, speak_response)
            .await
    }

    /// Processes a turn strictly scoped to a specific `SessionId` without cross-session state leakage.
    pub async fn process_turn_in_session(
        &self,
        sid: SessionId,
        user_input: &str,
        speak_response: bool,
    ) -> Result<String> {
        let turn_cancel = Arc::new(AtomicBool::new(false));
        {
            let mut lock = self.active_turn_cancel.lock().await;
            *lock = Some(turn_cancel.clone());
        }
        self.is_cancelled.store(false, Ordering::Relaxed);
        let t_start = Instant::now();

        info!(session = %sid, text = %user_input, "Processing turn with Decision Engine");

        let user_msg = Message::new_user(sid, user_input);
        self.memory.save_message(&user_msg).await?;

        // 1. Check active session-scoped ClarificationState without wiping on mismatch
        let pending_opt = {
            let mut map = self.clarifications.lock().await;
            map.remove(&sid)
        };

        if let Some(pending_state) = pending_opt {
            match resolve_followup(&pending_state, user_input) {
                ClarificationOutcome::ExecuteAction {
                    action_name,
                    arguments,
                    original_request,
                    selection_method,
                } => {
                    info!(
                        session = %sid,
                        action = %action_name,
                        method = %selection_method,
                        "Resolved pending clarification"
                    );
                    let _ = self.event_tx.send(PicoEvent::ClarificationResolved {
                        session_id: sid,
                        selected_action: Some(action_name.clone()),
                        cancelled: false,
                    });

                    // Record correction against the selected tool when disambiguated
                    if let Some(obs_id) = self.last_observation_id.lock().await.as_ref() {
                        let _ = self.learning.record_correction(obs_id, &action_name);
                    }

                    let ranked_scores: Vec<CandidateScore> = pending_state
                        .options
                        .iter()
                        .map(|o| CandidateScore {
                            action_name: o.action_name.clone(),
                            raw_score: o.probability,
                            probability: o.probability,
                            selection_probability: o.selection_probability,
                            executability_probability: o.probability,
                            intent_match: 1.0,
                            lexical_match: 0.5,
                            semantic_match: 0.5,
                            historical_success_rate: 0.5,
                        })
                        .collect();

                    let (reply, _) = self
                        .execute_verified_tool_call(
                            sid,
                            &original_request,
                            &action_name,
                            arguments,
                            1.0,
                            ranked_scores,
                            t_start,
                            &turn_cancel,
                        )
                        .await?;

                    return self.finalize_reply(sid, &reply, speak_response).await;
                }
                ClarificationOutcome::NeedMoreArguments {
                    updated_state,
                    prompt,
                }
                | ClarificationOutcome::StillPending {
                    updated_state,
                    prompt,
                } => {
                    if updated_state.attempts >= 3 {
                        let abort_msg =
                            "Cancelled clarification after multiple unclear responses.".to_string();
                        let _ = self.event_tx.send(PicoEvent::ClarificationResolved {
                            session_id: sid,
                            selected_action: None,
                            cancelled: true,
                        });
                        return self.finalize_reply(sid, &abort_msg, speak_response).await;
                    }
                    self.clarifications.lock().await.insert(sid, updated_state);
                    let _ = self.event_tx.send(PicoEvent::ClarificationRequested {
                        question: prompt.clone(),
                        candidates: pending_state
                            .options
                            .iter()
                            .map(|o| o.action_name.clone())
                            .collect(),
                    });
                    return self.finalize_reply(sid, &prompt, speak_response).await;
                }
                ClarificationOutcome::Cancelled { message } => {
                    let _ = self.event_tx.send(PicoEvent::ClarificationResolved {
                        session_id: sid,
                        selected_action: None,
                        cancelled: true,
                    });
                    return self.finalize_reply(sid, &message, speak_response).await;
                }
                ClarificationOutcome::Expired {
                    was_followup_reference,
                    message,
                } => {
                    let _ = self.event_tx.send(PicoEvent::ClarificationResolved {
                        session_id: sid,
                        selected_action: None,
                        cancelled: true,
                    });
                    if was_followup_reference {
                        return self.finalize_reply(sid, &message, speak_response).await;
                    }
                    // Otherwise fall through cleanly to evaluate the new standalone request
                }
                ClarificationOutcome::InterruptedByNewCommand => {
                    let _ = self.event_tx.send(PicoEvent::ClarificationResolved {
                        session_id: sid,
                        selected_action: None,
                        cancelled: true,
                    });
                    // Fall through cleanly to evaluate the new interrupting request
                }
            }
        }

        // 2. Normal request path (including compound sequential queries)
        let sub_queries = split_compound_queries(user_input);
        let total_sub_queries = sub_queries.len();
        let mut step_responses: Vec<String> = Vec::new();
        let mut executed_actions: Vec<(String, serde_json::Value)> = Vec::new();
        let max_cands = self.config.decision.max_candidates.max(1);
        let clarify_ttl = self.config.decision.clarification_timeout_seconds;

        for (q_idx, query) in sub_queries.iter().enumerate() {
            if turn_cancel.load(Ordering::Relaxed) || self.is_cancelled.load(Ordering::Relaxed) {
                return Err(PicoError::Cancelled("Turn was cancelled by user".into()));
            }

            if q_idx >= 5 {
                warn!("Autonomous execution step limit of 5 reached; halting.");
                step_responses.push("Reached maximum autonomous step limit (5).".into());
                break;
            }

            if total_sub_queries > 1 {
                let _ = self.event_tx.send(PicoEvent::StatusUpdate {
                    status: format!("Step {}/{}: {}", q_idx + 1, total_sub_queries, query),
                });
            }

            let _ = self.event_tx.send(PicoEvent::DecisionStarted {
                request: query.to_string(),
                candidate_count: max_cands,
            });

            let decision_outcome = self.decision.decide(query).await?;

            match decision_outcome {
                DecisionResult::DirectConversation { response } => {
                    let _ = self.event_tx.send(PicoEvent::DecisionSelected {
                        action: None,
                        confidence: 1.0,
                        latency_ms: t_start.elapsed().as_millis() as u64,
                    });
                    step_responses.push(response);
                }
                DecisionResult::ClarificationRequired {
                    kind,
                    prompt,
                    options,
                    top_candidates,
                } => {
                    let _ = self.event_tx.send(PicoEvent::ClarificationRequested {
                        question: prompt.clone(),
                        candidates: top_candidates
                            .iter()
                            .map(|c| c.action_name.clone())
                            .collect(),
                    });

                    let obs_id = uuid::Uuid::new_v4().to_string();
                    let obs = ObservationRecord {
                        id: obs_id.clone(),
                        timestamp: Utc::now(),
                        request_text: query.to_string(),
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
                    *self.last_observation_id.lock().await = Some(obs_id);

                    let cstate = ClarificationState::new(
                        sid,
                        query.to_string(),
                        kind,
                        options,
                        clarify_ttl,
                    );
                    self.clarifications.lock().await.insert(sid, cstate);

                    step_responses.push(prompt);
                    break;
                }
                DecisionResult::ActionSelected {
                    action_name,
                    arguments,
                    confidence,
                    ranked_scores,
                } => {
                    if executed_actions
                        .iter()
                        .any(|(a, args)| a == &action_name && args == &arguments)
                    {
                        warn!(
                            action = %action_name,
                            args = ?arguments,
                            "Loop detected: repeating identical action with identical arguments"
                        );
                        step_responses.push(format!(
                            "Execution halted: action '{}' was already performed with identical arguments in this turn.",
                            action_name
                        ));
                        break;
                    }
                    executed_actions.push((action_name.clone(), arguments.clone()));

                    let (synthesized, succeeded) = self
                        .execute_verified_tool_call(
                            sid,
                            query,
                            &action_name,
                            arguments,
                            confidence,
                            ranked_scores,
                            t_start,
                            &turn_cancel,
                        )
                        .await?;

                    step_responses.push(synthesized);

                    if !succeeded {
                        warn!("Step failed or failed verification; stopping compound execution.");
                        break;
                    }
                }
            }
        }

        let final_text = step_responses.join("\n\n");
        self.finalize_reply(sid, &final_text, speak_response).await
    }

    #[allow(clippy::too_many_arguments)]
    async fn execute_verified_tool_call(
        &self,
        sid: SessionId,
        request_text: &str,
        action_name: &str,
        arguments: serde_json::Value,
        confidence: f32,
        ranked_scores: Vec<CandidateScore>,
        t_start: Instant,
        turn_cancel: &Arc<AtomicBool>,
    ) -> Result<(String, bool)> {
        // Re-validate tool existence & schema before dispatch
        let tool_opt = self.registry.get_tool(action_name);
        if tool_opt.is_none() {
            return Ok((
                ResponseSynthesizer::format_failure(action_name, "Tool is not registered in this runtime."),
                false,
            ));
        }
        let schema = tool_opt.as_ref().unwrap().parameters_schema();
        if let Some(req_arr) = schema.get("required").and_then(|r| r.as_array()) {
            let missing: Vec<String> = req_arr
                .iter()
                .filter_map(|v| v.as_str())
                .filter(|k| !arguments.as_object().is_some_and(|m| m.contains_key(*k)))
                .map(String::from)
                .collect();
            if !missing.is_empty() {
                return Ok((
                    ResponseSynthesizer::format_missing_args_prompt(action_name, &missing),
                    false,
                ));
            }
        }

        let _ = self.event_tx.send(PicoEvent::CandidatesRanked {
            candidates: ranked_scores.clone(),
        });

        let _ = self.event_tx.send(PicoEvent::DecisionSelected {
            action: Some(action_name.to_string()),
            confidence,
            latency_ms: t_start.elapsed().as_millis() as u64,
        });

        if turn_cancel.load(Ordering::Relaxed) || self.is_cancelled.load(Ordering::Relaxed) {
            return Err(PicoError::Cancelled("Turn was cancelled by user".into()));
        }

        let call_id = uuid::Uuid::new_v4().to_string();
        let tool_call = ToolCall {
            id: call_id.clone(),
            name: action_name.to_string(),
            arguments: arguments.clone(),
            reason: None,
        };

        let _ = self
            .event_tx
            .send(PicoEvent::ToolCallRequested { call: tool_call });
        let _ = self.event_tx.send(PicoEvent::ToolExecutionStarted {
            call_id: call_id.clone(),
            tool: action_name.to_string(),
        });

        let result = self
            .registry
            .execute_call(&call_id, action_name, arguments.clone())
            .await;

        let _ = self.event_tx.send(PicoEvent::ToolExecutionFinished {
            result: result.clone(),
        });

        let verification = crate::verification::ObjectiveVerifier::verify_execution(
            action_name,
            &arguments,
            &result,
        )
        .await;

        let _ = self
            .event_tx
            .send(PicoEvent::ObjectiveVerificationCompleted {
                tool: action_name.to_string(),
                verified: verification.verified,
                evidence: verification.evidence.clone(),
            });

        if !verification.verified {
            warn!(
                action = %action_name,
                evidence = %verification.evidence,
                "Independent objective verification failed"
            );
        }

        let tool_msg = Message::new_tool_result(sid, &result);
        self.memory.save_message(&tool_msg).await?;

        let all_ok = result.success && verification.verified;
        let synthesized = if !result.success {
            let err_msg = result
                .error
                .as_deref()
                .or_else(|| result.content.get("error").and_then(|e| e.as_str()))
                .unwrap_or("Execution failed");
            ResponseSynthesizer::format_failure(action_name, err_msg)
        } else if !verification.verified {
            ResponseSynthesizer::format_failure(action_name, &verification.evidence)
        } else {
            ResponseSynthesizer::format_tool_result(action_name, &arguments, &result.content)
        };

        let obs_id = uuid::Uuid::new_v4().to_string();
        let obs = ObservationRecord {
            id: obs_id.clone(),
            timestamp: Utc::now(),
            request_text: request_text.to_string(),
            primary_intent: action_name.to_string(),
            candidate_names: ranked_scores.iter().map(|c| c.action_name.clone()).collect(),
            candidate_scores: ranked_scores.iter().map(|c| c.probability).collect(),
            selected_action: action_name.to_string(),
            confidence,
            execution_status: if all_ok {
                "success".into()
            } else {
                "failure".into()
            },
            latency_ms: t_start.elapsed().as_millis() as u64,
            user_feedback: None,
            corrected_action: None,
            quality: if all_ok {
                LabelQuality::ExecutionOutcome
            } else {
                LabelQuality::Inferred
            },
        };
        let _ = self.learning.record_observation(obs);
        *self.last_observation_id.lock().await = Some(obs_id.clone());

        let _ = self.event_tx.send(PicoEvent::ExperienceObserved {
            observation_id: obs_id,
            action: action_name.to_string(),
            success: all_ok,
        });

        Ok((synthesized, all_ok))
    }

    async fn finalize_reply(
        &self,
        sid: SessionId,
        reply: &str,
        speak_response: bool,
    ) -> Result<String> {
        let asst_msg = Message::new_assistant(sid, reply, None);
        self.memory.save_message(&asst_msg).await?;

        if speak_response && !reply.is_empty() {
            if let Err(e) = self.voice.speak(reply).await {
                warn!(error = %e, "Voice synthesis skipped or failed");
            }
        }

        Ok(reply.to_string())
    }

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

    pub async fn rollback_model(&self, target_version: Option<&str>) -> Result<ModelCheckpoint> {
        let cp = self.learning.rollback(target_version)?;
        self.decision
            .update_weights(cp.weights.clone(), cp.version.clone());
        let _ = self.event_tx.send(PicoEvent::ModelRollbackOccurred {
            target_version: cp.version.clone(),
        });
        Ok(cp)
    }

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

    pub fn voice(&self) -> &Arc<VoicePipeline> {
        &self.voice
    }

    pub fn memory(&self) -> &Arc<MemoryManager> {
        &self.memory
    }

    pub fn registry(&self) -> &Arc<ToolRegistry> {
        &self.registry
    }

    pub fn decision(&self) -> &Arc<DecisionEngine> {
        &self.decision
    }

    pub fn learning(&self) -> &Arc<LearningManager> {
        &self.learning
    }

    pub fn config(&self) -> &PicoConfig {
        &self.config
    }
}

/// Splits compound multi-step commands into sequenced independent queries.
fn split_compound_queries(input: &str) -> Vec<String> {
    let delimiters = [" and then ", " then ", "; ", " && "];
    let lower = input.to_lowercase();
    let mut matched_delim = None;
    for delim in delimiters {
        if lower.contains(delim) {
            matched_delim = Some(delim);
            break;
        }
    }

    if let Some(delim) = matched_delim {
        let mut parts = Vec::new();
        let mut cur = input;
        while let Some(pos) = cur.to_lowercase().find(delim) {
            let left = cur[..pos].trim();
            if !left.is_empty() {
                parts.push(left.to_string());
            }
            cur = &cur[pos + delim.len()..];
        }
        let rest = cur.trim();
        if !rest.is_empty() {
            parts.push(rest.to_string());
        }
        parts
    } else {
        vec![input.trim().to_string()]
    }
}
