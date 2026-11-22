use axum::{
    extract::{
        ws::{Message as WsMessage, WebSocket, WebSocketUpgrade},
        DefaultBodyLimit, Query, Request, State,
    },
    http::{header, HeaderMap, StatusCode},
    middleware::{self, Next},
    response::{Html, IntoResponse, Response},
    routing::get,
    Router,
};
use clap::Parser;
use colored::*;
use futures::{SinkExt, StreamExt};
use pico_core::config::PicoConfig;
use pico_orchestrator::PicoRuntime;
use serde::Deserialize;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use tower_http::cors::CorsLayer;
use tower_http::services::ServeDir;
use tracing::{error, info, warn};
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

const MAX_CONCURRENT_WS_CLIENTS: usize = 16;
const MAX_WS_MESSAGE_BYTES: usize = 65536; // 64 KB

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

#[derive(Deserialize)]
struct AuthParams {
    token: Option<String>,
}

#[derive(Clone)]
struct AppState {
    runtime: Arc<PicoRuntime>,
    auth_token: String,
    port: u16,
    active_ws_count: Arc<AtomicUsize>,
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
    let bind_ip = &config.desktop.bind_address;

    // Security warning if non-loopback interface is selected
    if bind_ip != "127.0.0.1" && bind_ip != "localhost" {
        warn!(
            bind = %bind_ip,
            "Binding to non-loopback network interface! Ensure proper perimeter firewalling."
        );
    }

    let bind_addr = format!("{}:{}", bind_ip, port);

    // Generate cryptographic local session authentication token
    let auth_token = std::env::var("PICO_AUTH_TOKEN")
        .unwrap_or_else(|_| uuid::Uuid::new_v4().simple().to_string());

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
    let state = AppState {
        runtime,
        auth_token: auth_token.clone(),
        port,
        active_ws_count: Arc::new(AtomicUsize::new(0)),
    };

    let ui_dir = PathBuf::from("apps/pico-desktop/ui");

    // Restrict CORS strictly to local loopback origins
    let cors = CorsLayer::new()
        .allow_origin([
            format!("http://127.0.0.1:{}", port).parse().unwrap(),
            format!("http://localhost:{}", port).parse().unwrap(),
        ])
        .allow_methods([axum::http::Method::GET, axum::http::Method::POST])
        .allow_headers([header::AUTHORIZATION, header::CONTENT_TYPE]);

    let app = Router::new()
        .route("/", get(index_fallback))
        .route("/index.html", get(index_fallback))
        .route("/ws", get(ws_handler))
        .route("/api/status", get(status_handler))
        .route("/api/tools", get(tools_handler))
        .fallback_service(ServeDir::new(&ui_dir))
        .layer(DefaultBodyLimit::max(MAX_WS_MESSAGE_BYTES))
        .layer(middleware::from_fn(security_headers_middleware))
        .layer(cors)
        .with_state(state);

