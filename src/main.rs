use axum::{
    extract::{Form, Path, Query, State},
    http::{header::SET_COOKIE, StatusCode},
    middleware,
    response::{IntoResponse, Redirect, Response},
    routing::{get, post},
    Json, Router,
};
use chrono::{Duration, Utc};
use serde::{Deserialize, Serialize};
use tokio::net::TcpListener;
use tower_http::{
    services::ServeDir,
    trace::{DefaultMakeSpan, DefaultOnFailure, DefaultOnRequest, DefaultOnResponse, TraceLayer},
};
use tracing::{error, info, Level};
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt, EnvFilter};

mod auth;
mod category_store;
mod config;
mod db;
mod error;
mod models;
mod password;
mod profile_store;
mod session_store;
mod templates;
mod thread_store;

use auth::{
    build_session_cookie, clear_session_cookie, signed_session_id_from_headers, CsrfToken, MaybeUser,
    RequireAdmin, RequireUser,
};
use category_store::{CategoryStore, CreateCategoryInput, UpdateCategoryInput};
use config::Config;
use db::Db;
use error::AppError;
use models::category::Category;
use models::user::User;
use password::{hash_password, verify_password};
use profile_store::ProfileStore;
use session_store::SessionStore;
use templates::{
    render, AdminCategoriesTemplate, AdminCategoryFormValues, AdminCategoryRow, CategoryHeader,
    CategoryTemplate, CategoryThreadRow, EditPostContext, EditPostFormValues, EditPostTemplate,
    HomeCategoryCard, HomeTemplate, LoginTemplate, NewThreadFormValues, NewThreadTemplate,
    ProfileHeader, ProfilePostRow, ProfileTemplate, RegisterTemplate, ThreadHeader, ThreadPostRow,
    ThreadTemplate,
};
use thread_store::{CreateThreadInput, ThreadStore};

#[derive(Clone)]
pub(crate) struct AppState {
    pub(crate) bind_addr: std::net::SocketAddr,
    pub(crate) db: Db,
    pub(crate) categories: CategoryStore,
    pub(crate) edit_window_minutes: u64,
    pub(crate) profiles: ProfileStore,
    pub(crate) sessions: SessionStore,
    pub(crate) session_secret: String,
    pub(crate) threads: ThreadStore,
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

#[derive(Debug, Default, Clone, Deserialize)]
struct AdminCategoryForm {
    name: String,
    slug: String,
    description: String,
    position: i32,
}

#[derive(Debug, Default, Clone, Deserialize)]
struct ReorderCategoryForm {
    position: i32,
}

#[derive(Debug, Default, Clone, Deserialize)]
struct PageQuery {
    page: Option<i64>,
}

#[derive(Debug, Default, Clone, Deserialize)]
struct NewThreadForm {
    title: String,
    body: String,
}

#[derive(Debug, Default, Clone, Deserialize)]
struct ReplyForm {
    body: String,
    page: Option<i64>,
}

#[derive(Debug, Default, Clone, Deserialize)]
struct EditPostForm {
    body: String,
}

#[tokio::main]
async fn main() -> Result<(), AppError> {
    init_tracing();

    let config = Config::from_env()?;
    let bind_addr = config.bind_addr;
    let db = Db::connect(&config).await?;
    db.run_migrations().await?;
    let categories = CategoryStore::new(db.pool());
    let profiles = ProfileStore::new(db.pool());
    let sessions = SessionStore::new(db.pool());
    let threads = ThreadStore::new(db.pool());

    let state = AppState {
        bind_addr,
        db,
        categories,
        edit_window_minutes: config.edit_window_minutes,
        profiles,
        sessions,
        session_secret: config.session_secret.clone(),
        threads,
    };

    let app = Router::new()
        .route("/", get(root))
        .route("/c/:slug", get(category_page))
        .route("/c/:slug/t/:thread_key", get(legacy_thread_page))
        .route("/c/:slug/new", get(new_thread_form).post(create_thread))
        .route("/p/:id/delete", post(delete_post))
        .route("/p/:id/edit", get(edit_post_form).post(update_post))
        .route("/t/:id/reply", post(reply_to_thread))
        .route("/t/:thread_key", get(thread_page))
        .route("/u/:username", get(public_profile))
        .route("/admin/categories", get(admin_categories))
        .route("/admin/categories/create", post(create_category))
        .route("/admin/categories/:id/update", post(update_category))
        .route("/admin/categories/:id/delete", post(delete_category))
        .route("/admin/categories/:id/reorder", post(reorder_category))
        .route("/register", get(register_form).post(register))
        .route("/login", get(login_form).post(login))
        .route("/logout", axum::routing::post(logout))
        .route("/health", get(health))
        .nest_service("/static", ServeDir::new("static"))
        .layer(middleware::from_fn_with_state(
            state.clone(),
            auth::csrf_verification_middleware,
        ))
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

async fn register_form(csrf_token: CsrfToken) -> Result<impl IntoResponse, AppError> {
    render_register(RegistrationForm::default(), None, csrf_token.0)
}

async fn login_form(csrf_token: CsrfToken) -> Result<impl IntoResponse, AppError> {
    render_login(LoginForm::default(), None, csrf_token.0)
}

async fn register(
    State(state): State<AppState>,
    csrf_token: CsrfToken,
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
            csrf_token.0.clone(),
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
            csrf_token.0.clone(),
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
                csrf_token.0.clone(),
            );
        }
        Err(err) => return Err(err.into()),
    };

    let session = state
        .sessions
        .create(Some(user.id), Utc::now() + Duration::days(30))
        .await?;
    let cookie_value =
        build_session_cookie(session.id, &state.session_secret, 30 * 24 * 60 * 60).map_err(AppError::from)?;

    Ok(([(SET_COOKIE, cookie_value)], Redirect::to("/")).into_response())
}

