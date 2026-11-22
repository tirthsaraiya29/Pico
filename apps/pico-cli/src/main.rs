use clap::{Parser, Subcommand};
use colored::*;
use pico_audio::AudioDeviceManager;
use pico_core::config::PicoConfig;
use pico_core::events::PicoEvent;
use pico_core::traits::DecisionModel;
use pico_learning::LabelQuality;
use pico_orchestrator::PicoRuntime;
use std::io::{self, Write};
use std::path::PathBuf;
use std::sync::Arc;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

#[derive(Parser)]
#[command(
    name = "pico",
    author = "Tirth Saraiya",
    version = "0.1.0",
    about = "Real-time, locally capable AI assistant with Probabilistic Decision Intelligence"
)]
struct Cli {
    #[arg(short, long, value_name = "FILE", default_value = "configs/pico.toml")]
    config: PathBuf,

    #[command(subcommand)]
    command: Option<Commands>,
}

#[derive(Subcommand)]
enum Commands {
    /// Start interactive terminal chat session with probabilistic tool execution
    Chat {
        #[arg(long, default_value_t = false)]
        speak: bool,
    },
    /// Start real-time hands-free voice assistant mode
    Voice,
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
    /// View recent experience observations
    History {
        #[arg(short, long, default_value_t = 10)]
        limit: usize,
    },
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
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

    match cli.command.unwrap_or(Commands::Chat { speak: false }) {
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
            Ok(())
        }
        Commands::Devices => {
            println!(
                "{}",
                "=== Audio Input Devices (Microphones) ===".cyan().bold()
            );
            match AudioDeviceManager::list_input_devices() {
                Ok(devices) => {
                    for dev in devices {
                        let def_mark = if dev.is_default {
                            "[Default]".green()
                        } else {
                            "".normal()
                        };
                        println!(" - {} {}", dev.name, def_mark);
                    }
                }
                Err(e) => println!("{} Failed to query input devices: {}", "[ERROR]".red(), e),
            }

            println!(
                "\n{}",
                "=== Audio Output Devices (Speakers) ===".cyan().bold()
            );
            match AudioDeviceManager::list_output_devices() {
                Ok(devices) => {
                    for dev in devices {
                        let def_mark = if dev.is_default {
                            "[Default]".green()
                        } else {
                            "".normal()
                        };
                        println!(" - {} {}", dev.name, def_mark);
                    }
                }
                Err(e) => println!("{} Failed to query output devices: {}", "[ERROR]".red(), e),
            }
            Ok(())
        }
        Commands::Tools => {
            let config = load_or_default_config(&cli.config);
            let runtime = PicoRuntime::new(config)?;
            let tools = runtime.registry().all_tools();

            println!(
                "\n{}",
                format!("=== Registered Tools ({}) ===", tools.len())
                    .cyan()
                    .bold()
            );
            for t in tools {
                let tier = match t.permission_tier() {
                    pico_core::traits::PermissionTier::ReadOnly => "[ReadOnly]".green(),
                    pico_core::traits::PermissionTier::LowRisk => "[LowRisk]".yellow(),
                    pico_core::traits::PermissionTier::ConfirmationRequired => {
                        "[Confirm]".magenta().bold()
                    }
                    pico_core::traits::PermissionTier::Restricted => "[Restricted]".red().bold(),
                };
                println!(
                    " - {:<24} {} {}",
                    t.name().bold(),
                    tier,
                    t.description().dimmed()
                );
            }
            Ok(())
        }
        Commands::Learn { action } => {
            let config = load_or_default_config(&cli.config);
            let runtime = PicoRuntime::new(config)?;

            match action.unwrap_or(LearnAction::Status) {
                LearnAction::Status => {
                    let stats = runtime.learning().stats()?;
                    println!(
                        "{}",
                        "=== Pico Continuous Learning Status ===".cyan().bold()
                    );
                    println!(
                        "  Active Model Version : {}",
                        stats.active_version.green().bold()
                    );
                    println!(
                        "  Benchmark Top-1 Acc  : {:.1}%",
                        stats.top1_accuracy * 100.0
                    );
                    println!("  Benchmark Top-3 Rec  : {:.1}%", stats.top3_recall * 100.0);
                    println!("  Total Observations   : {}", stats.total_observations);
                    println!("  Saved Checkpoints    : {}", stats.available_checkpoints);
                    println!(
                        "  Ranking Weights      : sem={:.2}, lex={:.2}, intent={:.2}, hist={:.2}",
                        stats.active_weights.w_semantic,
                        stats.active_weights.w_lexical,
                        stats.active_weights.w_intent,
                        stats.active_weights.w_history
                    );
                }
                LearnAction::Train => {
                    println!(
                        "{}",
                        "Triggering on-device model training and benchmark validation...".cyan()
                    );
                    match runtime.trigger_training().await? {
                        Some((version, metrics)) => {
                            println!(
                                "{} Model promoted successfully to {} (Top-1: {:.1}%, Top-3: {:.1}%)",
                                "[PROMOTED]".green().bold(),
                                version,
                                metrics.top1_accuracy * 100.0,
                                metrics.top3_recall * 100.0
                            );
                        }
                        None => {
                            println!("{}", "[INFO] Training completed; candidate did not exceed promotion criteria or insufficient samples.".yellow());
                        }
                    }
                }
                LearnAction::Rollback { version } => {
                    let res = runtime.rollback_model(version.as_deref()).await?;
                    println!(
                        "{} Rolled back active model to version {}",
                        "[ROLLBACK]".yellow().bold(),
                        res.version.bold()
                    );
                }
                LearnAction::History { limit } => {
                    let obs = runtime
                        .learning()
                        .store()
                        .get_training_observations(LabelQuality::Inferred, limit)?;
                    println!(
                        "{}",
                        format!("=== Recent Observations (last {}) ===", obs.len())
                            .cyan()
                            .bold()
                    );
                    for o in obs {
                        println!(
                            " [{}] \"{}\" -> {} ({:.0}%) status:{}",
                            o.timestamp.format("%H:%M:%S"),
                            o.request_text,
                            o.selected_action.bold(),
                            o.confidence * 100.0,
                            o.execution_status
                        );
                    }
                }
            }
            Ok(())
        }
        Commands::Doctor => run_doctor(&cli.config).await,
        Commands::Chat { speak } => run_chat_loop(&cli.config, speak).await,
        Commands::Voice => run_voice_loop(&cli.config).await,
    }
}

