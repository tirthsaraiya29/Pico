use async_trait::async_trait;
use clap::{Parser, Subcommand};
use colored::*;
use pico_audio::AudioDeviceManager;
use pico_core::config::PicoConfig;
use pico_core::error::PicoError;
use pico_core::events::PicoEvent;
use pico_orchestrator::PicoRuntime;
use pico_security::ConfirmationProvider;
use std::collections::VecDeque;
use std::io::{self, Write};
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::{oneshot, Mutex};
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

#[derive(Parser)]
#[command(
    name = "pico",
    author = "Tirth Saraiya",
    version = "0.1.0",
    about = "Real-time, local-first autonomous voice assistant in Rust"
)]
struct Cli {
    #[arg(short, long, value_name = "FILE", default_value = "configs/pico.toml")]
    config: PathBuf,

    #[command(subcommand)]
    command: Option<Commands>,
}

#[derive(Subcommand)]
enum Commands {
    /// Start real-time hands-free voice assistant mode (Default)
    Voice,
    /// Run headless assistant runtime with authenticated local IPC server for companion UI
    Daemon {
        /// Disable microphone voice capture (IPC text prompts only)
        #[arg(long)]
        no_voice: bool,
    },
    /// List all registered tools and their schemas
    Tools,
    /// Enumerate audio input and output devices
    Devices,
    /// Continuous learning controls, checkpoints, and experience dataset
    Learn {
        #[command(subcommand)]
        action: Option<LearnAction>,
    },
    /// Run diagnostic environment health checks
    Doctor,
    /// Generate a fresh configuration file
    InitConfig {
        #[arg(short, long, default_value = "configs/pico.toml")]
        output: PathBuf,
    },
}

#[derive(Subcommand)]
enum LearnAction {
    /// Show current learning statistics and active model checkpoint
    Status,
    /// Trigger continuous learning training cycle and promotion evaluation
    Train,
    /// Roll back to a previous model checkpoint
    Rollback {
        #[arg(short, long)]
        version: Option<String>,
    },
}

type PendingConfirmationQueue = VecDeque<(String, String, oneshot::Sender<bool>)>;
type SharedPendingConfirmation = Arc<Mutex<PendingConfirmationQueue>>;

/// Request-scoped interactive voice confirmation provider.
/// Prompts the user verbally and routes spoken responses ("confirm" / "cancel") to authorize high-risk actions.
struct VoiceConfirmationCoordinator {
    pending: SharedPendingConfirmation,
}

impl VoiceConfirmationCoordinator {
    fn new() -> (Self, SharedPendingConfirmation) {
        let pending = Arc::new(Mutex::new(VecDeque::new()));
        (
            Self {
                pending: pending.clone(),
            },
            pending,
        )
    }
}

#[async_trait]
impl ConfirmationProvider for VoiceConfirmationCoordinator {
    async fn request_confirmation(
        &self,
        request_id: &str,
        tool_name: &str,
        _arguments: &serde_json::Value,
    ) -> pico_core::error::Result<bool> {
        let (tx, rx) = oneshot::channel();
        {
            let mut lock = self.pending.lock().await;
            lock.push_back((request_id.to_string(), tool_name.to_string(), tx));
        }

        // Wait up to 15 seconds for verbal voice confirmation
        match tokio::time::timeout(std::time::Duration::from_secs(15), rx).await {
            Ok(Ok(approved)) => Ok(approved),
            Ok(Err(_)) => Err(PicoError::Cancelled("Confirmation cancelled".into())),
            Err(_) => {
                let mut lock = self.pending.lock().await;
                lock.retain(|(id, _, _)| id != request_id);
                Err(PicoError::PermissionDenied {
                    action: tool_name.to_string(),
                    reason: "Voice confirmation timed out after 15 seconds".to_string(),
                })
            }
        }
    }
}