async fn login(
    State(state): State<AppState>,
    csrf_token: CsrfToken,
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
            csrf_token.0.clone(),
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
            csrf_token.0.clone(),
        );
    };

    if !verify_password(&password, &user.password_hash)? {
        return render_login_response(
            normalized_form,
            Some("Invalid username or password.".to_string()),
            StatusCode::UNPROCESSABLE_ENTITY,
            csrf_token.0.clone(),
        );
    }

    let session = state
        .sessions
        .create(Some(user.id), Utc::now() + Duration::days(30))
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

async fn root(
    State(state): State<AppState>,
    _maybe_user: MaybeUser,
    csrf_token: CsrfToken,
) -> Result<impl IntoResponse, AppError> {
    let categories = state
        .categories
        .list()
        .await?
        .into_iter()
        .map(home_category_card)
        .collect();

    render(HomeTemplate {
        categories,
        csrf_token: csrf_token.0,
    })
}

async fn category_page(
    State(state): State<AppState>,
    Path(slug): Path<String>,
    Query(query): Query<PageQuery>,
    _maybe_user: MaybeUser,
    csrf_token: CsrfToken,
) -> Result<Response, AppError> {
    let Some(category) = state.categories.get_by_slug(&slug).await? else {
        return Ok(StatusCode::NOT_FOUND.into_response());
    };

    let page = query.page.unwrap_or(1).max(1);
    let listing = state.threads.list_by_category(category.id, page, 20).await?;
    let html = render(CategoryTemplate {
        category: CategoryHeader {
            name: category.name,
            slug: category.slug,
            description: category.description,
        },
        threads: listing.threads.into_iter().map(category_thread_row).collect(),
        total_threads: listing.total_threads,
        current_page: listing.current_page,
        total_pages: listing.total_pages,
        prev_page: (listing.current_page > 1).then_some(listing.current_page - 1),
        next_page: (listing.current_page < listing.total_pages).then_some(listing.current_page + 1),
        csrf_token: csrf_token.0,
    })?;

    Ok(html.into_response())
}

async fn new_thread_form(
    State(state): State<AppState>,
    Path(slug): Path<String>,
    _user: RequireUser,
    csrf_token: CsrfToken,
) -> Result<Response, AppError> {
    let Some(category) = state.categories.get_by_slug(&slug).await? else {
        return Ok(StatusCode::NOT_FOUND.into_response());
    };

    render_new_thread(
        &category,
        NewThreadFormValues::default(),
        None,
        csrf_token.0,
        StatusCode::OK,
    )
}