fn load_or_default_config(path: &PathBuf) -> PicoConfig {
    if path.exists() {
        match PicoConfig::load_from_file(path) {
            Ok(c) => c,
            Err(e) => {
                eprintln!(
                    "{} Failed loading {}: {}. Using default config.",
                    "[WARN]".yellow(),
                    path.display(),
                    e
                );
                PicoConfig::default()
            }
        }
    } else {
        PicoConfig::default()
    }
}

async fn run_doctor(config_path: &PathBuf) -> Result<(), Box<dyn std::error::Error>> {
    println!(
        "{}",
        "==================================================".cyan()
    );
    println!(
        "{}",
        "           PICO DIAGNOSTIC SYSTEM DOCTOR          "
            .cyan()
            .bold()
    );
    println!(
        "{}",
        "==================================================".cyan()
    );

    // 1. Toolchain & Platform
    println!("\n[1/5] Checking Platform & Environment...");
    let stats = pico_platform::PlatformSysInfo::get_system_stats();
    match stats {
        Ok(s) => {
            println!("  {} Host OS: {}", "[OK]".green(), s.os);
            println!("  {} CPU Cores: {}", "[OK]".green(), s.cpu_cores);
            println!(
                "  {} Total Memory: {} MB (Available: {} MB)",
                "[OK]".green(),
                s.total_memory_mb,
                s.available_memory_mb
            );
        }
        Err(e) => println!("  {} Platform error: {}", "[FAIL]".red(), e),
    }

    // 2. Audio Devices
    println!("\n[2/5] Checking Audio Capture & Playback Subsystem...");
    let in_devs = AudioDeviceManager::list_input_devices().unwrap_or_default();
    let out_devs = AudioDeviceManager::list_output_devices().unwrap_or_default();
    println!("  {} Microphones found: {}", "[OK]".green(), in_devs.len());
    println!(
        "  {} Audio output devices found: {}",
        "[OK]".green(),
        out_devs.len()
    );

    // 3. Speech Synthesis
    println!("\n[3/5] Testing Native Speech Synthesis...");
    let tts = pico_voice::NativeWindowsSynthesizer::new(None, 1.0);
    match tts.synthesize_sync("Pico diagnostic check complete.") {
        Ok(bytes) => println!(
            "  {} Native SAPI generated {} PCM audio bytes",
            "[OK]".green(),
            bytes.len()
        ),
        Err(e) => println!("  {} SAPI synthesis warning: {}", "[WARN]".yellow(), e),
    }

    // 4. Memory Database
    println!("\n[4/5] Testing SQLite Persistent Memory...");
    let temp_db =
        std::env::temp_dir().join(format!("pico_doc_{}.sqlite", uuid::Uuid::new_v4().simple()));
    let mem = pico_memory::SqliteStore::open(&temp_db);
    match mem {
        Ok(_) => {
            println!(
                "  {} SQLite database initialization and WAL journaling verified",
                "[OK]".green()
            );
            let _ = std::fs::remove_file(temp_db);
        }
        Err(e) => println!("  {} Database check failed: {}", "[FAIL]".red(), e),
    }

    // 5. Probabilistic Decision Model & Continuous Learning
    println!("\n[5/5] Checking Probabilistic Decision Intelligence & Continuous Learning...");
    let cfg = load_or_default_config(config_path);
    match PicoRuntime::new(cfg) {
        Ok(runtime) => {
            println!(
                "  {} Decision Model: {} (Version: {})",
                "[OK]".green(),
                runtime.decision().model_name(),
                runtime.decision().model_version()
            );
            let tools = runtime.registry().all_tools();
            println!(
                "  {} Tool Catalog indexed: {} operating system tools",
                "[OK]".green(),
                tools.len()
            );
            let stats = runtime.learning().stats()?;
            println!(
                "  {} Continuous Learning Store: Active (Total experiences: {})",
                "[OK]".green(),
                stats.total_observations
            );
        }
        Err(e) => {
            println!(
                "  {} Decision Engine initialization error: {}",
                "[FAIL]".red(),
                e
            );
        }
    }

    println!("\n{}", "Diagnostic pass completed.".green().bold());
    Ok(())
}

