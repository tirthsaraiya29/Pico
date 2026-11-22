use async_trait::async_trait;
use clap::Parser;
use eframe::egui::{self, Color32, Stroke, Vec2};
use pico_audio::AudioDeviceManager;
use pico_core::config::PicoConfig;
use pico_core::error::{PicoError, Result};
use pico_core::events::PicoEvent;
use pico_orchestrator::PicoRuntime;
use pico_security::ConfirmationProvider;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::{oneshot, Mutex as TokioMutex};
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

#[derive(Parser)]
#[command(
    name = "pico-desktop",
    author = "Tirth Saraiya",
    version = "0.1.0",
    about = "Pico Native Rust Autonomous Voice Assistant Desktop Application"
)]
struct Args {
    #[arg(short, long, default_value = "configs/pico.toml")]
    config: PathBuf,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AssistantState {
    Idle,
    Listening,
    Thinking,
    Confirming,
    Executing,
    Speaking,
}

#[derive(Debug, Clone)]
struct TimelineItem {
    timestamp: chrono::DateTime<chrono::Local>,
    category: TimelineCategory,
    title: String,
    detail: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TimelineCategory {
    VoiceInput,
    Decision,
    ToolExecution,
    SpeechOutput,
    Confirmation,
    BargeIn,
}

struct PendingConfirmation {
    tool_name: String,
    arguments: String,
    responder: oneshot::Sender<bool>,
}

/// Request-scoped interactive voice and UI confirmation coordinator.
struct DesktopConfirmationCoordinator {
    pending: Arc<TokioMutex<Option<PendingConfirmation>>>,
}

#[async_trait]
impl ConfirmationProvider for DesktopConfirmationCoordinator {
    async fn request_confirmation(
        &self,
        _request_id: &str,
        tool_name: &str,
        arguments: &serde_json::Value,
    ) -> Result<bool> {
        let (tx, rx) = oneshot::channel();
        {
            let mut lock = self.pending.lock().await;
            *lock = Some(PendingConfirmation {
                tool_name: tool_name.to_string(),
                arguments: arguments.to_string(),
                responder: tx,
            });
        }

        // Wait up to 20 seconds for verbal or UI click confirmation
        match tokio::time::timeout(std::time::Duration::from_secs(20), rx).await {
            Ok(Ok(approved)) => Ok(approved),
            Ok(Err(_)) => Err(PicoError::Cancelled("Confirmation request cancelled".into())),
            Err(_) => {
                let mut lock = self.pending.lock().await;
                *lock = None;
                Err(PicoError::PermissionDenied {
                    action: tool_name.to_string(),
                    reason: "Voice confirmation timed out after 20 seconds".to_string(),
                })
            }
        }
    }
}

struct PicoDesktopApp {
    runtime: Arc<PicoRuntime>,
    tokio_handle: tokio::runtime::Handle,
    ui_event_rx: std::sync::mpsc::Receiver<(TimelineCategory, String, String)>,
    state: AssistantState,
    energy_level: f32,
    animation_time: f32,
    is_listening_active: bool,
    last_heard: String,
    last_spoken: String,
    timeline: Vec<TimelineItem>,
    pending_confirm: Arc<TokioMutex<Option<PendingConfirmation>>>,
    active_confirm_ui: Option<(String, String)>,
    registered_tools_count: usize,
    active_checkpoint_version: String,
    accuracy_metric: f32,
    device_name: String,
}

impl PicoDesktopApp {
    fn new(
        _cc: &eframe::CreationContext<'_>,
        runtime: Arc<PicoRuntime>,
        tokio_handle: tokio::runtime::Handle,
        pending_confirm: Arc<TokioMutex<Option<PendingConfirmation>>>,
        ui_event_rx: std::sync::mpsc::Receiver<(TimelineCategory, String, String)>,
    ) -> Self {
        let tools_count = runtime.registry().all_tools().len();
        let learning_stats = runtime.learning().stats().ok();
        let (active_ver, acc) = if let Some(stats) = learning_stats {
            (stats.active_version, stats.top1_accuracy)
        } else {
            ("pico-model-v1.0.0".into(), 1.0)
        };

        let device_name = AudioDeviceManager::list_input_devices()
            .ok()
            .and_then(|devs| devs.into_iter().next())
            .map(|d| d.name)
            .unwrap_or_else(|| "Default Microphone (16kHz)".into());

        Self {
            runtime,
            tokio_handle,
            ui_event_rx,
            state: AssistantState::Idle,
            energy_level: 0.05,
            animation_time: 0.0,
            is_listening_active: true,
            last_heard: "Listening for your voice...".into(),
            last_spoken: "Pico autonomous voice assistant ready.".into(),
            timeline: Vec::new(),
            pending_confirm,
            active_confirm_ui: None,
            registered_tools_count: tools_count,
            active_checkpoint_version: active_ver,
            accuracy_metric: acc,
            device_name,
        }
    }