async fn thread_page(
    State(state): State<AppState>,
    Path(thread_key): Path<String>,
    Query(query): Query<PageQuery>,
    maybe_user: MaybeUser,
    csrf_token: CsrfToken,
) -> Result<Response, AppError> {
    let Some(thread_id) = parse_thread_key(&thread_key) else {
        return Ok(StatusCode::NOT_FOUND.into_response());
    };

    let Some(thread) = state.threads.get_thread_detail(thread_id).await? else {
        return Ok(StatusCode::NOT_FOUND.into_response());
    };

    let canonical_path = thread_path(&thread.slug, thread.id);
    if thread_key != format!("{}-{}", thread.id, thread.slug) {
        return Ok(Redirect::to(&canonical_path).into_response());
    }

    render_thread_page(
        &state,
        thread,
        query.page.unwrap_or(1).max(1),
        maybe_user.0.as_ref().map(|user| user.user.id),
        None,
        String::new(),
        csrf_token.0,
        StatusCode::OK,
    )
    .await
}

async fn legacy_thread_page(
    State(state): State<AppState>,
    Path((category_slug, thread_key)): Path<(String, String)>,
) -> Result<Response, AppError> {
    let Some(thread_id) = parse_thread_key(&thread_key) else {
        return Ok(StatusCode::NOT_FOUND.into_response());
    };

    let Some(thread) = state.threads.get_thread_detail(thread_id).await? else {
        return Ok(StatusCode::NOT_FOUND.into_response());
    };

    if thread.category_slug != category_slug {
        return Ok(StatusCode::NOT_FOUND.into_response());
    }

    Ok(Redirect::to(&thread_path(&thread.slug, thread.id)).into_response())
}

async fn create_thread(
    State(state): State<AppState>,
    Path(slug): Path<String>,
    user: RequireUser,
    csrf_token: CsrfToken,
    Form(form): Form<NewThreadForm>,
) -> Result<Response, AppError> {
    let Some(category) = state.categories.get_by_slug(&slug).await? else {
        return Ok(StatusCode::NOT_FOUND.into_response());
    };

    let normalized_form = normalize_new_thread_form(form);

    if let Err(message) = validate_new_thread_form(&normalized_form) {
        return render_new_thread(
            &category,
            NewThreadFormValues {
                title: normalized_form.title.clone(),
                body: normalized_form.body.clone(),
            },
            Some(message),
            csrf_token.0,
            StatusCode::UNPROCESSABLE_ENTITY,
        );
    }

    let thread = state
        .threads
        .create_thread_with_initial_post(&CreateThreadInput {
            category_id: category.id,
            author_id: user.0.user.id,
            title: normalized_form.title.clone(),
            slug: slugify(&normalized_form.title),
            body: normalized_form.body.clone(),
        })
        .await?;

    Ok(Redirect::to(&thread_path(&thread.slug, thread.id)).into_response())
}

async fn reply_to_thread(
    State(state): State<AppState>,
    Path(id): Path<i64>,
    user: RequireUser,
    csrf_token: CsrfToken,
    Form(form): Form<ReplyForm>,
) -> Result<Response, AppError> {
    let Some(thread) = state.threads.get_thread_detail(id).await? else {
        return Ok(StatusCode::NOT_FOUND.into_response());
    };

    let reply_page = form.page.unwrap_or(1).max(1);
    let body = form.body.trim().to_string();

    if thread.is_locked {
        return render_thread_page(
            &state,
            thread,
            reply_page,
            Some(user.0.user.id),
            Some("This thread is locked. New replies are disabled.".to_string()),
            body,
            csrf_token.0,
            StatusCode::LOCKED,
        )
        .await;
    }

    if body.is_empty() {
        return render_thread_page(
            &state,
            thread,
            reply_page,
            Some(user.0.user.id),
            Some("Reply body is required.".to_string()),
            body,
            csrf_token.0,
            StatusCode::UNPROCESSABLE_ENTITY,
        )
        .await;
    }

    let reply = state
        .threads
        .create_reply(id, user.0.user.id, &body)
        .await?;
    let last_page = ((reply.total_posts - 1) / 20) + 1;
    let redirect_target = format!(
        "{}?page={}#post-{}",
        thread_path(&thread.slug, thread.id),
        last_page,
        reply.post_id
    );

    Ok(Redirect::to(&redirect_target).into_response())
}