    let listener = tokio::net::TcpListener::bind(&bind_addr).await?;
    let local_url = format!("http://{}", bind_addr);
    println!(
        "{} Server active at {}",
        "[RUNNING]".green().bold(),
        local_url.cyan().underline()
    );
    println!(
        "{} Local Auth Token: {}",
        "[SECURITY]".yellow().bold(),
        auth_token.green()
    );
    println!(
        "{} WebSocket hub protected on ws://{}/ws",
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
    .with_graceful_shutdown(shutdown_signal())
    .await?;

    info!("Pico Desktop server gracefully stopped");
    Ok(())
}

/// Security headers middleware: prevents MIME sniffing, clickjacking, and restricts execution scope
async fn security_headers_middleware(req: Request, next: Next) -> Response {
    let mut resp = next.run(req).await;
    let headers = resp.headers_mut();
    headers.insert(header::X_CONTENT_TYPE_OPTIONS, "nosniff".parse().unwrap());
    headers.insert(header::X_FRAME_OPTIONS, "DENY".parse().unwrap());
    headers.insert(
        header::CONTENT_SECURITY_POLICY,
        "default-src 'self' 'unsafe-inline' https://cdnjs.cloudflare.com blob: data:; font-src https://fonts.gstatic.com; style-src 'self' 'unsafe-inline' https://fonts.googleapis.com; connect-src 'self' ws: wss: http: https:;"
            .parse()
            .unwrap(),
    );
    resp
}

/// Validates whether the request origin is authentic (local origin or local file/app)
fn is_valid_origin(headers: &HeaderMap, port: u16) -> bool {
    if let Some(origin) = headers.get(header::ORIGIN) {
        if let Ok(origin_str) = origin.to_str() {
            let allowed_127 = format!("http://127.0.0.1:{}", port);
            let allowed_local = format!("http://localhost:{}", port);
            return origin_str == "null"
                || origin_str.eq_ignore_ascii_case(&allowed_127)
                || origin_str.eq_ignore_ascii_case(&allowed_local);
        }
    }
    // Absence of Origin header is normal for direct same-origin and desktop webview requests
    true
}

/// Validates whether the client possesses the valid local auth token
fn is_authorized(headers: &HeaderMap, query_token: Option<&str>, expected_token: &str) -> bool {
    // 1. Check query parameter ?token=...
    if let Some(t) = query_token {
        if t == expected_token {
            return true;
        }
    }

    // 2. Check Authorization: Bearer <token>
    if let Some(auth_hdr) = headers.get(header::AUTHORIZATION) {
        if let Ok(auth_str) = auth_hdr.to_str() {
            if let Some(bearer) = auth_str.strip_prefix("Bearer ") {
                if bearer.trim() == expected_token {
                    return true;
                }
            }
        }
    }

    false
}

async fn index_fallback(State(state): State<AppState>) -> impl IntoResponse {
    let index_path = PathBuf::from("apps/pico-desktop/ui/index.html");
    if let Ok(content) = std::fs::read_to_string(index_path) {
        // Inject auth token seamlessly into local frontend
        let injection = format!(
            "<meta name=\"pico-auth-token\" content=\"{}\">\n<script>window.PICO_AUTH_TOKEN=\"{}\";</script>\n</head>",
            state.auth_token, state.auth_token
        );
        let dynamic_html = content.replace("</head>", &injection);
        Html(dynamic_html)
    } else {
        Html("<h1>Pico Desktop UI loading...</h1>".to_string())
    }
}

async fn status_handler(
    headers: HeaderMap,
    Query(params): Query<AuthParams>,
    State(state): State<AppState>,
) -> impl IntoResponse {
    if !is_authorized(&headers, params.token.as_deref(), &state.auth_token) {
        return (StatusCode::UNAUTHORIZED, "Unauthorized").into_response();
    }

    let stats = pico_platform::PlatformSysInfo::get_system_stats().ok();
    axum::Json(serde_json::json!({
        "status": "online",
        "session_id": state.runtime.current_session_id().await.to_string(),
        "system": stats
    }))
    .into_response()
}

async fn tools_handler(
    headers: HeaderMap,
    Query(params): Query<AuthParams>,
    State(state): State<AppState>,
) -> impl IntoResponse {
    if !is_authorized(&headers, params.token.as_deref(), &state.auth_token) {
        return (StatusCode::UNAUTHORIZED, "Unauthorized").into_response();
    }

    let defs = state.runtime.registry().get_tool_definitions();
    axum::Json(serde_json::json!({ "tools": defs })).into_response()
}

async fn ws_handler(
    ws: WebSocketUpgrade,
    headers: HeaderMap,
    Query(params): Query<AuthParams>,
    State(state): State<AppState>,
) -> impl IntoResponse {
    // 1. Strict Origin check to eliminate Cross-Site WebSocket Hijacking (CSWSH)
    if !is_valid_origin(&headers, state.port) {
        warn!("Blocked WebSocket connection from untrusted cross-origin origin");
        return (StatusCode::FORBIDDEN, "Forbidden Origin").into_response();
    }

    // 2. Token authorization
    if !is_authorized(&headers, params.token.as_deref(), &state.auth_token) {
        warn!("Blocked unauthorized WebSocket connection attempt");
        return (StatusCode::UNAUTHORIZED, "Unauthorized").into_response();
    }

    // 3. Connection limit check
    let count = state.active_ws_count.load(Ordering::Relaxed);
    if count >= MAX_CONCURRENT_WS_CLIENTS {
        warn!(active = count, "WebSocket connection limit reached");
        return (StatusCode::TOO_MANY_REQUESTS, "Connection limit exceeded").into_response();
    }

    ws.on_upgrade(move |socket| handle_socket(socket, state))
}

struct ConnectionGuard(Arc<AtomicUsize>);
impl Drop for ConnectionGuard {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::Relaxed);
    }
}

async fn handle_socket(socket: WebSocket, state: AppState) {
    state.active_ws_count.fetch_add(1, Ordering::Relaxed);
    let _guard = ConnectionGuard(state.active_ws_count.clone());

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
                // Reject oversized message payloads
                if text.len() > MAX_WS_MESSAGE_BYTES {
                    warn!(len = text.len(), "Dropped oversized WebSocket message");
                    continue;
                }

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

async fn shutdown_signal() {
    tokio::signal::ctrl_c()
        .await
        .expect("Failed to install Ctrl+C signal handler");
    info!("Shutdown signal received. Closing Desktop server cleanly...");
}