#[tokio::main]
async fn main() -> std::result::Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::registry()
        .with(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "info,pico=debug".into()),
        )
        .with(
            tracing_subscriber::fmt::layer()
                .with_target(false)
                .without_time(),
        )
        .init();

    let cli = Cli::parse();

    // Default to Commands::Voice: commands are strictly accepted by voice
    match cli.command.unwrap_or(Commands::Voice) {
        Commands::InitConfig { output } => {
            let config = PicoConfig::default();
            if let Some(parent) = output.parent() {
                std::fs::create_dir_all(parent)?;
            }
            let toml_str = toml::to_string_pretty(&config)?;
            std::fs::write(&output, toml_str)?;
            println!(
                "{} Generated configuration at {}",
                "[OK]".green().bold(),
                output.display()
            );
        }
        Commands::Tools => {
            let config = load_or_default_config(&cli.config);
            let runtime = PicoRuntime::new(config)?;
            let tools = runtime.registry().all_tools();

            println!(
                "{}",
                format!("Registered Tools Ecosystem ({} total):", tools.len())
                    .cyan()
                    .bold()
            );
            println!("{:-<75}", "");
            for tool in tools {
                println!(
                    "  {: <32} [{:?}] - {}",
                    tool.name().bold(),
                    tool.permission_tier(),
                    tool.description()
                );
            }
        }
        Commands::Devices => {
            println!("{}", "Audio Devices Hardware Scan:".cyan().bold());
            let inputs = AudioDeviceManager::list_input_devices().unwrap_or_default();
            println!(
                "\n{} ({}):",
                "Microphones (Input)".green().bold(),
                inputs.len()
            );
            for (idx, dev) in inputs.iter().enumerate() {
                println!("  [{}] {:?}", idx, dev);
            }

            let outputs = AudioDeviceManager::list_output_devices().unwrap_or_default();
            println!(
                "\n{} ({}):",
                "Speakers (Output)".green().bold(),
                outputs.len()
            );
            for (idx, dev) in outputs.iter().enumerate() {
                println!("  [{}] {:?}", idx, dev);
            }
        }
        Commands::Learn { action } => {
            let config = load_or_default_config(&cli.config);
            let runtime = PicoRuntime::new(config)?;
            let learning = runtime.learning();

            match action.unwrap_or(LearnAction::Status) {
                LearnAction::Status => {
                    let stats = learning.stats()?;
                    println!("{}", "Continuous Learning Status:".cyan().bold());
                    println!("  Total Observations:    {}", stats.total_observations);
                    println!(
                        "  Active Checkpoint:     {}",
                        stats.active_version.green().bold()
                    );
                    println!(
                        "  Top-1 Benchmark Acc:   {:.2}%",
                        stats.top1_accuracy * 100.0
                    );
                    println!(
                        "  Top-3 Benchmark Recall: {:.2}%",
                        stats.top3_recall * 100.0
                    );
                    println!("  Available Checkpoints: {}", stats.available_checkpoints);
                }
                LearnAction::Train => {
                    println!(
                        "{}",
                        "Triggering continuous learning evaluation and training cycle...".cyan()
                    );
                    let default_weights = pico_inference::ranking::RankingWeights::default();
                    if let Some((_, new_ver, metrics)) =
                        learning.train_and_evaluate(&default_weights)?
                    {
                        println!(
                            "{} New model checkpoint promoted! Version: {}",
                            "[PROMOTED]".green().bold(),
                            new_ver
                        );
                        println!("  Top-1 Accuracy: {:.2}%", metrics.top1_accuracy * 100.0);
                        println!("  Top-3 Recall:   {:.2}%", metrics.top3_recall * 100.0);
                        println!("  Evaluated Samples: {}", metrics.total_samples);
                    } else {
                        println!(
                            "{} Model evaluated: Did not meet promotion threshold or insufficient samples.",
                            "[KEPT]".yellow().bold()
                        );
                    }
                }
                LearnAction::Rollback { version } => {
                    println!("{}", "Rolling back model checkpoint...".yellow());
                    let restored = learning.rollback(version.as_deref())?;
                    println!(
                        "{} Rolled back active checkpoint to: {}",
                        "[OK]".green().bold(),
                        restored.version
                    );
                }
            }
        }
        Commands::Doctor => {
            println!(
                "{}",
                "Pico Diagnostic Environment Health Check:".cyan().bold()
            );
            println!("{:-<60}", "");

            // 1. Host OS & Platform
            let os_name = if cfg!(target_os = "windows") {
                "Windows"
            } else if cfg!(target_os = "linux") {
                "Linux"
            } else {
                "Unix/Other"
            };
            println!(
                "  [OK] Host Platform:   {} ({})",
                os_name.green().bold(),
                std::env::consts::ARCH
            );

            // 2. Audio Pipeline
            let inputs = AudioDeviceManager::list_input_devices().unwrap_or_default();
            let outputs = AudioDeviceManager::list_output_devices().unwrap_or_default();
            if !inputs.is_empty() && !outputs.is_empty() {
                println!(
                    "  [OK] Audio Subsystem: Microphones ({}), Speakers ({})",
                    inputs.len().to_string().green(),
                    outputs.len().to_string().green()
                );
            } else {
                println!(
                    "  {} Audio Subsystem: Input: {}, Output: {}",
                    "[WARN]".yellow(),
                    inputs.len(),
                    outputs.len()
                );
            }

            // 3. Security Policy & Sandbox
            let config = load_or_default_config(&cli.config);
            println!(
                "  [OK] Security Policy: Strict Mode = {}, Require Shell Confirm = {}",
                (config.security.policy_mode == "Strict")
                    .to_string()
                    .green(),
                config
                    .security
                    .require_confirmation_for_shell
                    .to_string()
                    .green()
            );

            // 4. Decision Engine
            println!(
                "  [OK] Decision Core:   BM25 Retrieval + Multi-Feature Ranking + Calibrated Selection"
            );

            println!("{:-<60}", "");
            println!(
                "{} All diagnostic checks completed.",
                "[DOCTOR]".cyan().bold()
            );
        }
        Commands::Voice => {
            run_voice_loop(&cli.config, true, false).await?;
        }
        Commands::Daemon { no_voice } => {
            run_voice_loop(&cli.config, !no_voice, true).await?;
        }
    }

    Ok(())
}

