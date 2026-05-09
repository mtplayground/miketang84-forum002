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
mod search_store;
mod session_store;
mod templates;
mod thread_store;

use auth::{
    build_session_cookie, clear_session_cookie, signed_session_id_from_headers, CsrfToken, MaybeUser,
    RequireAdmin, RequireModerator, RequireUser,
};
use category_store::{CategoryStore, CreateCategoryInput, UpdateCategoryInput};
use config::Config;
use db::Db;
use error::AppError;
use models::category::Category;
use models::user::{Role, User};
use password::{hash_password, verify_password};
use profile_store::ProfileStore;
use search_store::SearchStore;
use session_store::SessionStore;
use templates::{
    render, AdminCategoriesTemplate, AdminCategoryFormValues, AdminCategoryRow, AdminUserRow,
    AdminUsersTemplate, CategoryHeader, CategoryTemplate, CategoryThreadRow, EditPostContext,
    EditPostFormValues, EditPostTemplate, EditProfileContext, EditProfileFormValues,
    EditProfileTemplate, ErrorTemplate, HomeCategoryCard, HomeTemplate, LoginTemplate,
    NewThreadFormValues, NewThreadTemplate, ProfileHeader, ProfilePostRow, ProfileTemplate,
    RegisterTemplate, SearchResultRow, SearchTemplate, ThreadHeader, ThreadPostRow, ThreadTemplate,
};
use thread_store::{CreateThreadInput, ThreadStore};

#[derive(Clone)]
pub(crate) struct AppState {
    pub(crate) bind_addr: std::net::SocketAddr,
    pub(crate) db: Db,
    pub(crate) categories: CategoryStore,
    pub(crate) edit_window_minutes: u64,
    pub(crate) profiles: ProfileStore,
    pub(crate) search: SearchStore,
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
struct SearchQuery {
    q: Option<String>,
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

#[derive(Debug, Default, Clone, Deserialize)]
struct EditProfileForm {
    display_name: String,
    bio: String,
}

#[derive(Debug, Default, Clone, Deserialize)]
struct UpdateUserRoleForm {
    role: String,
}

#[tokio::main]
async fn main() -> Result<(), AppError> {
    init_tracing();

    let config = Config::from_env()?;
    let bind_addr = config.bind_addr;
    let session_secret_configured = !config.session_secret.is_empty();
    let db = Db::connect(&config).await?;
    db.run_migrations().await?;
    let state = build_state(bind_addr, db, config.edit_window_minutes, config.session_secret);
    let app = app_router(state);

    let listener = TcpListener::bind(bind_addr).await?;
    info!("listening on {}", bind_addr);
    info!(
        edit_window_minutes = config.edit_window_minutes,
        database_configured = !config.database_url.is_empty(),
        session_secret_configured,
        "configuration loaded"
    );

    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal())
        .await?;

