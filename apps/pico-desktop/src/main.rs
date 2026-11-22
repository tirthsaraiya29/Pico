use axum::{
    extract::{
        ws::{Message as WsMessage, WebSocket, WebSocketUpgrade},
        State,
    },
    response::{Html, IntoResponse},
    routing::get,
    Router,
};
use clap::Parser;
use colored::*;
use futures::{SinkExt, StreamExt};
use pico_core::config::PicoConfig;
use pico_orchestrator::PicoRuntime;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;
use tower_http::cors::CorsLayer;
use tower_http::services::ServeDir;
use tracing::error;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

#[derive(Parser)]
#[command(
    name = "pico-desktop",
    author = "Tirth Saraiya",
    version = "0.1.0",
    about = "Pico Real-time Desktop UI & 3D Assistant Interface"
)]
struct Args {
    #[arg(short, long, default_value = "configs/pico.toml")]
    config: PathBuf,

    #[arg(short, long)]
    port: Option<u16>,

    #[arg(long, default_value_t = false)]
    no_browser: bool,
}

#[derive(Clone)]
struct AppState {
    runtime: Arc<PicoRuntime>,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
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

    let port = args.port.unwrap_or(config.desktop.port);
    let bind_addr = format!("{}:{}", config.desktop.bind_address, port);

    println!(
        "{}",
        "==================================================".cyan()
    );
    println!(
        "{}",
        "      PICO REAL-TIME DESKTOP & 3D INTERFACE       "
            .cyan()
            .bold()
    );
    println!(
        "{}",
        "==================================================".cyan()
    );

    let runtime = Arc::new(PicoRuntime::new(config.clone())?);
    let state = AppState { runtime };

    let ui_dir = PathBuf::from("apps/pico-desktop/ui");
    let app = Router::new()
        .route("/ws", get(ws_handler))
        .route("/api/status", get(status_handler))
        .route("/api/tools", get(tools_handler))
        .fallback_service(ServeDir::new(&ui_dir).fallback(get(index_fallback)))
        .layer(CorsLayer::permissive())
        .with_state(state);

    let listener = tokio::net::TcpListener::bind(&bind_addr).await?;
    let local_url = format!("http://{}", bind_addr);
    println!(
        "{} Server active at {}",
        "[RUNNING]".green().bold(),
        local_url.cyan().underline()
    );
    println!(
        "{} WebSocket hub listening on ws://{}/ws",
        "[STREAM]".yellow().bold(),
        bind_addr
    );

    if config.desktop.auto_open_browser && !args.no_browser {
        let _ = pico_platform::PlatformApps::open_application(&local_url);
    }

    axum::serve(
        listener,
        app.into_make_service_with_connect_info::<SocketAddr>(),
    )
    .await?;
    Ok(())
}

async fn index_fallback() -> impl IntoResponse {
    let index_path = PathBuf::from("apps/pico-desktop/ui/index.html");
    if let Ok(content) = std::fs::read_to_string(index_path) {
        Html(content)
    } else {
        Html("<h1>Pico Desktop UI loading...</h1>".to_string())
    }
}

async fn status_handler(State(state): State<AppState>) -> impl IntoResponse {
    let stats = pico_platform::PlatformSysInfo::get_system_stats().ok();
    axum::Json(serde_json::json!({
        "status": "online",
        "session_id": state.runtime.current_session_id().await.to_string(),
        "system": stats
    }))
}

async fn tools_handler(State(state): State<AppState>) -> impl IntoResponse {
    let defs = state.runtime.registry().get_tool_definitions();
    axum::Json(serde_json::json!({ "tools": defs }))
}

async fn ws_handler(ws: WebSocketUpgrade, State(state): State<AppState>) -> impl IntoResponse {
    ws.on_upgrade(move |socket| handle_socket(socket, state))
}

async fn handle_socket(socket: WebSocket, state: AppState) {
    let (mut sender, mut receiver) = socket.split();
    let mut event_rx = state.runtime.subscribe_events();

    let mut send_task = tokio::spawn(async move {
        while let Ok(event) = event_rx.recv().await {
            if let Ok(json_str) = serde_json::to_string(&event) {
                if sender.send(WsMessage::Text(json_str)).await.is_err() {
                    break;
                }
            }
        }
    });

    let rt = state.runtime.clone();
    let mut recv_task = tokio::spawn(async move {
        while let Some(Ok(msg)) = receiver.next().await {
            if let WsMessage::Text(text) = msg {
                if let Ok(req) = serde_json::from_str::<serde_json::Value>(&text) {
                    let action = req.get("action").and_then(|v| v.as_str()).unwrap_or("");
                    match action {
                        "chat" => {
                            let content = req.get("text").and_then(|v| v.as_str()).unwrap_or("");
                            let speak = req.get("speak").and_then(|v| v.as_bool()).unwrap_or(false);
                            if !content.is_empty() {
                                let rt_clone = rt.clone();
                                let content_str = content.to_string();
                                tokio::spawn(async move {
                                    if let Err(e) = rt_clone.process_turn(&content_str, speak).await
                                    {
                                        error!(error = %e, "WebSocket turn processing failed");
                                    }
                                });
                            }
                        }
                        "cancel" => {
                            rt.cancel();
                        }
                        "reset_session" => {
                            rt.reset_session().await;
                        }
                        _ => {}
                    }
                }
            }
        }
    });

    tokio::select! {
        _ = (&mut send_task) => recv_task.abort(),
        _ = (&mut recv_task) => send_task.abort(),
    };
}