    fn add_timeline_entry(
        &mut self,
        category: TimelineCategory,
        title: impl Into<String>,
        detail: impl Into<String>,
    ) {
        self.timeline.insert(
            0,
            TimelineItem {
                timestamp: chrono::Local::now(),
                category,
                title: title.into(),
                detail: detail.into(),
            },
        );
        if self.timeline.len() > 60 {
            self.timeline.pop();
        }
    }
}

impl eframe::App for PicoDesktopApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        // Continuous smooth 60fps repainting for audio pulse and radar animations
        ctx.request_repaint_after(std::time::Duration::from_millis(16));
        self.animation_time += 0.04;

        // Drain incoming background events
        while let Ok((category, title, detail)) = self.ui_event_rx.try_recv() {
            match category {
                TimelineCategory::VoiceInput => {
                    self.last_heard = title.clone();
                    self.state = AssistantState::Thinking;
                }
                TimelineCategory::Decision => {
                    self.state = AssistantState::Executing;
                }
                TimelineCategory::SpeechOutput => {
                    self.last_spoken = title.clone();
                    self.state = AssistantState::Speaking;
                }
                TimelineCategory::ToolExecution => {
                    self.state = AssistantState::Idle;
                }
                TimelineCategory::Confirmation => {
                    self.state = AssistantState::Confirming;
                }
                TimelineCategory::BargeIn => {
                    self.state = AssistantState::Listening;
                }
            }
            self.add_timeline_entry(category, title, detail);
        }

        // Check if playback is currently active
        if self.runtime.voice().is_playing() {
            self.state = AssistantState::Speaking;
        } else if self.state == AssistantState::Speaking {
            self.state = if self.is_listening_active {
                AssistantState::Listening
            } else {
                AssistantState::Idle
            };
        }

        // Check pending confirmation state
        if let Ok(lock) = self.pending_confirm.try_lock() {
            if let Some(ref c) = *lock {
                self.active_confirm_ui = Some((c.tool_name.clone(), c.arguments.clone()));
                self.state = AssistantState::Confirming;
            } else {
                self.active_confirm_ui = None;
            }
        }

        // Apply dark futuristic theme styling
        let mut visuals = egui::Visuals::dark();
        visuals.override_text_color = Some(Color32::from_rgb(248, 250, 252));
        visuals.panel_fill = Color32::from_rgb(7, 9, 14);
        visuals.window_fill = Color32::from_rgb(14, 18, 27);
        ctx.set_visuals(visuals);

        egui::CentralPanel::default().show(ctx, |ui| {
            // 1. TOP HEADER & TELEMETRY
            ui.horizontal(|ui| {
                ui.label(
                    egui::RichText::new("PICO")
                        .font(egui::FontId::proportional(22.0))
                        .color(Color32::from_rgb(0, 240, 255))
                        .strong(),
                );
                ui.label(
                    egui::RichText::new("AUTONOMOUS LOCAL ASSISTANT (RUST NATIVE)")
                        .font(egui::FontId::monospace(10.0))
                        .color(Color32::from_rgb(148, 163, 184)),
                );

                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    // Checkpoint Pill
                    ui.label(
                        egui::RichText::new(format!(
                            "Model: {} ({:.0}% Acc)",
                            self.active_checkpoint_version,
                            self.accuracy_metric * 100.0
                        ))
                        .font(egui::FontId::monospace(10.5))
                        .color(Color32::from_rgb(16, 185, 129)),
                    );

                    // Tools Count
                    ui.label(
                        egui::RichText::new(format!("Tools: {}", self.registered_tools_count))
                            .font(egui::FontId::monospace(10.5))
                            .color(Color32::from_rgb(0, 240, 255)),
                    );

                    // Mic Pill
                    ui.label(
                        egui::RichText::new(format!("🎙️ {}", self.device_name))
                            .font(egui::FontId::monospace(10.5))
                            .color(Color32::from_rgb(148, 163, 184)),
                    );
                });
            });

