use axum::{
    extract::{Form, State},
    http::{header::SET_COOKIE, StatusCode},
    middleware,
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

mod auth;
mod config;
mod db;
mod error;
mod models;
mod password;
mod session_store;
mod templates;

use auth::{build_session_cookie, clear_session_cookie, signed_session_id_from_headers, MaybeUser};
use config::Config;
use db::Db;
use error::AppError;
use models::user::User;
use password::{hash_password, verify_password};
use session_store::SessionStore;
use templates::{render, HomeTemplate, LoginTemplate, RegisterTemplate};

#[derive(Clone)]
pub(crate) struct AppState {
    pub(crate) bind_addr: std::net::SocketAddr,
    pub(crate) db: Db,
    pub(crate) sessions: SessionStore,
    pub(crate) session_secret: String,
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

#[derive(Debug, Default, Clone, Serialize, serde::Deserialize)]
struct LoginForm {
    username: String,
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
        session_secret: config.session_secret.clone(),
    };

    let app = Router::new()
        .route("/", get(root))
        .route("/register", get(register_form).post(register))
        .route("/login", get(login_form).post(login))
        .route("/logout", axum::routing::post(logout))
        .route("/health", get(health))
        .nest_service("/static", ServeDir::new("static"))
        .layer(middleware::from_fn_with_state(
            state.clone(),
            auth::session_cookie_middleware,
        ))
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

async fn register_form() -> Result<impl IntoResponse, AppError> {
    render_register(RegistrationForm::default(), None)
}

async fn login_form() -> Result<impl IntoResponse, AppError> {
    render_login(LoginForm::default(), None)
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
    let cookie_value =
        build_session_cookie(session.id, &state.session_secret, 30 * 24 * 60 * 60).map_err(AppError::from)?;

    Ok(([(SET_COOKIE, cookie_value)], Redirect::to("/")).into_response())
}

async fn login(
    State(state): State<AppState>,
    Form(form): Form<LoginForm>,
) -> Result<Response, AppError> {
    let username = form.username.trim().to_lowercase();
    let password = form.password.clone();
    let normalized_form = LoginForm {
        username: username.clone(),
        password: String::new(),
    };

    if username.is_empty() || password.is_empty() {
        return render_login_response(
            normalized_form,
            Some("Username and password are required.".to_string()),
            StatusCode::UNPROCESSABLE_ENTITY,
        );
    }

    let user = sqlx::query_as::<_, User>(
        r#"
        SELECT id, username, password_hash, display_name, bio, role, created_at
        FROM users
        WHERE username = $1
        "#,
    )
    .bind(&username)
    .fetch_optional(state.db.pool())
    .await?;

    let Some(user) = user else {
        return render_login_response(
            normalized_form,
            Some("Invalid username or password.".to_string()),
            StatusCode::UNPROCESSABLE_ENTITY,
        );
    };

    if !verify_password(&password, &user.password_hash)? {
        return render_login_response(
            normalized_form,
            Some("Invalid username or password.".to_string()),
            StatusCode::UNPROCESSABLE_ENTITY,
        );
    }

    let session = state
        .sessions
        .create(user.id, Utc::now() + Duration::days(30))
        .await?;
    let cookie_value =
        build_session_cookie(session.id, &state.session_secret, 30 * 24 * 60 * 60).map_err(AppError::from)?;

    Ok(([(SET_COOKIE, cookie_value)], Redirect::to("/")).into_response())
}

async fn logout(State(state): State<AppState>, headers: axum::http::HeaderMap) -> Result<Response, AppError> {
    if let Some(session_id) = signed_session_id_from_headers(&headers, &state.session_secret) {
        let _ = state.sessions.delete(session_id).await?;
    }

    let cookie_value = clear_session_cookie().map_err(AppError::from)?;

    Ok(([(SET_COOKIE, cookie_value)], Redirect::to("/")).into_response())
}

async fn root(_maybe_user: MaybeUser) -> Result<impl IntoResponse, AppError> {
    render(HomeTemplate)
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

fn render_login(form: LoginForm, error_message: Option<String>) -> Result<Response, AppError> {
    render_login_response(form, error_message, StatusCode::OK)
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

fn render_login_response(
    form: LoginForm,
    error_message: Option<String>,
    status: StatusCode,
) -> Result<Response, AppError> {
    let html = render(LoginTemplate {
        username: form.username,
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
