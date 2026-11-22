use pico_core::config::PicoConfig;
use pico_core::error::{PicoError, Result};
use pico_core::events::PicoEvent;
use pico_core::session::{Message, SessionId};
use pico_core::traits::{InferenceProvider, MemoryStore};
use pico_inference::InferenceFactory;
use pico_memory::{ContextBudgeter, MemoryManager};
use pico_security::{AuditLogger, PathSandbox, SecurityPolicy};
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
    inference: Arc<dyn InferenceProvider>,
    registry: Arc<ToolRegistry>,
    memory: Arc<MemoryManager>,
    voice: Arc<VoicePipeline>,
    event_tx: broadcast::Sender<PicoEvent>,
    is_cancelled: Arc<AtomicBool>,
}

impl PicoRuntime {
    /// Initializes the full Pico assistant runtime with all subsystems.
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

        let policy = SecurityPolicy::new(
            config.security.policy_mode == "Strict",
            config.security.require_confirmation_for_shell,
        );

        // Tool Registry
        let registry = Arc::new(build_default_registry(policy, sandbox, audit_logger));

        // Inference Provider
        let inference = InferenceFactory::create_provider(&config.inference);

        // Memory Manager
        let memory = Arc::new(MemoryManager::new(config.memory.clone())?);

        // Voice Pipeline
        let voice = Arc::new(VoicePipeline::new(config.voice.clone(), event_tx.clone())?);

        let session_id = Arc::new(Mutex::new(SessionId::new()));
        let is_cancelled = Arc::new(AtomicBool::new(false));

        info!(
            provider = %inference.provider_name(),
            model = %inference.model_name(),
            "Pico runtime initialized successfully"
        );

        Ok(Self {
            config,
            session_id,
            inference,
            registry,
            memory,
            voice,
            event_tx,
            is_cancelled,
        })
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

    /// Processes a text input turn from the user.
    pub async fn process_turn(&self, user_input: &str, speak_response: bool) -> Result<String> {
        self.is_cancelled.store(false, Ordering::Relaxed);
        let sid = self.current_session_id().await;
        let t_start = Instant::now();

        info!(session = %sid, text = %user_input, "Processing turn");

        // 1. Record user message
        let user_msg = Message::new_user(sid, user_input);
        self.memory.save_message(&user_msg).await?;

        // 2. Query relevant long-term memory facts
        let facts = self
            .memory
            .search_relevant_context(user_input, 3)
            .await
            .unwrap_or_default();
        let memory_context = ContextBudgeter::format_memory_context(&facts);

        // 3. Assemble budgeted conversation history
        let mut history = self.memory.get_budgeted_history(&sid).await?;

        // Merge memory context into effective system prompt
        let mut effective_system = self.config.inference.system_prompt.clone();
        if let Some(mem_block) = memory_context {
            effective_system.push_str(&mem_block);
        }

        // 4. Retrieve tool definitions
        let tool_defs = self.registry.get_tool_definitions();

        let _ = self.event_tx.send(PicoEvent::InferenceStarted {
            model: self.inference.model_name().to_string(),
            prompt_tokens: history.len() * 10,
        });

        // 5. Query Inference Provider
        let mut current_turn_depth = 0;
        let max_turn_depth = 4; // allow multi-step tool execution loops
        let mut final_text = String::new();

        while current_turn_depth < max_turn_depth {
            if self.is_cancelled.load(Ordering::Relaxed) {
                return Err(PicoError::Cancelled("Turn was cancelled by user".into()));
            }

            let response = self
                .inference
                .generate_response(&history, Some(&effective_system), Some(&tool_defs))
                .await?;

            // If the model produced tool calls
            if !response.tool_calls.is_empty() {
                // Record assistant message requesting tool calls
                let asst_tool_msg = Message::new_assistant(
                    sid,
                    &response.content,
                    Some(response.tool_calls.clone()),
                );
                self.memory.save_message(&asst_tool_msg).await?;
                history.push(asst_tool_msg);

                // Execute each tool call
                for call in response.tool_calls {
                    let _ = self
                        .event_tx
                        .send(PicoEvent::ToolCallRequested { call: call.clone() });

                    let _ = self.event_tx.send(PicoEvent::ToolExecutionStarted {
                        call_id: call.id.clone(),
                        tool: call.name.clone(),
                    });

                    let result = self
                        .registry
                        .execute_call(&call.id, &call.name, call.arguments)
                        .await;

                    let _ = self.event_tx.send(PicoEvent::ToolExecutionFinished {
                        result: result.clone(),
                    });

                    // Record tool result into history
                    let tool_result_msg = Message::new_tool_result(sid, &result);
                    self.memory.save_message(&tool_result_msg).await?;
                    history.push(tool_result_msg);
                }

                current_turn_depth += 1;
            } else {
                // Model completed with natural language response
                final_text = response.content;
                let _ = self.event_tx.send(PicoEvent::InferenceFinished {
                    total_tokens: response.usage_tokens.unwrap_or(final_text.len() / 4),
                    latency_ms: t_start.elapsed().as_millis() as u64,
                });
                break;
            }
        }

        // 6. Save final assistant message
        let final_msg = Message::new_assistant(sid, &final_text, None);
        self.memory.save_message(&final_msg).await?;

        // 7. Spoken response synthesis if requested
        if speak_response && !final_text.is_empty() {
            if let Err(e) = self.voice.speak(&final_text).await {
                warn!(error = %e, "Voice synthesis skipped or failed");
            }
        }

        Ok(final_text)
    }

    /// Triggers immediate cancellation of active generation, tool tasks, or audio playback.
    pub fn cancel(&self) {
        self.is_cancelled.store(true, Ordering::Relaxed);
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
}