            ui.add_space(8.0);
            ui.separator();
            ui.add_space(10.0);

            // 2. DANGEROUS ACTION CONFIRMATION BANNER (if active)
            if let Some((tool_name, args)) = self.active_confirm_ui.clone() {
                ui.group(|ui| {
                    ui.horizontal(|ui| {
                        ui.label(
                            egui::RichText::new("⚠️ CONFIRMATION REQUIRED:")
                                .color(Color32::from_rgb(245, 158, 11))
                                .strong(),
                        );
                        ui.label(
                            egui::RichText::new(&tool_name)
                                .color(Color32::from_rgb(255, 255, 255))
                                .monospace()
                                .strong(),
                        );
                    });
                    ui.label(
                        egui::RichText::new(format!("Parameters: {}", args))
                            .color(Color32::from_rgb(148, 163, 184))
                            .monospace()
                            .small(),
                    );
                    ui.label(
                        egui::RichText::new("Say 'confirm' or 'yes' verbally to authorize, or say 'cancel'. Alternatively, click:")
                            .color(Color32::from_rgb(203, 213, 225)),
                    );

                    ui.horizontal(|ui| {
                        if ui.button(egui::RichText::new("✓ Confirm Action").color(Color32::from_rgb(16, 185, 129)).strong()).clicked() {
                            if let Ok(mut lock) = self.pending_confirm.try_lock() {
                                if let Some(c) = lock.take() {
                                    let _ = c.responder.send(true);
                                    let rt = self.runtime.clone();
                                    self.tokio_handle.spawn(async move {
                                        let _ = rt.voice().speak("Confirmed. Proceeding with execution.").await;
                                    });
                                }
                            }
                        }

                        if ui.button(egui::RichText::new("✗ Cancel Action").color(Color32::from_rgb(244, 63, 94)).strong()).clicked() {
                            if let Ok(mut lock) = self.pending_confirm.try_lock() {
                                if let Some(c) = lock.take() {
                                    let _ = c.responder.send(false);
                                    let rt = self.runtime.clone();
                                    self.tokio_handle.spawn(async move {
                                        let _ = rt.voice().speak("Cancelled. Operation was aborted.").await;
                                    });
                                }
                            }
                        }
                    });
                });
                ui.add_space(8.0);
            }

