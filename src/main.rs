use std::{
    env,
    error::Error,
    net::{IpAddr, SocketAddr},
};

use axum::{extract::State, http::StatusCode, response::IntoResponse, routing::get, Json, Router};
use serde::Serialize;
use tokio::net::TcpListener;
use tower_http::trace::TraceLayer;
use tracing::{error, info};
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt, EnvFilter};

#[derive(Clone, Copy)]
struct AppState {
    bind_addr: SocketAddr,
}

#[derive(Serialize)]
struct HealthResponse {
    status: &'static str,
    bind_addr: String,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    load_env();
    init_tracing();

    let bind_addr = read_bind_addr()?;
    let state = AppState { bind_addr };

    let app = Router::new()
        .route("/", get(root))
        .route("/health", get(health))
        .layer(TraceLayer::new_for_http())
        .with_state(state);

    let listener = TcpListener::bind(bind_addr).await?;
    info!("listening on {}", bind_addr);

    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal())
        .await?;

    Ok(())
}

async fn root() -> &'static str {
    "miketang84-forum002"
}

async fn health(State(state): State<AppState>) -> impl IntoResponse {
    let payload = HealthResponse {
        status: "ok",
        bind_addr: state.bind_addr.to_string(),
    };

    (StatusCode::OK, Json(payload))
}

fn load_env() {
    if dotenvy::dotenv().is_err() {
        let _ = dotenvy::from_filename(".env.production");
    }
}

fn init_tracing() {
    let env_filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new("miketang84_forum002=debug,tower_http=info"));

    tracing_subscriber::registry()
        .with(env_filter)
        .with(tracing_subscriber::fmt::layer())
        .init();
}

fn read_bind_addr() -> Result<SocketAddr, Box<dyn Error>> {
    let host = env::var("HOST").unwrap_or_else(|_| "0.0.0.0".to_string());
    let port = match env::var("PORT") {
        Ok(raw) => raw.parse::<u16>()?,
        Err(_) => 8080,
    };
    let ip = host.parse::<IpAddr>()?;

    Ok(SocketAddr::from((ip, port)))
}

async fn shutdown_signal() {
    if let Err(err) = tokio::signal::ctrl_c().await {
        error!("failed to install shutdown signal handler: {}", err);
    }
}