async fn run_chat_loop(
    config_path: &PathBuf,
    speak: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    let config = load_or_default_config(config_path);
    let runtime = PicoRuntime::new(config)?;

    println!(
        "{}",
        "==================================================".cyan()
    );
    println!(
        "{}",
        "      PICO REAL-TIME AI ASSISTANT (CLI)           "
            .cyan()
            .bold()
    );
    println!(
        "{}",
        "  Type your message below. Commands: /up, /down, /correct <act>, 'exit'.   ".dimmed()
    );
    println!(
        "{}",
        "==================================================".cyan()
    );

    let mut event_rx = runtime.subscribe_events();
    tokio::spawn(async move {
        while let Ok(event) = event_rx.recv().await {
            match event {
                PicoEvent::DecisionSelected {
                    action, confidence, ..
                } => {
                    let act_str = action.unwrap_or_else(|| "direct_conversation".into());
                    println!(
                        "\n{} Decision: {} ({:.0}% confidence)",
                        "⚡ [DECIDE]".cyan().bold(),
                        act_str.bold(),
                        confidence * 100.0
                    );
                }
                PicoEvent::ToolCallRequested { call } => {
                    println!(
                        "{} Calling tool: {} with {}",
                        "⚙ [TOOL]".yellow().bold(),
                        call.name.bold(),
                        call.arguments
                    );
                }
                PicoEvent::ToolExecutionFinished { result } => {
                    let status = if result.success {
                        "[OK]".green()
                    } else {
                        "[ERR]".red()
                    };
                    println!(
                        "{} Tool {} completed in {}ms: {}",
                        status, result.name, result.execution_duration_ms, result.content
                    );
                }
                PicoEvent::SpeechSynthesisStarted { text_length } => {
                    print!(
                        "{} Synthesizing {} chars of speech...\r",
                        "♪ [TTS]".magenta(),
                        text_length
                    );
                    let _ = io::stdout().flush();
                }
                _ => {}
            }
        }
    });

    let stdin = io::stdin();
    let mut line_buf = String::new();

    loop {
        print!("\n{} ", "You >".green().bold());
        io::stdout().flush()?;
        line_buf.clear();

        if stdin.read_line(&mut line_buf)? == 0 {
            break;
        }

        let trimmed = line_buf.trim();
        if trimmed.eq_ignore_ascii_case("exit") || trimmed.eq_ignore_ascii_case("quit") {
            println!("{}", "Exiting Pico. Goodbye!".yellow());
            break;
        }

        if trimmed == "/up" || trimmed == "/thumbup" {
            let _ = runtime.provide_feedback(1).await;
            println!("{}", "✓ Recorded thumbs up feedback (+1).".green());
            continue;
        }

        if trimmed == "/down" || trimmed == "/thumbdown" {
            let _ = runtime.provide_feedback(-1).await;
            println!("{}", "✓ Recorded thumbs down feedback (-1).".yellow());
            continue;
        }

        if let Some(target) = trimmed.strip_prefix("/correct ") {
            let _ = runtime.provide_correction(target.trim()).await;
            println!(
                "{}",
                format!("✓ Recorded correction to action: {}", target.trim()).green()
            );
            continue;
        }

        if trimmed.is_empty() {
            continue;
        }

        print!("{} ", "Pico >".cyan().bold());
        io::stdout().flush()?;

        match runtime.process_turn(trimmed, speak).await {
            Ok(reply) => {
                println!("{}", reply);
            }
            Err(e) => {
                println!("{} {}", "[ERROR]".red().bold(), e);
            }
        }
    }

    Ok(())
}