async fn edit_post_form(
    State(state): State<AppState>,
    Path(id): Path<i64>,
    user: RequireUser,
    csrf_token: CsrfToken,
) -> Result<Response, AppError> {
    let Some(post) = state.threads.get_post_detail(id).await? else {
        return Ok(StatusCode::NOT_FOUND.into_response());
    };

    if !can_edit_post(&post, user.0.user.id, state.edit_window_minutes) {
        return Ok(StatusCode::FORBIDDEN.into_response());
    }

    render_edit_post(
        &post,
        EditPostFormValues {
            body: post.body.clone(),
        },
        None,
        csrf_token.0,
        StatusCode::OK,
    )
}

async fn update_post(
    State(state): State<AppState>,
    Path(id): Path<i64>,
    user: RequireUser,
    csrf_token: CsrfToken,
    Form(form): Form<EditPostForm>,
) -> Result<Response, AppError> {
    let Some(post) = state.threads.get_post_detail(id).await? else {
        return Ok(StatusCode::NOT_FOUND.into_response());
    };

    if !can_edit_post(&post, user.0.user.id, state.edit_window_minutes) {
        return Ok(StatusCode::FORBIDDEN.into_response());
    }

    let body = form.body.trim().to_string();
    if body.is_empty() {
        return render_edit_post(
            &post,
            EditPostFormValues { body },
            Some("Post body is required.".to_string()),
            csrf_token.0,
            StatusCode::UNPROCESSABLE_ENTITY,
        );
    }

    if !state.threads.update_post_body(id, &body).await? {
        return Ok(StatusCode::NOT_FOUND.into_response());
    }

    let page = state.threads.page_for_post(id, 20).await?.unwrap_or(1);
    let redirect_target = format!(
        "/t/{}-{}?page={}#post-{}",
        post.thread_id, post.thread_slug, page, post.id
    );

    Ok(Redirect::to(&redirect_target).into_response())
}

async fn delete_post(
    State(state): State<AppState>,
    Path(id): Path<i64>,
    user: RequireUser,
) -> Result<Response, AppError> {
    let Some(post) = state.threads.get_post_detail(id).await? else {
        return Ok(StatusCode::NOT_FOUND.into_response());
    };

    if !can_edit_post(&post, user.0.user.id, state.edit_window_minutes) {
        return Ok(StatusCode::FORBIDDEN.into_response());
    }

    if !state.threads.soft_delete_post(id).await? {
        return Ok(StatusCode::NOT_FOUND.into_response());
    }

    let page = state.threads.page_for_post(id, 20).await?.unwrap_or(1);
    let redirect_target = format!(
        "/t/{}-{}?page={}#post-{}",
        post.thread_id, post.thread_slug, page, post.id
    );

    Ok(Redirect::to(&redirect_target).into_response())
}

async fn public_profile(
    State(state): State<AppState>,
    Path(username): Path<String>,
    _maybe_user: MaybeUser,
    csrf_token: CsrfToken,
) -> Result<Response, AppError> {
    let username = username.trim().to_lowercase();
    let Some(profile) = state.profiles.get_public_profile(&username).await? else {
        return Ok(StatusCode::NOT_FOUND.into_response());
    };
    let recent_posts = state.profiles.recent_posts(profile.id, 10).await?;
    let html = render(ProfileTemplate {
        profile: ProfileHeader {
            username: profile.username,
            display_name: profile.display_name,
            bio: profile.bio,
            created_at: profile.created_at,
            post_count: profile.post_count,
        },
        recent_posts: recent_posts.into_iter().map(profile_post_row).collect(),
        csrf_token: csrf_token.0,
    })?;

    Ok(html.into_response())
}

async fn admin_categories(
    State(state): State<AppState>,
    _admin: RequireAdmin,
    csrf_token: CsrfToken,
) -> Result<impl IntoResponse, AppError> {
    render_admin_categories(
        &state,
        AdminCategoryFormValues::default(),
        None,
        csrf_token.0,
        StatusCode::OK,
    )
    .await
}