            // 3. MAIN SPLIT: RADAR VOICE CORE (Left) & TIMELINE (Right)
            ui.columns(2, |columns| {
                // COLUMN 0: RADAR & VOICE VISUALIZER
                columns[0].vertical_centered(|ui| {
                    ui.add_space(10.0);
                    // Custom Painter for Glowing Concentric Voice Radar
                    let (response, painter) = ui.allocate_painter(Vec2::new(300.0, 240.0), egui::Sense::hover());
                    let center = response.rect.center();

                    let (core_color, ring_color) = match self.state {
                        AssistantState::Idle => (
                            Color32::from_rgb(0, 240, 255),
                            Color32::from_rgba_premultiplied(0, 240, 255, 30),
                        ),
                        AssistantState::Listening => (
                            Color32::from_rgb(16, 185, 129),
                            Color32::from_rgba_premultiplied(16, 185, 129, 60),
                        ),
                        AssistantState::Thinking => (
                            Color32::from_rgb(139, 92, 246),
                            Color32::from_rgba_premultiplied(139, 92, 246, 50),
                        ),
                        AssistantState::Confirming => (
                            Color32::from_rgb(245, 158, 11),
                            Color32::from_rgba_premultiplied(245, 158, 11, 70),
                        ),
                        AssistantState::Executing => (
                            Color32::from_rgb(56, 189, 248),
                            Color32::from_rgba_premultiplied(56, 189, 248, 60),
                        ),
                        AssistantState::Speaking => (
                            Color32::from_rgb(244, 63, 94),
                            Color32::from_rgba_premultiplied(244, 63, 94, 70),
                        ),
                    };

                    let pulse = (self.animation_time.sin() * 0.15 + 1.0) * (1.0 + self.energy_level * 0.8);

                    // Concentric expanding rings
                    for i in 1..=3 {
                        let radius = (i as f32 * 26.0) * pulse;
                        painter.circle_stroke(
                            center,
                            radius,
                            Stroke::new(1.5_f32, ring_color),
                        );
                    }

                    // Inner Core Orb
                    let core_radius = 28.0 * pulse;
                    painter.circle_filled(center, core_radius, core_color);
                    painter.circle_stroke(center, core_radius + 2.0, Stroke::new(2.0_f32, Color32::WHITE));

                    ui.add_space(8.0);
                    let state_str = match self.state {
                        AssistantState::Idle => "● IDLE — READY",
                        AssistantState::Listening => "🎙️ LISTENING (SPEAK NOW)",
                        AssistantState::Thinking => "🧠 THINKING & RANKING TOOLS",
                        AssistantState::Confirming => "⚠️ AWAITING VOICE CONFIRMATION",
                        AssistantState::Executing => "⚙️ EXECUTING OS TOOL",
                        AssistantState::Speaking => "🔊 SPEAKING (BARGE-IN READY)",
                    };

                    ui.label(
                        egui::RichText::new(state_str)
                            .font(egui::FontId::monospace(12.0))
                            .color(core_color)
                            .strong(),
                    );

                    ui.add_space(12.0);

                    // Controls: Barge-in & Toggle
                    ui.horizontal(|ui| {
                        if self.runtime.voice().is_playing() {
                            if ui.button(egui::RichText::new("⚡ Barge-in (Stop Speech)").color(Color32::from_rgb(244, 63, 94))).clicked() {
                                self.runtime.voice().interrupt_playback();
                            }
                        }

                        let listen_label = if self.is_listening_active {
                            "Mic: ALWAYS-ON (VAD Active)"
                        } else {
                            "Mic: PAUSED"
                        };
                        if ui.button(egui::RichText::new(listen_label).color(Color32::from_rgb(0, 240, 255))).clicked() {
                            self.is_listening_active = !self.is_listening_active;
                        }
                    });

                    ui.add_space(14.0);
                    // Speech Display Cards
                    ui.group(|ui| {
                        ui.set_width(320.0);
                        ui.label(
                            egui::RichText::new("🎤 Heard from User:")
                                .font(egui::FontId::monospace(10.0))
                                .color(Color32::from_rgb(148, 163, 184)),
                        );
                        ui.label(
                            egui::RichText::new(&self.last_heard)
                                .font(egui::FontId::proportional(13.0))
                                .color(Color32::WHITE),
                        );
                    });

                    ui.add_space(6.0);
                    ui.group(|ui| {
                        ui.set_width(320.0);
                        ui.label(
                            egui::RichText::new("🔊 Pico Spoke:")
                                .font(egui::FontId::monospace(10.0))
                                .color(Color32::from_rgb(148, 163, 184)),
                        );
                        ui.label(
                            egui::RichText::new(&self.last_spoken)
                                .font(egui::FontId::proportional(13.0))
                                .color(Color32::from_rgb(0, 240, 255)),
                        );
                    });
                });

                // COLUMN 1: LIVE EVENT & TOOL EXECUTION TIMELINE
                columns[1].vertical(|ui| {
                    ui.label(
                        egui::RichText::new("EXECUTION TIMELINE & ATTRIBUTION")
                            .font(egui::FontId::monospace(11.0))
                            .color(Color32::from_rgb(148, 163, 184))
                            .strong(),
                    );
                    ui.add_space(4.0);

                    egui::ScrollArea::vertical().max_height(480.0).show(ui, |ui| {
                        if self.timeline.is_empty() {
                            ui.label(
                                egui::RichText::new("Speak a command into your microphone to begin...\nExample: 'What is my CPU usage?' or 'Lock the computer' or 'Calculate 25 times 40'.")
                                    .color(Color32::from_rgb(100, 116, 139))
                                    .italics(),
                            );
                        }

                        for item in &self.timeline {
                            ui.group(|ui| {
                                ui.horizontal(|ui| {
                                    let (tag_color, tag_icon) = match item.category {
                                        TimelineCategory::VoiceInput => (Color32::from_rgb(16, 185, 129), "🎤 HEARD"),
                                        TimelineCategory::Decision => (Color32::from_rgb(139, 92, 246), "🧠 DECIDE"),
                                        TimelineCategory::ToolExecution => (Color32::from_rgb(56, 189, 248), "⚙️ TOOL"),
                                        TimelineCategory::SpeechOutput => (Color32::from_rgb(244, 63, 94), "🔊 SPOKE"),
                                        TimelineCategory::Confirmation => (Color32::from_rgb(245, 158, 11), "⚠️ CONFIRM"),
                                        TimelineCategory::BargeIn => (Color32::from_rgb(234, 179, 8), "⚡ BARGE-IN"),
                                    };

                                    ui.label(
                                        egui::RichText::new(tag_icon)
                                            .font(egui::FontId::monospace(9.5))
                                            .color(tag_color)
                                            .strong(),
                                    );
                                    ui.label(
                                        egui::RichText::new(&item.title)
                                            .font(egui::FontId::proportional(12.0))
                                            .color(Color32::WHITE)
                                            .strong(),
                                    );
                                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                        ui.label(
                                            egui::RichText::new(item.timestamp.format("%H:%M:%S").to_string())
                                                .font(egui::FontId::monospace(9.0))
                                                .color(Color32::from_rgb(100, 116, 139)),
                                        );
                                    });
                                });

                                if !item.detail.is_empty() {
                                    ui.label(
                                        egui::RichText::new(&item.detail)
                                            .font(egui::FontId::monospace(10.0))
                                            .color(Color32::from_rgb(148, 163, 184)),
                                    );
                                }
                            });
                            ui.add_space(2.0);
                        }
                    });
                });
            });

            // 4. BOTTOM STATUS BAR
            ui.add_space(10.0);
            ui.separator();
            ui.horizontal(|ui| {
                ui.label(
                    egui::RichText::new("✓ Zero Cloud Telemetry")
                        .font(egui::FontId::monospace(9.5))
                        .color(Color32::from_rgb(16, 185, 129)),
                );
                ui.label(
                    egui::RichText::new("✓ Sub-15ms Audio Barge-in")
                        .font(egui::FontId::monospace(9.5))
                        .color(Color32::from_rgb(0, 240, 255)),
                );
                ui.label(
                    egui::RichText::new("✓ Zero-Trust OS Sandbox")
                        .font(egui::FontId::monospace(9.5))
                        .color(Color32::from_rgb(139, 92, 246)),
                );
                ui.label(
                    egui::RichText::new("✓ Non-Generative Decision Intelligence")
                        .font(egui::FontId::monospace(9.5))
                        .color(Color32::from_rgb(245, 158, 11)),
                );
            });
        });
    }
}