async fn run_voice_loop(config_path: &PathBuf) -> Result<(), Box<dyn std::error::Error>> {
    let config = load_or_default_config(config_path);
    let runtime = Arc::new(PicoRuntime::new(config)?);

    println!(
        "{}",
        "==================================================".cyan()
    );
    println!(
        "{}",
        "      PICO HANDS-FREE VOICE ASSISTANT             "
            .cyan()
            .bold()
    );
    println!(
        "{}",
        "  Listening continuously via microphone.          ".green()
    );
    println!(
        "{}",
        "  Speak naturally. Speak during output to barge in.".dimmed()
    );
    println!(
        "{}",
        "  Press Ctrl+C to terminate voice mode.           ".dimmed()
    );
    println!(
        "{}",
        "==================================================".cyan()
    );

    let (speech_tx, mut speech_rx) = tokio::sync::mpsc::channel::<String>(16);
    let _capture = runtime.voice().start_listening(speech_tx)?;

    let rt = runtime.clone();
    tokio::spawn(async move {
        while let Some(user_speech) = speech_rx.recv().await {
            println!("\n{} \"{}\"", "🎤 Heard:".green().bold(), user_speech);
            print!("{} Evaluating decision...", "⚡".cyan());
            io::stdout().flush().ok();

            match rt.process_turn(&user_speech, true).await {
                Ok(reply) => {
                    println!("\n{} \"{}\"", "🔊 Spoke:".cyan().bold(), reply);
                }
                Err(e) => {
                    println!("\n{} {}", "[Voice processing error]".red(), e);
                }
            }
        }
    });

    tokio::signal::ctrl_c().await?;
    println!("\n{}", "Voice session ended.".yellow());
    Ok(())
}