async fn create_category(
    State(state): State<AppState>,
    _admin: RequireAdmin,
    csrf_token: CsrfToken,
    Form(form): Form<AdminCategoryForm>,
) -> Result<Response, AppError> {
    let normalized = normalize_admin_category_form(form);

    if let Err(message) = validate_admin_category_form(&normalized) {
        return render_admin_categories(
            &state,
            admin_form_values(&normalized),
            Some(message),
            csrf_token.0,
            StatusCode::UNPROCESSABLE_ENTITY,
        )
        .await;
    }

    let input = CreateCategoryInput {
        name: normalized.name.clone(),
        slug: normalized.slug.clone(),
        description: normalized.description.clone(),
        position: normalized.position,
    };

    match state.categories.create(&input).await {
        Ok(_) => Ok(Redirect::to("/admin/categories").into_response()),
        Err(sqlx::Error::Database(db_error)) if db_error.constraint() == Some("categories_slug_key") => {
            render_admin_categories(
                &state,
                admin_form_values(&normalized),
                Some("That category slug is already in use.".to_string()),
                csrf_token.0,
                StatusCode::UNPROCESSABLE_ENTITY,
            )
            .await
        }
        Err(err) => Err(err.into()),
    }
}

async fn update_category(
    State(state): State<AppState>,
    _admin: RequireAdmin,
    csrf_token: CsrfToken,
    Path(id): Path<i64>,
    Form(form): Form<AdminCategoryForm>,
) -> Result<Response, AppError> {
    let normalized = normalize_admin_category_form(form);

    if let Err(message) = validate_admin_category_form(&normalized) {
        return render_admin_categories(
            &state,
            AdminCategoryFormValues::default(),
            Some(message),
            csrf_token.0,
            StatusCode::UNPROCESSABLE_ENTITY,
        )
        .await;
    }

    let input = UpdateCategoryInput {
        name: normalized.name,
        slug: normalized.slug,
        description: normalized.description,
        position: normalized.position,
    };

    match state.categories.update(id, &input).await {
        Ok(Some(_)) => Ok(Redirect::to("/admin/categories").into_response()),
        Ok(None) => Ok(StatusCode::NOT_FOUND.into_response()),
        Err(sqlx::Error::Database(db_error)) if db_error.constraint() == Some("categories_slug_key") => {
            render_admin_categories(
                &state,
                AdminCategoryFormValues::default(),
                Some("That category slug is already in use.".to_string()),
                csrf_token.0,
                StatusCode::UNPROCESSABLE_ENTITY,
            )
            .await
        }
        Err(err) => Err(err.into()),
    }
}

async fn delete_category(
    State(state): State<AppState>,
    _admin: RequireAdmin,
    Path(id): Path<i64>,
) -> Result<Response, AppError> {
    let deleted = state.categories.delete(id).await?;

    if deleted {
        Ok(Redirect::to("/admin/categories").into_response())
    } else {
        Ok(StatusCode::NOT_FOUND.into_response())
    }
}

