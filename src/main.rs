use std::net::SocketAddr;

use axum::{
    extract::{Form, State},
    http::{header::SET_COOKIE, HeaderValue, StatusCode},
    response::{IntoResponse, Redirect, Response},
    routing::get,
    Json, Router,
};
use chrono::{Duration, Utc};
use serde::Serialize;
use tokio::net::TcpListener;
use tower_http::{
    services::ServeDir,
    trace::{DefaultMakeSpan, DefaultOnFailure, DefaultOnRequest, DefaultOnResponse, TraceLayer},
};
use tracing::{error, info, Level};
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt, EnvFilter};

mod config;
mod db;
mod error;
mod models;
mod password;
mod session_store;
mod templates;

use config::Config;
use db::Db;
use error::AppError;
use models::user::User;
use password::hash_password;
use session_store::SessionStore;
use templates::{render, HomeTemplate, RegisterTemplate};

#[derive(Clone)]
struct AppState {
    bind_addr: SocketAddr,
    db: Db,
    sessions: SessionStore,
}

#[derive(Serialize)]
struct HealthResponse {
    status: &'static str,
    bind_addr: String,
    database_connected: bool,
}

#[derive(Debug, Default, Clone, Serialize, serde::Deserialize)]
struct RegistrationForm {
    username: String,
    display_name: String,
    bio: String,
    password: String,
}

#[tokio::main]
async fn main() -> Result<(), AppError> {
    init_tracing();

    let config = Config::from_env()?;
    let bind_addr = config.bind_addr;
    let db = Db::connect(&config).await?;
    db.run_migrations().await?;
    let sessions = SessionStore::new(db.pool());

    let state = AppState {
        bind_addr,
        db,
        sessions,
    };

    let app = Router::new()
        .route("/", get(root))
        .route("/register", get(register_form).post(register))
        .route("/health", get(health))
        .nest_service("/static", ServeDir::new("static"))
        .layer(
            TraceLayer::new_for_http()
                .make_span_with(DefaultMakeSpan::new().level(Level::INFO))
                .on_request(DefaultOnRequest::new().level(Level::INFO))
                .on_response(DefaultOnResponse::new().level(Level::INFO))
                .on_failure(DefaultOnFailure::new().level(Level::ERROR)),
        )
        .with_state(state);

    let listener = TcpListener::bind(bind_addr).await?;
    info!("listening on {}", bind_addr);
    info!(
        edit_window_minutes = config.edit_window_minutes,
        database_configured = !config.database_url.is_empty(),
        session_secret_configured = !config.session_secret.is_empty(),
        "configuration loaded"
    );

    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal())
        .await?;

    Ok(())
}

async fn root() -> Result<impl IntoResponse, AppError> {
    render(HomeTemplate)
}

async fn register_form() -> Result<impl IntoResponse, AppError> {
    render_register(RegistrationForm::default(), None)
}

async fn register(
    State(state): State<AppState>,
    Form(form): Form<RegistrationForm>,
) -> Result<Response, AppError> {
    let username = form.username.trim().to_lowercase();
    let display_name = if form.display_name.trim().is_empty() {
        username.clone()
    } else {
        form.display_name.trim().to_string()
    };
    let bio = form.bio.trim().to_string();
    let password = form.password.clone();

    let normalized_form = RegistrationForm {
        username: username.clone(),
        display_name: display_name.clone(),
        bio: bio.clone(),
        password: String::new(),
    };

    if let Err(message) = validate_registration_form(&normalized_form, &password) {
        return render_register_response(
            normalized_form,
            Some(message),
            StatusCode::UNPROCESSABLE_ENTITY,
        );
    }

    let username_taken = sqlx::query_scalar::<_, bool>(
        r#"
        SELECT EXISTS(
            SELECT 1
            FROM users
            WHERE username = $1
        )
        "#,
    )
    .bind(&username)
    .fetch_one(state.db.pool())
    .await?;

    if username_taken {
        return render_register_response(
            normalized_form,
            Some("That username is already taken.".to_string()),
            StatusCode::UNPROCESSABLE_ENTITY,
        );
    }

    let password_hash = hash_password(&password)?;
    let user = sqlx::query_as::<_, User>(
        r#"
        INSERT INTO users (username, password_hash, display_name, bio)
        VALUES ($1, $2, $3, $4)
        RETURNING id, username, password_hash, display_name, bio, role, created_at
        "#,
    )
    .bind(&username)
    .bind(&password_hash)
    .bind(&display_name)
    .bind(&bio)
    .fetch_one(state.db.pool())
    .await;

    let user = match user {
        Ok(user) => user,
        Err(sqlx::Error::Database(db_error))
            if db_error.constraint() == Some("users_username_key") =>
        {
            return render_register_response(
                normalized_form,
                Some("That username is already taken.".to_string()),
                StatusCode::UNPROCESSABLE_ENTITY,
            );
        }
        Err(err) => return Err(err.into()),
    };

    let session = state
        .sessions
        .create(user.id, Utc::now() + Duration::days(30))
        .await?;

    let cookie = format!(
        "session_id={}; Path=/; HttpOnly; SameSite=Lax; Max-Age={}",
        session.id,
        30 * 24 * 60 * 60
    );
    let cookie_value = HeaderValue::from_str(&cookie).map_err(|err| {
        AppError::Io(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            err.to_string(),
        ))
    })?;

    Ok(([(
        SET_COOKIE,
        cookie_value,
    )], Redirect::to("/"))
        .into_response())
}

async fn health(State(state): State<AppState>) -> impl IntoResponse {
    let payload = HealthResponse {
        status: "ok",
        bind_addr: state.bind_addr.to_string(),
        database_connected: !state.db.pool().is_closed(),
    };

    (StatusCode::OK, Json(payload))
}

fn init_tracing() {
    let env_filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new("miketang84_forum002=debug,tower_http=info"));

    tracing_subscriber::registry()
        .with(env_filter)
        .with(tracing_subscriber::fmt::layer())
        .init();
}

fn render_register(form: RegistrationForm, error_message: Option<String>) -> Result<Response, AppError> {
    render_register_response(form, error_message, StatusCode::OK)
}

fn render_register_response(
    form: RegistrationForm,
    error_message: Option<String>,
    status: StatusCode,
) -> Result<Response, AppError> {
    let html = render(RegisterTemplate {
        username: form.username,
        display_name: form.display_name,
        bio: form.bio,
        error_message,
    })?;

    Ok((status, html).into_response())
}

fn validate_registration_form(form: &RegistrationForm, password: &str) -> Result<(), String> {
    if !(3..=32).contains(&form.username.len()) {
        return Err("Username must be between 3 and 32 characters.".to_string());
    }

    if !form
        .username
        .chars()
        .all(|ch| ch.is_ascii_lowercase() || ch.is_ascii_digit() || ch == '_')
    {
        return Err(
            "Username may only contain lowercase letters, numbers, and underscores.".to_string(),
        );
    }

    if password.len() < 8 {
        return Err("Password must be at least 8 characters long.".to_string());
    }

    Ok(())
}

async fn shutdown_signal() {
    if let Err(err) = tokio::signal::ctrl_c().await {
        error!("failed to install shutdown signal handler: {}", err);
    }
}