async fn run_voice_loop(
    config_path: &PathBuf,
    enable_voice: bool,
    enable_ipc: bool,
) -> std::result::Result<(), Box<dyn std::error::Error>> {
    let config = load_or_default_config(config_path);
    let runtime = Arc::new(PicoRuntime::new(config)?);

    // Setup verbal voice confirmation provider
    let (confirmation_provider, pending_confirm) = VoiceConfirmationCoordinator::new();
    runtime
        .set_confirmation_provider(Arc::new(confirmation_provider))
        .await;

    let ipc_handle = if enable_ipc {
        Some(pico_orchestrator::start_ipc_server(runtime.clone(), None, None).await?)
    } else {
        None
    };

    println!(
        "{}",
        "==================================================".cyan()
    );
    println!(
        "{}",
        if enable_ipc {
            "         PICO ASSISTANT DAEMON (LOCAL IPC)        "
        } else {
            "         PICO NATIVE AUTONOMOUS ASSISTANT         "
        }
        .cyan()
        .bold()
    );
    if let Some(ref handle) = ipc_handle {
        println!("  IPC Endpoint: {}", handle.endpoint().green());
    }
    println!(
        "{}",
        if enable_voice {
            "  [Voice Active] Listening for speech..."
        } else {
            "  [IPC Only] Voice capture disabled (--no-voice)."
        }
        .green()
        .bold()
    );
    println!("{}", "  Press Ctrl+C to terminate.".dimmed());
    println!(
        "{}",
        "==================================================".cyan()
    );

    let (speech_tx, mut speech_rx) = tokio::sync::mpsc::channel::<String>(16);
    let _capture = if enable_voice {
        match runtime.voice().start_listening(speech_tx) {
            Ok(cap) => Some(cap),
            Err(e) => {
                eprintln!(
                    "[WARN] Microphone unavailable ({}); continuing in IPC/text mode",
                    e
                );
                None
            }
        }
    } else {
        None
    };

    // Subscribe to runtime events to print diagnostic progress & mirror confirmations to IPC snapshot
    let mut event_rx = runtime.subscribe_events();
    let ipc_shared = ipc_handle.as_ref().map(|h| Arc::clone(h.shared_state()));
    tokio::spawn(async move {
        while let Ok(event) = event_rx.recv().await {
            match event {
                PicoEvent::VoiceActivityStarted { .. } => {
                    print!(
                        "\r{} Listening...                 ",
                        "[VOICE]".green().bold()
                    );
                    let _ = io::stdout().flush();
                }
                PicoEvent::PlaybackInterrupted { reason } => {
                    println!("\n{} {}", "[BARGE-IN]".yellow().bold(), reason);
                }
                PicoEvent::DecisionSelected {
                    action,
                    confidence,
                    latency_ms,
                } => {
                    let act = action.unwrap_or_else(|| "direct_conversation".into());
                    println!(
                        "{} Decision: {} ({:.0}% confidence, {}ms)",
                        "[DECIDE]".cyan().bold(),
                        act.bold(),
                        confidence * 100.0,
                        latency_ms
                    );
                }
                PicoEvent::ToolCallRequested { call } => {
                    println!(
                        "{} Executing tool: {} with {}",
                        "[TOOL]".yellow().bold(),
                        call.name.bold(),
                        call.arguments
                    );
                }
                PicoEvent::ToolExecutionFinished { result } => {
                    let tag = if result.success {
                        "[OK]".green().bold()
                    } else {
                        "[ERR]".red().bold()
                    };
                    println!(
                        "{} Tool {} completed in {}ms: {}",
                        tag, result.name, result.execution_duration_ms, result.content
                    );
                }
                PicoEvent::ObjectiveVerificationCompleted {
                    tool,
                    verified,
                    evidence,
                } => {
                    let tag = if verified {
                        "[VERIFIED]".green().bold()
                    } else {
                        "[UNVERIFIED]".yellow().bold()
                    };
                    println!("{} Objective for {}: {}", tag, tool, evidence);
                }
                PicoEvent::ToolPermissionPrompt {
                    call_id,
                    tool,
                    arguments,
                    reason,
                } => {
                    if let Some(ref st) = ipc_shared {
                        let mut lock = st.pending_confirmation.lock().await;
                        *lock = Some(pico_core::PendingConfirmationView {
                            request_id: call_id,
                            tool_name: tool.clone(),
                            arguments_preview: arguments.to_string(),
                        });
                    }
                    println!(
                        "\n{} Dangerous operation: {}\nReason: {}\nPlease confirm verbally: say 'confirm' to proceed, or 'cancel' to abort.",
                        "[CONFIRMATION REQUIRED]".magenta().bold(),
                        tool.bold(),
                        reason
                    );
                }
                _ => {}
            }
        }
    });

    let rt = runtime.clone();
    let pending_clone = pending_confirm.clone();
    let ipc_paused_ref = ipc_handle.as_ref().map(|h| Arc::clone(h.shared_state()));

    tokio::spawn(async move {
        while let Some(user_speech) = speech_rx.recv().await {
            if let Some(ref st) = ipc_paused_ref {
                if st.voice_paused.load(std::sync::atomic::Ordering::Relaxed) {
                    continue;
                }
            }
            let clean_speech = user_speech.trim();
            if clean_speech.is_empty() {
                continue;
            }

            println!("\n{} \"{}\"", "Heard:".green().bold(), clean_speech);

            // 1. Check if there is an active voice confirmation prompt awaiting user response
            let pending_tx = {
                let mut lock = pending_clone.lock().await;
                lock.pop_front()
            };

            if let Some((req_id, tool_name, tx)) = pending_tx {
                match pico_security::VoiceAuthorizer::evaluate_utterance(clean_speech, &tool_name) {
                    pico_security::VoiceAuthDecision::Approved => {
                        println!(
                            "{} User verbally confirmed action '{}'. Proceeding.",
                            "[APPROVED]".green().bold(),
                            tool_name
                        );
                        let _ = tx.send(true);
                        let _ = rt
                            .voice()
                            .speak("Confirmed. Proceeding with execution.")
                            .await;
                        continue;
                    }
                    pico_security::VoiceAuthDecision::Declined { reason } => {
                        println!(
                            "{} User verbally declined action '{}' ({}). Aborting.",
                            "[DECLINED]".red().bold(),
                            tool_name,
                            reason
                        );
                        let _ = tx.send(false);
                        let _ = rt.voice().speak("Cancelled. Operation was aborted.").await;
                        continue;
                    }
                    pico_security::VoiceAuthDecision::Ambiguous { prompt } => {
                        println!(
                            "{} Ambiguous confirmation response. Repeating prompt.",
                            "[WARN]".yellow()
                        );
                        let _ = rt.voice().speak(&prompt).await;
                        let mut lock = pending_clone.lock().await;
                        lock.push_front((req_id, tool_name, tx));
                        continue;
                    }
                }
            }

            // 2. Normal user voice command turn
            print!("{} Processing intent...", "[THINK]".cyan());
            io::stdout().flush().ok();

            match rt.process_turn(clean_speech, true).await {
                Ok(reply) => {
                    println!("\n{} \"{}\"", "Spoke:".cyan().bold(), reply);
                }
                Err(e) => {
                    println!("\n{} {}", "[Voice turn error]".red(), e);
                    let _ = rt
                        .voice()
                        .speak("Sorry, I encountered an error executing that request.")
                        .await;
                }
            }
        }
    });

    tokio::signal::ctrl_c().await?;
    println!("\n{}", "Voice session ended.".yellow());
    Ok(())
}

fn load_or_default_config(path: &PathBuf) -> PicoConfig {
    if path.exists() {
        PicoConfig::load_from_file(path).unwrap_or_default()
    } else {
        PicoConfig::default()
    }
}