    Ok(())
}

fn build_state(
    bind_addr: std::net::SocketAddr,
    db: Db,
    edit_window_minutes: u64,
    session_secret: String,
) -> AppState {
    let categories = CategoryStore::new(db.pool());
    let profiles = ProfileStore::new(db.pool());
    let search = SearchStore::new(db.pool());
    let sessions = SessionStore::new(db.pool());
    let threads = ThreadStore::new(db.pool());

    AppState {
        bind_addr,
        db,
        categories,
        edit_window_minutes,
        profiles,
        search,
        sessions,
        session_secret,
        threads,
    }
}

fn app_router(state: AppState) -> Router {
    Router::new()
        .route("/", get(root))
        .route("/c/:slug", get(category_page))
        .route("/c/:slug/t/:thread_key", get(legacy_thread_page))
        .route("/c/:slug/new", get(new_thread_form).post(create_thread))
        .route("/p/:id/delete", post(delete_post))
        .route("/p/:id/mod-delete", post(moderator_delete_post))
        .route("/p/:id/edit", get(edit_post_form).post(update_post))
        .route("/t/:id/lock", post(lock_thread))
        .route("/t/:id/mod-delete", post(moderator_delete_thread))
        .route("/t/:id/pin", post(pin_thread))
        .route("/t/:id/reply", post(reply_to_thread))
        .route("/t/:id/unlock", post(unlock_thread))
        .route("/t/:id/unpin", post(unpin_thread))
        .route("/t/:thread_key", get(thread_page))
        .route("/me/profile", get(edit_profile_form).post(update_profile))
        .route("/search", get(search_page))
        .route("/u/:username", get(public_profile))
        .route("/admin/categories", get(admin_categories))
        .route("/admin/categories/create", post(create_category))
        .route("/admin/categories/:id/update", post(update_category))
        .route("/admin/categories/:id/delete", post(delete_category))
        .route("/admin/categories/:id/reorder", post(reorder_category))
        .route("/admin/users", get(admin_users))
        .route("/admin/users/:id/role", post(update_user_role))
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
        .with_state(state)
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

    if thread.deleted_at.is_some() {
        return render_removed_thread(csrf_token.0, StatusCode::GONE);
    }

    render_thread_page(
        &state,
        thread,
        query.page.unwrap_or(1).max(1),
        maybe_user.0,
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

    if thread.deleted_at.is_some() {
        return render_removed_thread(csrf_token.0, StatusCode::GONE);
    }

    let reply_page = form.page.unwrap_or(1).max(1);
    let body = form.body.trim().to_string();

    if thread.is_locked {
        return render_thread_page(
            &state,
            thread,
            reply_page,
            Some(user.0.clone()),
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
            Some(user.0.clone()),
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

async fn lock_thread(
    State(state): State<AppState>,
    Path(id): Path<i64>,
    _moderator: RequireModerator,
) -> Result<Response, AppError> {
    toggle_thread_lock(&state, id, true).await
}

async fn unlock_thread(
    State(state): State<AppState>,
    Path(id): Path<i64>,
    _moderator: RequireModerator,
) -> Result<Response, AppError> {
    toggle_thread_lock(&state, id, false).await
}

async fn pin_thread(
    State(state): State<AppState>,
    Path(id): Path<i64>,
    _moderator: RequireModerator,
) -> Result<Response, AppError> {
    toggle_thread_pin(&state, id, true).await
}

async fn unpin_thread(
    State(state): State<AppState>,
    Path(id): Path<i64>,
    _moderator: RequireModerator,
) -> Result<Response, AppError> {
    toggle_thread_pin(&state, id, false).await
}

async fn moderator_delete_thread(
    State(state): State<AppState>,
    Path(id): Path<i64>,
    _moderator: RequireModerator,
) -> Result<Response, AppError> {
    let Some(thread) = state.threads.get_thread_detail(id).await? else {
        return Ok(StatusCode::NOT_FOUND.into_response());
    };

    if !state.threads.soft_delete_thread(id).await? {
        return Ok(StatusCode::NOT_FOUND.into_response());
    }

    Ok(Redirect::to(&thread_path(&thread.slug, thread.id)).into_response())
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

async fn moderator_delete_post(
    State(state): State<AppState>,
    Path(id): Path<i64>,
    moderator: RequireModerator,
) -> Result<Response, AppError> {
    let Some(post) = state.threads.get_post_detail(id).await? else {
        return Ok(StatusCode::NOT_FOUND.into_response());
    };

    if !state
        .threads
        .moderator_soft_delete_post(id, moderator.0.user.id)
        .await?
    {
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
    maybe_user: MaybeUser,
    csrf_token: CsrfToken,
) -> Result<Response, AppError> {
    let username = username.trim().to_lowercase();
    let Some(profile) = state.profiles.get_public_profile(&username).await? else {
        return Ok(StatusCode::NOT_FOUND.into_response());
    };
    let recent_posts = state.profiles.recent_posts(profile.id, 10).await?;
    let can_edit = maybe_user
        .0
        .as_ref()
        .map(|current_user| current_user.user.id == profile.id)
        .unwrap_or(false);
    let html = render(ProfileTemplate {
        profile: ProfileHeader {
            username: profile.username,
            display_name: profile.display_name,
            bio: profile.bio,
            created_at: profile.created_at,
            post_count: profile.post_count,
        },
        recent_posts: recent_posts.into_iter().map(profile_post_row).collect(),
        can_edit,
        edit_profile_url: "/me/profile".to_string(),
        csrf_token: csrf_token.0,
    })?;

    Ok(html.into_response())
}

async fn search_page(
    State(state): State<AppState>,
    Query(query): Query<SearchQuery>,
    _maybe_user: MaybeUser,
    csrf_token: CsrfToken,
) -> Result<Response, AppError> {
    let query_text = query.q.unwrap_or_default().trim().to_string();

    if query_text.is_empty() {
        let html = render(SearchTemplate {
            query: String::new(),
            results: Vec::new(),
            total_results: 0,
            current_page: 1,
            total_pages: 1,
            prev_page: None,
            next_page: None,
            csrf_token: csrf_token.0,
        })?;

        return Ok(html.into_response());
    }

    let results_page = state
        .search
        .search(&query_text, query.page.unwrap_or(1).max(1), 20)
        .await?;
    let html = render(SearchTemplate {
        query: query_text,
        results: results_page
            .results
            .into_iter()
            .map(search_result_row)
            .collect(),
        total_results: results_page.total_results,
        current_page: results_page.current_page,
        total_pages: results_page.total_pages,
        prev_page: (results_page.current_page > 1).then_some(results_page.current_page - 1),
        next_page: (results_page.current_page < results_page.total_pages)
            .then_some(results_page.current_page + 1),
        csrf_token: csrf_token.0,
    })?;

    Ok(html.into_response())
}

async fn edit_profile_form(
    State(state): State<AppState>,
    user: RequireUser,
    csrf_token: CsrfToken,
) -> Result<Response, AppError> {
    let Some(profile) = state.profiles.get_editable_profile(user.0.user.id).await? else {
        return Ok(StatusCode::NOT_FOUND.into_response());
    };

    render_edit_profile(
        &profile,
        EditProfileFormValues {
            display_name: profile.display_name.clone(),
            bio: profile.bio.clone(),
        },
        None,
        csrf_token.0,
        StatusCode::OK,
    )
}

async fn update_profile(
    State(state): State<AppState>,
    user: RequireUser,
    csrf_token: CsrfToken,
    Form(form): Form<EditProfileForm>,
) -> Result<Response, AppError> {
    let Some(profile) = state.profiles.get_editable_profile(user.0.user.id).await? else {
        return Ok(StatusCode::NOT_FOUND.into_response());
    };

    let normalized_form = normalize_edit_profile_form(form);
    if let Err(message) = validate_edit_profile_form(&normalized_form) {
        return render_edit_profile(
            &profile,
            edit_profile_form_values(&normalized_form),
            Some(message),
            csrf_token.0,
            StatusCode::UNPROCESSABLE_ENTITY,
        );
    }

    if !state
        .profiles
        .update_profile(
            user.0.user.id,
            &normalized_form.display_name,
            &normalized_form.bio,
        )
        .await?
    {
        return Ok(StatusCode::NOT_FOUND.into_response());
    }

    Ok(Redirect::to(&format!("/u/{}", profile.username)).into_response())
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

async fn admin_users(
    State(state): State<AppState>,
    admin: RequireAdmin,
    Query(query): Query<PageQuery>,
    csrf_token: CsrfToken,
) -> Result<Response, AppError> {
    render_admin_users(
        &state,
        admin.0.user.id,
        query.page.unwrap_or(1).max(1),
        None,
        csrf_token.0,
        StatusCode::OK,
    )
    .await
}

async fn update_user_role(
    State(state): State<AppState>,
    admin: RequireAdmin,
    csrf_token: CsrfToken,
    Path(id): Path<i64>,
    Form(form): Form<UpdateUserRoleForm>,
) -> Result<Response, AppError> {
    let role = match parse_role(&form.role) {
        Some(role) => role,
        None => {
            return render_admin_users(
                &state,
                admin.0.user.id,
                1,
                Some("Invalid role selection.".to_string()),
                csrf_token.0,
                StatusCode::UNPROCESSABLE_ENTITY,
            )
            .await
        }
    };

    if admin.0.user.id == id && role != Role::Admin {
        return render_admin_users(
            &state,
            admin.0.user.id,
            1,
            Some("You cannot demote your own admin account.".to_string()),
            csrf_token.0,
            StatusCode::UNPROCESSABLE_ENTITY,
        )
        .await;
    }

    let updated = sqlx::query(
        r#"
        UPDATE users
        SET role = $2
        WHERE id = $1
        "#,
    )
    .bind(id)
    .bind(role)
    .execute(state.db.pool())
    .await?
    .rows_affected()
        > 0;

    if updated {
        Ok(Redirect::to("/admin/users").into_response())
    } else {
        Ok(StatusCode::NOT_FOUND.into_response())
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

fn search_result_row(result: search_store::SearchResultItem) -> SearchResultRow {
    let target_url = match result.post_id {
        Some(post_id) => format!("/t/{}-{}#post-{}", result.thread_id, result.thread_slug, post_id),
        None => format!("/t/{}-{}", result.thread_id, result.thread_slug),
    };

    SearchResultRow {
        result_kind: result.result_kind,
        thread_id: result.thread_id,
        thread_slug: result.thread_slug,
        thread_title: result.thread_title,
        target_url,
        body: result.body,
        created_at: result.created_at,
    }
}

fn thread_post_row(
    post: thread_store::ThreadPostItem,
    current_user_id: Option<i64>,
    edit_window_minutes: u64,
    can_moderate: bool,
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
        mod_delete_action: if can_moderate && post.deleted_at.is_none() {
            Some(format!("/p/{}/mod-delete", post.id))
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

fn normalize_edit_profile_form(form: EditProfileForm) -> EditProfileForm {
    EditProfileForm {
        display_name: form.display_name.trim().to_string(),
        bio: form.bio.trim().to_string(),
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

fn edit_profile_form_values(form: &EditProfileForm) -> EditProfileFormValues {
    EditProfileFormValues {
        display_name: form.display_name.clone(),
        bio: form.bio.clone(),
    }
}

fn validate_edit_profile_form(form: &EditProfileForm) -> Result<(), String> {
    if form.display_name.is_empty() {
        return Err("Display name is required.".to_string());
    }

    if form.display_name.len() > 64 {
        return Err("Display name must be 64 characters or fewer.".to_string());
    }

    if form.bio.len() > 280 {
        return Err("Bio must be 280 characters or fewer.".to_string());
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

fn render_edit_profile(
    profile: &profile_store::EditableProfile,
    form: EditProfileFormValues,
    error_message: Option<String>,
    csrf_token: Option<String>,
    status: StatusCode,
) -> Result<Response, AppError> {
    let html = render(EditProfileTemplate {
        profile: EditProfileContext {
            username: profile.username.clone(),
        },
        form,
        error_message,
        csrf_token,
    })?;

    Ok((status, html).into_response())
}

fn render_removed_thread(
    csrf_token: Option<String>,
    status: StatusCode,
) -> Result<Response, AppError> {
    let html = render(ErrorTemplate {
        status_code: status.as_u16(),
        title: "Thread Removed",
        message: "This thread has been removed from public view.",
        csrf_token,
    })?;

    Ok((status, html).into_response())
}

async fn toggle_thread_lock(
    state: &AppState,
    thread_id: i64,
    is_locked: bool,
) -> Result<Response, AppError> {
    let Some(thread) = state.threads.get_thread_detail(thread_id).await? else {
        return Ok(StatusCode::NOT_FOUND.into_response());
    };

    if !state.threads.set_locked(thread_id, is_locked).await? {
        return Ok(StatusCode::NOT_FOUND.into_response());
    }

    Ok(Redirect::to(&thread_path(&thread.slug, thread.id)).into_response())
}

async fn toggle_thread_pin(
    state: &AppState,
    thread_id: i64,
    is_pinned: bool,
) -> Result<Response, AppError> {
    let Some(thread) = state.threads.get_thread_detail(thread_id).await? else {
        return Ok(StatusCode::NOT_FOUND.into_response());
    };

    if !state.threads.set_pinned(thread_id, is_pinned).await? {
        return Ok(StatusCode::NOT_FOUND.into_response());
    }

    Ok(Redirect::to(&thread_path(&thread.slug, thread.id)).into_response())
}

async fn render_thread_page(
    state: &AppState,
    thread: thread_store::ThreadDetail,
    requested_page: i64,
    current_user: Option<auth::CurrentUser>,
    reply_error_message: Option<String>,
    reply_body: String,
    csrf_token: Option<String>,
    status: StatusCode,
) -> Result<Response, AppError> {
    let posts_page = state
        .threads
        .list_posts_for_thread(thread.id, requested_page, 20)
        .await?;
    let current_user_id = current_user.as_ref().map(|user| user.user.id);
    let can_moderate = current_user
        .as_ref()
        .map(|user| matches!(user.user.role, crate::models::user::Role::Moderator | crate::models::user::Role::Admin))
        .unwrap_or(false);
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
            .map(|post| thread_post_row(post, current_user_id, state.edit_window_minutes, can_moderate))
            .collect(),
        total_posts: posts_page.total_posts,
        current_page: posts_page.current_page,
        total_pages: posts_page.total_pages,
        prev_page: (posts_page.current_page > 1).then_some(posts_page.current_page - 1),
        next_page: (posts_page.current_page < posts_page.total_pages)
            .then_some(posts_page.current_page + 1),
        can_reply,
        can_moderate,
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

async fn render_admin_users(
    state: &AppState,
    current_admin_id: i64,
    requested_page: i64,
    error_message: Option<String>,
    csrf_token: Option<String>,
    status: StatusCode,
) -> Result<Response, AppError> {
    let per_page = 20_i64;
    let total_users = sqlx::query_scalar::<_, i64>(
        r#"
        SELECT COUNT(*)
        FROM users
        "#,
    )
    .fetch_one(state.db.pool())
    .await?;

    let total_pages = if total_users == 0 {
        1
    } else {
        ((total_users - 1) / per_page) + 1
    };
    let current_page = requested_page.clamp(1, total_pages);
    let offset = (current_page - 1) * per_page;

    let users = sqlx::query_as::<_, User>(
        r#"
        SELECT id, username, password_hash, display_name, bio, role, created_at
        FROM users
        ORDER BY created_at DESC, id DESC
        LIMIT $1
        OFFSET $2
        "#,
    )
    .bind(per_page)
    .bind(offset)
    .fetch_all(state.db.pool())
    .await?;

    let html = render(AdminUsersTemplate {
        users: users
            .into_iter()
            .map(|user| admin_user_row(user, current_admin_id))
            .collect(),
        error_message,
        current_page,
        total_pages,
        prev_page: (current_page > 1).then_some(current_page - 1),
        next_page: (current_page < total_pages).then_some(current_page + 1),
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

fn admin_user_row(user: User, current_admin_id: i64) -> AdminUserRow {
    AdminUserRow {
        id: user.id,
        username: user.username,
        display_name: user.display_name,
        role: role_as_str(user.role).to_string(),
        created_at: user.created_at,
        is_self: user.id == current_admin_id,
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

fn parse_role(value: &str) -> Option<Role> {
    match value.trim() {
        "user" => Some(Role::User),
        "moderator" => Some(Role::Moderator),
        "admin" => Some(Role::Admin),
        _ => None,
    }
}

fn role_as_str(role: Role) -> &'static str {
    match role {
        Role::User => "user",
        Role::Moderator => "moderator",
        Role::Admin => "admin",
    }
}

#[cfg(test)]
mod tests {
    use super::{app_router, auth, build_state};
    use axum::{
        body::{to_bytes, Body},
        http::{header, Request, StatusCode},
    };
    use sqlx::PgPool;
    use std::net::SocketAddr;
    use tower::ServiceExt;

    #[sqlx::test(migrator = "crate::db::MIGRATOR")]
    #[ignore = "requires a direct PostgreSQL test database; the Fly proxy endpoint cannot provision sqlx::test databases"]
    async fn register_login_logout_and_protected_route_access(pool: PgPool) {
        let app = test_app(pool.clone());

        let (register_cookie, register_csrf) = fetch_csrf(&app, "/register", None).await;
        let register_response = post_form(
            &app,
            "/register",
            Some(&register_cookie),
            &[
                ("username", "alice"),
                ("display_name", "Alice"),
                ("bio", "testing"),
                ("password", "password123"),
                (auth::CSRF_FORM_FIELD, &register_csrf),
            ],
        )
        .await;
        assert_eq!(register_response.status(), StatusCode::SEE_OTHER);

        let auth_cookie = response_cookie(&register_response).expect("register should issue auth cookie");
        let protected_response = get(&app, "/me/profile", Some(&auth_cookie)).await;
        assert_eq!(protected_response.status(), StatusCode::OK);

        let (_logout_cookie, logout_csrf) = fetch_csrf(&app, "/me/profile", Some(&auth_cookie)).await;
        let logout_response = post_form(
            &app,
            "/logout",
            Some(&auth_cookie),
            &[(auth::CSRF_FORM_FIELD, &logout_csrf)],
        )
        .await;
        assert_eq!(logout_response.status(), StatusCode::SEE_OTHER);

        let post_logout_response = get(&app, "/me/profile", Some(&auth_cookie)).await;
        assert_eq!(post_logout_response.status(), StatusCode::FOUND);
        assert_eq!(
            post_logout_response
                .headers()
                .get(header::LOCATION)
                .expect("redirect location should be present"),
            "/login"
        );

        let (login_cookie, login_csrf) = fetch_csrf(&app, "/login", None).await;
        let login_response = post_form(
            &app,
            "/login",
            Some(&login_cookie),
            &[
                ("username", "alice"),
                ("password", "password123"),
                (auth::CSRF_FORM_FIELD, &login_csrf),
            ],
        )
        .await;
        assert_eq!(login_response.status(), StatusCode::SEE_OTHER);

        let relogin_cookie = response_cookie(&login_response).expect("login should issue auth cookie");
        let relogin_protected_response = get(&app, "/me/profile", Some(&relogin_cookie)).await;
        assert_eq!(relogin_protected_response.status(), StatusCode::OK);
    }

    #[sqlx::test(migrator = "crate::db::MIGRATOR")]
    #[ignore = "requires a direct PostgreSQL test database; the Fly proxy endpoint cannot provision sqlx::test databases"]
    async fn duplicate_username_registration_is_rejected(pool: PgPool) {
        let app = test_app(pool);

        let (first_cookie, first_csrf) = fetch_csrf(&app, "/register", None).await;
        let first_response = post_form(
            &app,
            "/register",
            Some(&first_cookie),
            &[
                ("username", "duplicate"),
                ("display_name", "Duplicate"),
                ("bio", ""),
                ("password", "password123"),
                (auth::CSRF_FORM_FIELD, &first_csrf),
            ],
        )
        .await;
        assert_eq!(first_response.status(), StatusCode::SEE_OTHER);

        let (second_cookie, second_csrf) = fetch_csrf(&app, "/register", None).await;
        let second_response = post_form(
            &app,
            "/register",
            Some(&second_cookie),
            &[
                ("username", "duplicate"),
                ("display_name", "Duplicate Again"),
                ("bio", ""),
                ("password", "password123"),
                (auth::CSRF_FORM_FIELD, &second_csrf),
            ],
        )
        .await;
        assert_eq!(second_response.status(), StatusCode::UNPROCESSABLE_ENTITY);

        let body = response_text(second_response).await;
        assert!(body.contains("That username is already taken."));
    }

    #[sqlx::test(migrator = "crate::db::MIGRATOR")]
    #[ignore = "requires a direct PostgreSQL test database; the Fly proxy endpoint cannot provision sqlx::test databases"]
    async fn wrong_password_and_expired_session_are_rejected(pool: PgPool) {
        let app = test_app(pool.clone());

        let (register_cookie, register_csrf) = fetch_csrf(&app, "/register", None).await;
        let register_response = post_form(
            &app,
            "/register",
            Some(&register_cookie),
            &[
                ("username", "bob"),
                ("display_name", "Bob"),
                ("bio", ""),
                ("password", "password123"),
                (auth::CSRF_FORM_FIELD, &register_csrf),
            ],
        )
        .await;
        assert_eq!(register_response.status(), StatusCode::SEE_OTHER);

        let auth_cookie = response_cookie(&register_response).expect("register should issue auth cookie");

        let (login_cookie, login_csrf) = fetch_csrf(&app, "/login", None).await;
        let wrong_password_response = post_form(
            &app,
            "/login",
            Some(&login_cookie),
            &[
                ("username", "bob"),
                ("password", "not-the-password"),
                (auth::CSRF_FORM_FIELD, &login_csrf),
            ],
        )
        .await;
        assert_eq!(wrong_password_response.status(), StatusCode::UNPROCESSABLE_ENTITY);
        let body = response_text(wrong_password_response).await;
        assert!(body.contains("Invalid username or password."));

        let user_id = sqlx::query_scalar::<_, i64>(
            r#"
            SELECT id
            FROM users
            WHERE username = 'bob'
            "#,
        )
        .fetch_one(&pool)
        .await
        .expect("user should exist");

        sqlx::query(
            r#"
            UPDATE sessions
            SET expires_at = NOW() - INTERVAL '1 day'
            WHERE user_id = $1
            "#,
        )
        .bind(user_id)
        .execute(&pool)
        .await
        .expect("session expiration update should succeed");

        let expired_session_response = get(&app, "/me/profile", Some(&auth_cookie)).await;
        assert_eq!(expired_session_response.status(), StatusCode::FOUND);
        assert_eq!(
            expired_session_response
                .headers()
                .get(header::LOCATION)
                .expect("redirect location should be present"),
            "/login"
        );
    }

    #[sqlx::test(migrator = "crate::db::MIGRATOR")]
    #[ignore = "requires a direct PostgreSQL test database; the Fly proxy endpoint cannot provision sqlx::test databases"]
    async fn create_thread_and_reply_flow(pool: PgPool) {
        let app = test_app(pool.clone());
        let category_id = create_category(&pool, "general", "General discussion").await;
        let user_cookie = register_user(&app, "writer", "password123").await;
        let user_id = user_id_by_username(&pool, "writer").await;

        let (_new_thread_cookie, new_thread_csrf) =
            fetch_csrf(&app, "/c/general/new", Some(&user_cookie)).await;
        let create_thread_response = post_form(
            &app,
            "/c/general/new",
            Some(&user_cookie),
            &[
                ("title", "Hello Forum"),
                ("body", "Opening post"),
                (auth::CSRF_FORM_FIELD, &new_thread_csrf),
            ],
        )
        .await;
        assert_eq!(create_thread_response.status(), StatusCode::SEE_OTHER);

        let thread_location = response_location(&create_thread_response).expect("thread redirect should exist");
        let thread_id = thread_id_by_title(&pool, "Hello Forum").await;
        let thread_page_response = get(&app, &thread_location, Some(&user_cookie)).await;
        assert_eq!(thread_page_response.status(), StatusCode::OK);

        let (_thread_cookie, reply_csrf) = fetch_csrf(&app, &thread_location, Some(&user_cookie)).await;
        let reply_response = post_form(
            &app,
            &format!("/t/{thread_id}/reply"),
            Some(&user_cookie),
            &[
                ("body", "Second post"),
                ("page", "1"),
                (auth::CSRF_FORM_FIELD, &reply_csrf),
            ],
        )
        .await;
        assert_eq!(reply_response.status(), StatusCode::SEE_OTHER);

        let total_posts = sqlx::query_scalar::<_, i64>(
            r#"
            SELECT COUNT(*)
            FROM posts
            WHERE thread_id = $1
              AND deleted_at IS NULL
            "#,
        )
        .bind(thread_id)
        .fetch_one(&pool)
        .await
        .expect("post count query should succeed");
        assert_eq!(total_posts, 2);

        let reply_author_id = sqlx::query_scalar::<_, i64>(
            r#"
            SELECT author_id
            FROM posts
            WHERE thread_id = $1
            ORDER BY id DESC
            LIMIT 1
            "#,
        )
        .bind(thread_id)
        .fetch_one(&pool)
        .await
        .expect("reply author query should succeed");
        assert_eq!(reply_author_id, user_id);

        let category_page_response = get(&app, "/c/general", Some(&user_cookie)).await;
        let category_page_body = response_text(category_page_response).await;
        assert!(category_page_body.contains("Hello Forum"));
        assert!(category_page_body.contains("1"));

        let _ = category_id;
    }

    #[sqlx::test(migrator = "crate::db::MIGRATOR")]
    #[ignore = "requires a direct PostgreSQL test database; the Fly proxy endpoint cannot provision sqlx::test databases"]
    async fn edit_window_enforcement_and_lock_blocks_reply(pool: PgPool) {
        let app = test_app(pool.clone());
        create_category(&pool, "support", "Support").await;
        let user_cookie = register_user(&app, "editor", "password123").await;
        let moderator_cookie = register_user(&app, "moduser", "password123").await;
        set_user_role(&pool, "moduser", "moderator").await;

        let (_new_thread_cookie, new_thread_csrf) =
            fetch_csrf(&app, "/c/support/new", Some(&user_cookie)).await;
        let create_thread_response = post_form(
            &app,
            "/c/support/new",
            Some(&user_cookie),
            &[
                ("title", "Need Help"),
                ("body", "First post"),
                (auth::CSRF_FORM_FIELD, &new_thread_csrf),
            ],
        )
        .await;
        assert_eq!(create_thread_response.status(), StatusCode::SEE_OTHER);

        let thread_id = thread_id_by_title(&pool, "Need Help").await;
        let thread_path = response_location(&create_thread_response).expect("thread redirect should exist");
        let post_id = sqlx::query_scalar::<_, i64>(
            r#"
            SELECT id
            FROM posts
            WHERE thread_id = $1
            ORDER BY id ASC
            LIMIT 1
            "#,
        )
        .bind(thread_id)
        .fetch_one(&pool)
        .await
        .expect("post id query should succeed");

        sqlx::query(
            r#"
            UPDATE posts
            SET created_at = NOW() - INTERVAL '1 day',
                updated_at = NOW() - INTERVAL '1 day'
            WHERE id = $1
            "#,
        )
        .bind(post_id)
        .execute(&pool)
        .await
        .expect("post timestamps should update");

        let edit_response = get(&app, &format!("/p/{post_id}/edit"), Some(&user_cookie)).await;
        assert_eq!(edit_response.status(), StatusCode::FORBIDDEN);

        let (_moderator_page_cookie, moderator_csrf) =
            fetch_csrf(&app, &thread_path, Some(&moderator_cookie)).await;
        let lock_response = post_form(
            &app,
            &format!("/t/{thread_id}/lock"),
            Some(&moderator_cookie),
            &[(auth::CSRF_FORM_FIELD, &moderator_csrf)],
        )
        .await;
        assert_eq!(lock_response.status(), StatusCode::SEE_OTHER);

        let (_reply_page_cookie, reply_csrf) = fetch_csrf(&app, &thread_path, Some(&user_cookie)).await;
        let locked_reply_response = post_form(
            &app,
            &format!("/t/{thread_id}/reply"),
            Some(&user_cookie),
            &[
                ("body", "Can I still reply?"),
                ("page", "1"),
                (auth::CSRF_FORM_FIELD, &reply_csrf),
            ],
        )
        .await;
        assert_eq!(locked_reply_response.status(), StatusCode::LOCKED);
        let locked_body = response_text(locked_reply_response).await;
        assert!(locked_body.contains("This thread is locked. New replies are disabled."));
    }

    #[sqlx::test(migrator = "crate::db::MIGRATOR")]
    #[ignore = "requires a direct PostgreSQL test database; the Fly proxy endpoint cannot provision sqlx::test databases"]
    async fn moderator_delete_and_role_guard_rejections(pool: PgPool) {
        let app = test_app(pool.clone());
        create_category(&pool, "meta", "Meta").await;
        let author_cookie = register_user(&app, "author", "password123").await;
        let moderator_cookie = register_user(&app, "modboss", "password123").await;
        let plain_cookie = register_user(&app, "plainuser", "password123").await;
        set_user_role(&pool, "modboss", "moderator").await;

        let (_new_thread_cookie, new_thread_csrf) =
            fetch_csrf(&app, "/c/meta/new", Some(&author_cookie)).await;
        let create_thread_response = post_form(
            &app,
            "/c/meta/new",
            Some(&author_cookie),
            &[
                ("title", "Moderation Target"),
                ("body", "Original body"),
                (auth::CSRF_FORM_FIELD, &new_thread_csrf),
            ],
        )
        .await;
        assert_eq!(create_thread_response.status(), StatusCode::SEE_OTHER);

        let thread_id = thread_id_by_title(&pool, "Moderation Target").await;
        let thread_path = response_location(&create_thread_response).expect("thread redirect should exist");
        let post_id = sqlx::query_scalar::<_, i64>(
            r#"
            SELECT id
            FROM posts
            WHERE thread_id = $1
            ORDER BY id ASC
            LIMIT 1
            "#,
        )
        .bind(thread_id)
        .fetch_one(&pool)
        .await
        .expect("post id query should succeed");

        let (guest_cookie, guest_csrf) = fetch_csrf(&app, &thread_path, None).await;
        let guest_lock_response = post_form(
            &app,
            &format!("/t/{thread_id}/lock"),
            Some(&guest_cookie),
            &[(auth::CSRF_FORM_FIELD, &guest_csrf)],
        )
        .await;
        assert_eq!(guest_lock_response.status(), StatusCode::FOUND);
        assert_eq!(
            guest_lock_response
                .headers()
                .get(header::LOCATION)
                .expect("guest redirect should have location"),
            "/login"
        );

        let (_plain_cookie_page, plain_csrf) = fetch_csrf(&app, &thread_path, Some(&plain_cookie)).await;
        let plain_lock_response = post_form(
            &app,
            &format!("/t/{thread_id}/lock"),
            Some(&plain_cookie),
            &[(auth::CSRF_FORM_FIELD, &plain_csrf)],
        )
        .await;
        assert_eq!(plain_lock_response.status(), StatusCode::FORBIDDEN);

        let (_moderator_page_cookie, moderator_csrf) =
            fetch_csrf(&app, &thread_path, Some(&moderator_cookie)).await;
        let mod_delete_response = post_form(
            &app,
            &format!("/p/{post_id}/mod-delete"),
            Some(&moderator_cookie),
            &[(auth::CSRF_FORM_FIELD, &moderator_csrf)],
        )
        .await;
        assert_eq!(mod_delete_response.status(), StatusCode::SEE_OTHER);

        let deleted_row = sqlx::query_as::<_, (Option<chrono::DateTime<chrono::Utc>>, Option<i64>)>(
            r#"
            SELECT deleted_at, deleted_by
            FROM posts
            WHERE id = $1
            "#,
        )
        .bind(post_id)
        .fetch_one(&pool)
        .await
        .expect("deleted post row should load");
        assert!(deleted_row.0.is_some());

        let moderator_id = user_id_by_username(&pool, "modboss").await;
        assert_eq!(deleted_row.1, Some(moderator_id));
    }

    fn test_app(pool: PgPool) -> axum::Router {
        let db = crate::db::Db::from_pool(pool);
        let state = build_state(
            SocketAddr::from(([127, 0, 0, 1], 3000)),
            db,
            15,
            "integration-test-session-secret-1234567890".to_string(),
        );

        app_router(state)
    }

    async fn fetch_csrf(
        app: &axum::Router,
        path: &str,
        cookie: Option<&str>,
    ) -> (String, String) {
        let response = get(app, path, cookie).await;
        let response_cookie = response_cookie(&response).unwrap_or_else(|| cookie.unwrap_or_default().to_string());
        let body = response_text(response).await;
        let csrf_token = extract_csrf_token(&body).expect("csrf token should be present");

        (response_cookie, csrf_token)
    }

    async fn get(app: &axum::Router, path: &str, cookie: Option<&str>) -> axum::response::Response {
        let mut builder = Request::builder().method("GET").uri(path);
        if let Some(cookie) = cookie {
            builder = builder.header(header::COOKIE, cookie);
        }

        app.clone()
            .oneshot(builder.body(Body::empty()).expect("request should build"))
            .await
            .expect("request should succeed")
    }

    async fn post_form(
        app: &axum::Router,
        path: &str,
        cookie: Option<&str>,
        fields: &[(&str, &str)],
    ) -> axum::response::Response {
        let encoded = serde_urlencoded::to_string(fields).expect("form should encode");
        let mut builder = Request::builder()
            .method("POST")
            .uri(path)
            .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded");

        if let Some(cookie) = cookie {
            builder = builder.header(header::COOKIE, cookie);
        }

        app.clone()
            .oneshot(builder.body(Body::from(encoded)).expect("request should build"))
            .await
            .expect("request should succeed")
    }

    async fn register_user(app: &axum::Router, username: &str, password: &str) -> String {
        let (cookie, csrf_token) = fetch_csrf(app, "/register", None).await;
        let response = post_form(
            app,
            "/register",
            Some(&cookie),
            &[
                ("username", username),
                ("display_name", username),
                ("bio", ""),
                ("password", password),
                (auth::CSRF_FORM_FIELD, &csrf_token),
            ],
        )
        .await;
        assert_eq!(response.status(), StatusCode::SEE_OTHER);
        response_cookie(&response).expect("register should issue auth cookie")
    }

    async fn create_category(pool: &PgPool, slug: &str, name: &str) -> i64 {
        sqlx::query_scalar::<_, i64>(
            r#"
            INSERT INTO categories (name, slug, description, position)
            VALUES ($1, $2, $3, 0)
            RETURNING id
            "#,
        )
        .bind(name)
        .bind(slug)
        .bind(format!("{name} description"))
        .fetch_one(pool)
        .await
        .expect("category insert should succeed")
    }

    async fn set_user_role(pool: &PgPool, username: &str, role: &str) {
        sqlx::query(
            r#"
            UPDATE users
            SET role = $2::user_role
            WHERE username = $1
            "#,
        )
        .bind(username)
        .bind(role)
        .execute(pool)
        .await
        .expect("role update should succeed");
    }

    async fn user_id_by_username(pool: &PgPool, username: &str) -> i64 {
        sqlx::query_scalar::<_, i64>(
            r#"
            SELECT id
            FROM users
            WHERE username = $1
            "#,
        )
        .bind(username)
        .fetch_one(pool)
        .await
        .expect("user id query should succeed")
    }

    async fn thread_id_by_title(pool: &PgPool, title: &str) -> i64 {
        sqlx::query_scalar::<_, i64>(
            r#"
            SELECT id
            FROM threads
            WHERE title = $1
            ORDER BY id DESC
            LIMIT 1
            "#,
        )
        .bind(title)
        .fetch_one(pool)
        .await
        .expect("thread id query should succeed")
    }

    fn response_cookie(response: &axum::response::Response) -> Option<String> {
        response
            .headers()
            .get(header::SET_COOKIE)
            .and_then(|value| value.to_str().ok())
            .and_then(|cookie| cookie.split(';').next().map(str::to_string))
    }

    fn response_location(response: &axum::response::Response) -> Option<String> {
        response
            .headers()
            .get(header::LOCATION)
            .and_then(|value| value.to_str().ok())
            .map(str::to_string)
    }

    async fn response_text(response: axum::response::Response) -> String {
        let bytes = to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("response body should read");
        String::from_utf8(bytes.to_vec()).expect("response body should be utf-8")
    }

    fn extract_csrf_token(body: &str) -> Option<String> {
        let marker = "name=\"csrf_token\" value=\"";
        let start = body.find(marker)? + marker.len();
        let end = body[start..].find('"')? + start;
        Some(body[start..end].to_string())
    }
}