fn main() -> std::result::Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::registry()
        .with(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "info,pico=debug".into()),
        )
        .with(tracing_subscriber::fmt::layer().with_target(false))
        .init();

    let args = Args::parse();
    let config = if args.config.exists() {
        PicoConfig::load_from_file(&args.config)?
    } else {
        PicoConfig::default()
    };

    // Initialize async Tokio runtime in background
    let tokio_runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?;
    let handle = tokio_runtime.handle().clone();

    // Initialize full Pico assistant runtime
    let pico_runtime = Arc::new(PicoRuntime::new(config)?);

    // Setup voice confirmation coordinator
    let pending_confirm = Arc::new(TokioMutex::new(None));
    let coordinator = DesktopConfirmationCoordinator {
        pending: pending_confirm.clone(),
    };
    let rt_clone = pico_runtime.clone();
    tokio_runtime.block_on(async {
        rt_clone
            .set_confirmation_provider(Arc::new(coordinator))
            .await;
    });

    // Start voice listening loop in background
    let (speech_tx, mut speech_rx) = tokio::sync::mpsc::channel::<String>(16);
    let _capture = pico_runtime.voice().start_listening(speech_tx)?;

    // Shared UI update channel
    let (ui_event_tx, ui_event_rx) = std::sync::mpsc::channel::<(TimelineCategory, String, String)>();
    let ui_tx_clone = ui_event_tx.clone();

    // Background speech processing loop
    let rt_speech = pico_runtime.clone();
    let pending_speech = pending_confirm.clone();
    tokio_runtime.spawn(async move {
        while let Some(user_speech) = speech_rx.recv().await {
            let clean = user_speech.trim().to_string();
            if clean.is_empty() {
                continue;
            }

            let _ = ui_tx_clone.send((TimelineCategory::VoiceInput, clean.clone(), "".into()));

            // Check if there is an active voice confirmation prompt
            let pending_tx = {
                let mut lock = pending_speech.lock().await;
                lock.take()
            };

            if let Some(c) = pending_tx {
                let lower = clean.to_lowercase();
                if lower.contains("yes")
                    || lower.contains("confirm")
                    || lower.contains("proceed")
                    || lower.contains("go ahead")
                    || lower.contains("do it")
                    || lower.contains("sure")
                    || lower.contains("ok")
                {
                    let _ = c.responder.send(true);
                    let _ = ui_tx_clone.send((TimelineCategory::Confirmation, format!("Confirmed: {}", c.tool_name), "User authorized dangerous execution.".into()));
                    let _ = rt_speech.voice().speak("Confirmed. Proceeding with execution.").await;
                    continue;
                } else if lower.contains("no")
                    || lower.contains("cancel")
                    || lower.contains("abort")
                    || lower.contains("stop")
                    || lower.contains("don't")
                {
                    let _ = c.responder.send(false);
                    let _ = ui_tx_clone.send((TimelineCategory::Confirmation, format!("Declined: {}", c.tool_name), "User cancelled dangerous execution.".into()));
                    let _ = rt_speech.voice().speak("Cancelled. Operation was aborted.").await;
                    continue;
                } else {
                    let _ = rt_speech.voice().speak(&format!("Please say confirm or cancel to proceed with {}.", c.tool_name)).await;
                    let mut lock = pending_speech.lock().await;
                    *lock = Some(c);
                    continue;
                }
            }

            // Normal voice turn execution
            match rt_speech.process_turn(&clean, true).await {
                Ok(reply) => {
                    let _ = ui_tx_clone.send((TimelineCategory::SpeechOutput, reply.clone(), "".into()));
                }
                Err(e) => {
                    let _ = ui_tx_clone.send((TimelineCategory::SpeechOutput, "Execution Error".into(), e.to_string()));
                    let _ = rt_speech.voice().speak("Sorry, I encountered an error processing that request.").await;
                }
            }
        }
    });

    // Event listener loop for telemetry updates
    let mut event_sub = pico_runtime.subscribe_events();
    let ui_event_tx_sub = ui_event_tx.clone();
    tokio_runtime.spawn(async move {
        while let Ok(event) = event_sub.recv().await {
            match event {
                PicoEvent::DecisionSelected { action, confidence, latency_ms } => {
                    let act = action.unwrap_or_else(|| "direct_conversation".into());
                    let _ = ui_event_tx_sub.send((
                        TimelineCategory::Decision,
                        act,
                        format!("{:.0}% calibrated confidence, {}ms latency", confidence * 100.0, latency_ms),
                    ));
                }
                PicoEvent::ToolExecutionFinished { result } => {
                    let status = if result.success { "Success" } else { "Error" };
                    let _ = ui_event_tx_sub.send((
                        TimelineCategory::ToolExecution,
                        format!("{}: {}", result.name, status),
                        format!("{}ms duration | {}", result.execution_duration_ms, result.content),
                    ));
                }
                PicoEvent::PlaybackInterrupted { reason } => {
                    let _ = ui_event_tx_sub.send((
                        TimelineCategory::BargeIn,
                        "Playback Interrupted".into(),
                        reason,
                    ));
                }
                _ => {}
            }
        }
    });

    // Launch native eframe application window
    let native_options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1040.0, 720.0])
            .with_min_inner_size([760.0, 520.0])
            .with_title("Pico — Autonomous Local Voice Assistant"),
        ..Default::default()
    };

    eframe::run_native(
        "Pico Desktop",
        native_options,
        Box::new(move |cc| {
            let app = PicoDesktopApp::new(cc, pico_runtime, handle, pending_confirm, ui_event_rx);
            Ok(Box::new(app))
        }),
    )?;

    Ok(())
}
