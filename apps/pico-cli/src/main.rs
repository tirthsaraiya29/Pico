use clap::{Parser, Subcommand};
use colored::*;
use pico_audio::AudioDeviceManager;
use pico_core::config::PicoConfig;
use pico_core::events::PicoEvent;
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
    about = "Real-time, locally capable AI voice assistant"
)]
struct Cli {
    #[arg(short, long, value_name = "FILE", default_value = "configs/pico.toml")]
    config: PathBuf,

    #[command(subcommand)]
    command: Option<Commands>,
}

#[derive(Subcommand)]
enum Commands {
    /// Start interactive terminal chat session with tool execution
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
    /// Run diagnostic environment health checks
    Doctor,
    /// Generate a fresh configuration file
    InitConfig {
        #[arg(short, long, default_value = "configs/pico.toml")]
        output: PathBuf,
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
            let defs = runtime.registry().get_tool_definitions();
            println!(
                "{} Registered Tools ({} total):\n",
                "=== PICO TOOL REGISTRY ===".cyan().bold(),
                defs.len()
            );
            for d in defs {
                let name = d
                    .pointer("/function/name")
                    .and_then(|v| v.as_str())
                    .unwrap_or("unknown");
                let desc = d
                    .pointer("/function/description")
                    .and_then(|v| v.as_str())
                    .unwrap_or("");
                println!("• {}: {}", name.yellow().bold(), desc);
            }
            Ok(())
        }
        Commands::Doctor => {
            run_doctor(&cli.config).await?;
            Ok(())
        }
        Commands::Chat { speak } => {
            run_chat_loop(&cli.config, speak).await?;
            Ok(())
        }
        Commands::Voice => {
            run_voice_loop(&cli.config).await?;
            Ok(())
        }
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

    // 5. Inference Connectivity
    println!("\n[5/5] Checking Local Inference Provider (Ollama / Llama.cpp)...");
    let cfg = load_or_default_config(config_path);
    let url = format!("{}/api/tags", cfg.inference.endpoint.trim_end_matches('/'));
    match reqwest::get(&url).await {
        Ok(resp) => {
            if resp.status().is_success() {
                println!(
                    "  {} Ollama service reachable at {}",
                    "[OK]".green(),
                    cfg.inference.endpoint
                );
                println!(
                    "  {} Target model configured: {}",
                    "[OK]".green(),
                    cfg.inference.model
                );
            } else {
                println!(
                    "  {} Ollama returned HTTP {}",
                    "[WARN]".yellow(),
                    resp.status()
                );
            }
        }
        Err(e) => {
            println!(
                "  {} Inference endpoint ({}) unreachable: {}",
                "[WARN]".yellow(),
                cfg.inference.endpoint,
                e
            );
            println!("     Make sure Ollama is running (`ollama serve`) or adjust endpoint in configs/pico.toml");
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
        "  Type your message below. Type 'exit' to quit.   ".dimmed()
    );
    println!(
        "{}",
        "==================================================".cyan()
    );

    let mut event_rx = runtime.subscribe_events();
    tokio::spawn(async move {
        while let Ok(event) = event_rx.recv().await {
            match event {
                PicoEvent::ToolCallRequested { call } => {
                    println!(
                        "\n{} Calling tool: {} with {}",
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
            print!("{} Thinking and responding...", "⚡".cyan());
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