async fn reorder_category(
    State(state): State<AppState>,
    _admin: RequireAdmin,
    csrf_token: CsrfToken,
    Path(id): Path<i64>,
    Form(form): Form<ReorderCategoryForm>,
) -> Result<Response, AppError> {
    if form.position < 0 {
        return render_admin_categories(
            &state,
            AdminCategoryFormValues::default(),
            Some("Category position must be zero or greater.".to_string()),
            csrf_token.0,
            StatusCode::UNPROCESSABLE_ENTITY,
        )
        .await;
    }

    match state.categories.update_position(id, form.position).await? {
        Some(_) => Ok(Redirect::to("/admin/categories").into_response()),
        None => Ok(StatusCode::NOT_FOUND.into_response()),
    }
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

fn render_register(
    form: RegistrationForm,
    error_message: Option<String>,
    csrf_token: Option<String>,
) -> Result<Response, AppError> {
    render_register_response(form, error_message, StatusCode::OK, csrf_token)
}

fn render_login(
    form: LoginForm,
    error_message: Option<String>,
    csrf_token: Option<String>,
) -> Result<Response, AppError> {
    render_login_response(form, error_message, StatusCode::OK, csrf_token)
}

fn render_register_response(
    form: RegistrationForm,
    error_message: Option<String>,
    status: StatusCode,
    csrf_token: Option<String>,
) -> Result<Response, AppError> {
    let html = render(RegisterTemplate {
        username: form.username,
        display_name: form.display_name,
        bio: form.bio,
        error_message,
        csrf_token,
    })?;

    Ok((status, html).into_response())
}

fn render_login_response(
    form: LoginForm,
    error_message: Option<String>,
    status: StatusCode,
    csrf_token: Option<String>,
) -> Result<Response, AppError> {
    let html = render(LoginTemplate {
        username: form.username,
        error_message,
        csrf_token,
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

fn home_category_card(category: Category) -> HomeCategoryCard {
    HomeCategoryCard {
        name: category.name,
        slug: category.slug,
        description: category.description,
        position: category.position,
        thread_count: 0,
        most_recent_thread: None,
    }
}

fn category_thread_row(thread: thread_store::ThreadListItem) -> CategoryThreadRow {
    CategoryThreadRow {
        id: thread.id,
        title: thread.title,
        slug: thread.slug,
        author_username: thread.author_username,
        reply_count: thread.reply_count,
        last_activity_at: thread.last_activity_at,
        is_pinned: thread.is_pinned,
        is_locked: thread.is_locked,
    }
}

fn profile_post_row(post: profile_store::PublicProfilePost) -> ProfilePostRow {
    ProfilePostRow {
        post_id: post.post_id,
        thread_id: post.thread_id,
        thread_slug: post.thread_slug,
        thread_title: post.thread_title,
        body: post.body,
        created_at: post.created_at,
    }
}

fn thread_post_row(
    post: thread_store::ThreadPostItem,
    current_user_id: Option<i64>,
    edit_window_minutes: u64,
) -> ThreadPostRow {
    let can_edit = can_edit_post_item(
        post.author_id,
        post.created_at,
        current_user_id,
        edit_window_minutes,
    );

    ThreadPostRow {
        id: post.id,
        author_username: post.author_username,
        body: post.body,
        created_at: post.created_at,
        updated_at: post.updated_at,
        edit_url: if can_edit {
            Some(format!("/p/{}/edit", post.id))
        } else {
            None
        },
        delete_action: if can_edit {
            Some(format!("/p/{}/delete", post.id))
        } else {
            None
        },
        is_deleted: post.deleted_at.is_some(),
    }
}

fn normalize_new_thread_form(form: NewThreadForm) -> NewThreadForm {
    NewThreadForm {
        title: form.title.trim().to_string(),
        body: form.body.trim().to_string(),
    }
}

fn validate_new_thread_form(form: &NewThreadForm) -> Result<(), String> {
    if !(3..=120).contains(&form.title.len()) {
        return Err("Thread title must be between 3 and 120 characters.".to_string());
    }

    if form.body.is_empty() {
        return Err("The first post body is required.".to_string());
    }

    Ok(())
}

fn slugify(title: &str) -> String {
    let mut slug = String::with_capacity(title.len());
    let mut previous_was_dash = false;

    for ch in title.chars().flat_map(|ch| ch.to_lowercase()) {
        if ch.is_ascii_alphanumeric() {
            slug.push(ch);
            previous_was_dash = false;
        } else if !previous_was_dash {
            slug.push('-');
            previous_was_dash = true;
        }
    }

    let slug = slug.trim_matches('-').to_string();

    if slug.is_empty() {
        "thread".to_string()
    } else {
        slug
    }
}

fn thread_path(thread_slug: &str, thread_id: i64) -> String {
    format!("/t/{thread_id}-{thread_slug}")
}

fn parse_thread_key(thread_key: &str) -> Option<i64> {
    let (id, _slug) = thread_key.split_once('-')?;
    id.parse().ok()
}

fn render_new_thread(
    category: &Category,
    form: NewThreadFormValues,
    error_message: Option<String>,
    csrf_token: Option<String>,
    status: StatusCode,
) -> Result<Response, AppError> {
    let html = render(NewThreadTemplate {
        category: CategoryHeader {
            name: category.name.clone(),
            slug: category.slug.clone(),
            description: category.description.clone(),
        },
        form,
        error_message,
        csrf_token,
    })?;

    Ok((status, html).into_response())
}

fn render_edit_post(
    post: &thread_store::PostDetail,
    form: EditPostFormValues,
    error_message: Option<String>,
    csrf_token: Option<String>,
    status: StatusCode,
) -> Result<Response, AppError> {
    let html = render(EditPostTemplate {
        post: EditPostContext {
            id: post.id,
            thread_id: post.thread_id,
            thread_title: post.thread_title.clone(),
            thread_slug: post.thread_slug.clone(),
            author_username: post.author_username.clone(),
            created_at: post.created_at,
            updated_at: post.updated_at,
        },
        form,
        error_message,
        csrf_token,
    })?;

    Ok((status, html).into_response())
}

async fn render_thread_page(
    state: &AppState,
    thread: thread_store::ThreadDetail,
    requested_page: i64,
    current_user_id: Option<i64>,
    reply_error_message: Option<String>,
    reply_body: String,
    csrf_token: Option<String>,
    status: StatusCode,
) -> Result<Response, AppError> {
    let posts_page = state
        .threads
        .list_posts_for_thread(thread.id, requested_page, 20)
        .await?;
    let can_reply = current_user_id.is_some() && !thread.is_locked;
    let html = render(ThreadTemplate {
        thread: ThreadHeader {
            id: thread.id,
            title: thread.title,
            slug: thread.slug,
            category_name: thread.category_name,
            category_slug: thread.category_slug,
            author_username: thread.author_username,
            created_at: thread.created_at,
            last_activity_at: thread.last_activity_at,
            is_pinned: thread.is_pinned,
            is_locked: thread.is_locked,
        },
        posts: posts_page
            .posts
            .into_iter()
            .map(|post| thread_post_row(post, current_user_id, state.edit_window_minutes))
            .collect(),
        total_posts: posts_page.total_posts,
        current_page: posts_page.current_page,
        total_pages: posts_page.total_pages,
        prev_page: (posts_page.current_page > 1).then_some(posts_page.current_page - 1),
        next_page: (posts_page.current_page < posts_page.total_pages)
            .then_some(posts_page.current_page + 1),
        can_reply,
        reply_form_action: format!("/t/{}/reply", thread.id),
        reply_error_message,
        reply_body,
        csrf_token,
    })?;

    Ok((status, html).into_response())
}

async fn render_admin_categories(
    state: &AppState,
    create_form: AdminCategoryFormValues,
    error_message: Option<String>,
    csrf_token: Option<String>,
    status: StatusCode,
) -> Result<Response, AppError> {
    let categories = state
        .categories
        .list()
        .await?
        .into_iter()
        .map(admin_category_row)
        .collect();
    let html = render(AdminCategoriesTemplate {
        categories,
        create_form,
        error_message,
        csrf_token,
    })?;

    Ok((status, html).into_response())
}

fn admin_category_row(category: Category) -> AdminCategoryRow {
    AdminCategoryRow {
        id: category.id,
        name: category.name,
        slug: category.slug,
        description: category.description,
        position: category.position,
    }
}

fn can_edit_post(post: &thread_store::PostDetail, user_id: i64, edit_window_minutes: u64) -> bool {
    can_edit_post_item(
        post.author_id,
        post.created_at,
        Some(user_id),
        edit_window_minutes,
    )
}

fn can_edit_post_item(
    author_id: i64,
    created_at: chrono::DateTime<Utc>,
    current_user_id: Option<i64>,
    edit_window_minutes: u64,
) -> bool {
    match current_user_id {
        Some(user_id) if user_id == author_id => {
            Utc::now() <= created_at + Duration::minutes(edit_window_minutes as i64)
        }
        _ => false,
    }
}

fn normalize_admin_category_form(form: AdminCategoryForm) -> AdminCategoryForm {
    AdminCategoryForm {
        name: form.name.trim().to_string(),
        slug: form.slug.trim().to_lowercase(),
        description: form.description.trim().to_string(),
        position: form.position,
    }
}

fn admin_form_values(form: &AdminCategoryForm) -> AdminCategoryFormValues {
    AdminCategoryFormValues {
        name: form.name.clone(),
        slug: form.slug.clone(),
        description: form.description.clone(),
        position: form.position,
    }
}

fn validate_admin_category_form(form: &AdminCategoryForm) -> Result<(), String> {
    if form.name.is_empty() {
        return Err("Category name is required.".to_string());
    }

    if form.slug.is_empty() {
        return Err("Category slug is required.".to_string());
    }

    if !form
        .slug
        .chars()
        .all(|ch| ch.is_ascii_lowercase() || ch.is_ascii_digit() || ch == '-')
    {
        return Err("Category slug may only contain lowercase letters, numbers, and hyphens.".to_string());
    }

    if form.description.is_empty() {
        return Err("Category description is required.".to_string());
    }

    if form.position < 0 {
        return Err("Category position must be zero or greater.".to_string());
    }

    Ok(())
}
