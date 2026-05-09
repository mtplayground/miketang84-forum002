use axum::{
    extract::{FromRequestParts, State},
    http::{header::{COOKIE, LOCATION}, request::Parts, HeaderMap, HeaderValue, StatusCode},
    middleware::Next,
    response::{IntoResponse, Response},
};
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};
use chrono::Utc;
use hmac::{Hmac, Mac};
use sha2::Sha256;
use tracing::warn;
use uuid::Uuid;

use crate::{models::user::User, AppState};

type HmacSha256 = Hmac<Sha256>;

pub const SESSION_COOKIE_NAME: &str = "session_id";

#[allow(dead_code)]
#[derive(Clone, Debug)]
pub struct CurrentUser {
    pub session_id: Uuid,
    pub user: User,
}

#[derive(Clone, Debug, Default)]
pub struct MaybeUser(pub Option<CurrentUser>);

#[allow(dead_code)]
pub struct RequireUser(pub CurrentUser);

#[allow(dead_code)]
pub struct RequireModerator(pub CurrentUser);

#[allow(dead_code)]
pub struct RequireAdmin(pub CurrentUser);

pub async fn session_cookie_middleware(
    State(state): State<AppState>,
    mut request: axum::extract::Request,
    next: Next,
) -> Response {
    let maybe_user = match signed_session_id_from_headers(request.headers(), &state.session_secret) {
        Some(session_id) => match state.sessions.load(session_id).await {
            Ok(Some(session)) if session.expires_at > Utc::now() => {
                match sqlx::query_as::<_, User>(
                    r#"
                    SELECT id, username, password_hash, display_name, bio, role, created_at
                    FROM users
                    WHERE id = $1
                    "#,
                )
                .bind(session.user_id)
                .fetch_optional(state.db.pool())
                .await
                {
                    Ok(Some(user)) => MaybeUser(Some(CurrentUser { session_id, user })),
                    Ok(None) => MaybeUser(None),
                    Err(err) => {
                        warn!(error = %err, "failed to load current user from session");
                        MaybeUser(None)
                    }
                }
            }
            Ok(Some(_expired)) => {
                let _ = state.sessions.delete(session_id).await;
                MaybeUser(None)
            }
            Ok(None) => MaybeUser(None),
            Err(err) => {
                warn!(error = %err, "failed to load session from cookie");
                MaybeUser(None)
            }
        },
        None => MaybeUser(None),
    };

    request.extensions_mut().insert(maybe_user);

    next.run(request).await
}

#[axum::async_trait]
impl<S> FromRequestParts<S> for MaybeUser
where
    S: Send + Sync,
{
    type Rejection = Response;

    async fn from_request_parts(parts: &mut Parts, _state: &S) -> Result<Self, Self::Rejection> {
        Ok(parts
            .extensions
            .get::<MaybeUser>()
            .cloned()
            .unwrap_or_default())
    }
}

#[axum::async_trait]
impl<S> FromRequestParts<S> for CurrentUser
where
    S: Send + Sync,
{
    type Rejection = Response;

    async fn from_request_parts(parts: &mut Parts, _state: &S) -> Result<Self, Self::Rejection> {
        let maybe_user = parts
            .extensions
            .get::<MaybeUser>()
            .cloned()
            .unwrap_or_default();

        maybe_user
            .0
            .ok_or_else(|| StatusCode::UNAUTHORIZED.into_response())
    }
}

#[axum::async_trait]
impl<S> FromRequestParts<S> for RequireUser
where
    S: Send + Sync,
{
    type Rejection = Response;

    async fn from_request_parts(parts: &mut Parts, state: &S) -> Result<Self, Self::Rejection> {
        let maybe_user = MaybeUser::from_request_parts(parts, state).await?;
        let user = maybe_user
            .0
            .ok_or_else(|| (StatusCode::FOUND, [(LOCATION, "/login")]).into_response())?;

        Ok(Self(user))
    }
}

#[axum::async_trait]
impl<S> FromRequestParts<S> for RequireModerator
where
    S: Send + Sync,
{
    type Rejection = Response;

    async fn from_request_parts(parts: &mut Parts, state: &S) -> Result<Self, Self::Rejection> {
        let user = RequireUser::from_request_parts(parts, state).await?.0;

        if matches!(user.user.role, crate::models::user::Role::Moderator | crate::models::user::Role::Admin)
        {
            Ok(Self(user))
        } else {
            Err(StatusCode::FORBIDDEN.into_response())
        }
    }
}

#[axum::async_trait]
impl<S> FromRequestParts<S> for RequireAdmin
where
    S: Send + Sync,
{
    type Rejection = Response;

    async fn from_request_parts(parts: &mut Parts, state: &S) -> Result<Self, Self::Rejection> {
        let user = RequireUser::from_request_parts(parts, state).await?.0;

        if matches!(user.user.role, crate::models::user::Role::Admin) {
            Ok(Self(user))
        } else {
            Err(StatusCode::FORBIDDEN.into_response())
        }
    }
}

pub fn build_session_cookie(
    session_id: Uuid,
    session_secret: &str,
    max_age_seconds: i64,
) -> Result<HeaderValue, std::io::Error> {
    let signed_value = signed_session_value(session_id, session_secret)?;
    HeaderValue::from_str(&format!(
        "{name}={value}; Path=/; HttpOnly; SameSite=Lax; Max-Age={age}",
        name = SESSION_COOKIE_NAME,
        value = signed_value,
        age = max_age_seconds
    ))
    .map_err(header_error)
}

pub fn clear_session_cookie() -> Result<HeaderValue, std::io::Error> {
    HeaderValue::from_str(&format!(
        "{name}=; Path=/; HttpOnly; SameSite=Lax; Max-Age=0",
        name = SESSION_COOKIE_NAME
    ))
    .map_err(header_error)
}

pub fn signed_session_id_from_headers(headers: &HeaderMap, session_secret: &str) -> Option<Uuid> {
    let cookie_header = headers.get(COOKIE)?;
    let cookie_str = cookie_header.to_str().ok()?;

    cookie_str.split(';').find_map(|pair| {
        let (name, value) = pair.trim().split_once('=')?;

        if name == SESSION_COOKIE_NAME {
            verify_signed_session_value(value, session_secret).ok()
        } else {
            None
        }
    })
}

fn signed_session_value(session_id: Uuid, session_secret: &str) -> Result<String, std::io::Error> {
    let session_id = session_id.to_string();
    let mut mac =
        HmacSha256::new_from_slice(session_secret.as_bytes()).map_err(header_error)?;
    mac.update(session_id.as_bytes());
    let signature = URL_SAFE_NO_PAD.encode(mac.finalize().into_bytes());

    Ok(format!("{session_id}.{signature}"))
}

fn verify_signed_session_value(
    signed_value: &str,
    session_secret: &str,
) -> Result<Uuid, std::io::Error> {
    let (session_id, signature) = signed_value
        .split_once('.')
        .ok_or_else(|| std::io::Error::new(std::io::ErrorKind::InvalidData, "missing signature"))?;
    let expected = signed_session_value(
        Uuid::parse_str(session_id).map_err(header_error)?,
        session_secret,
    )?;

    if expected != signed_value {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "invalid session signature",
        ));
    }

    let _ = signature;
    Uuid::parse_str(session_id).map_err(header_error)
}

fn header_error<E>(err: E) -> std::io::Error
where
    E: std::fmt::Display,
{
    std::io::Error::new(std::io::ErrorKind::InvalidInput, err.to_string())
}

#[cfg(test)]
mod tests {
    use axum::{extract::Extension, http::Request, routing::get, Router};
    use chrono::Utc;
    use tower::ServiceExt;
    use uuid::Uuid;

    use super::{CurrentUser, MaybeUser, RequireAdmin, RequireModerator, RequireUser};
    use crate::models::user::{Role, User};

    #[tokio::test]
    async fn require_user_redirects_guest_to_login() {
        async fn handler(_: RequireUser) -> &'static str {
            "ok"
        }

        let response = Router::new()
            .route("/", get(handler))
            .oneshot(Request::builder().uri("/").body(axum::body::Body::empty()).unwrap())
            .await
            .expect("request should succeed");

        assert_eq!(response.status(), axum::http::StatusCode::FOUND);
        assert_eq!(
            response.headers().get(axum::http::header::LOCATION).unwrap(),
            "/login"
        );
    }

    #[tokio::test]
    async fn require_user_allows_authenticated_user() {
        async fn handler(_: RequireUser) -> &'static str {
            "ok"
        }

        let response = Router::new()
            .route("/", get(handler))
            .layer(Extension(MaybeUser(Some(test_user(Role::User)))))
            .oneshot(Request::builder().uri("/").body(axum::body::Body::empty()).unwrap())
            .await
            .expect("request should succeed");

        assert_eq!(response.status(), axum::http::StatusCode::OK);
    }

    #[tokio::test]
    async fn require_moderator_redirects_guest_to_login() {
        async fn handler(_: RequireModerator) -> &'static str {
            "ok"
        }

        let response = Router::new()
            .route("/", get(handler))
            .oneshot(Request::builder().uri("/").body(axum::body::Body::empty()).unwrap())
            .await
            .expect("request should succeed");

        assert_eq!(response.status(), axum::http::StatusCode::FOUND);
    }

    #[tokio::test]
    async fn require_moderator_forbids_plain_user() {
        async fn handler(_: RequireModerator) -> &'static str {
            "ok"
        }

        let response = Router::new()
            .route("/", get(handler))
            .layer(Extension(MaybeUser(Some(test_user(Role::User)))))
            .oneshot(Request::builder().uri("/").body(axum::body::Body::empty()).unwrap())
            .await
            .expect("request should succeed");

        assert_eq!(response.status(), axum::http::StatusCode::FORBIDDEN);
    }

    #[tokio::test]
    async fn require_moderator_allows_moderator() {
        async fn handler(_: RequireModerator) -> &'static str {
            "ok"
        }

        let response = Router::new()
            .route("/", get(handler))
            .layer(Extension(MaybeUser(Some(test_user(Role::Moderator)))))
            .oneshot(Request::builder().uri("/").body(axum::body::Body::empty()).unwrap())
            .await
            .expect("request should succeed");

        assert_eq!(response.status(), axum::http::StatusCode::OK);
    }

    #[tokio::test]
    async fn require_admin_forbids_moderator() {
        async fn handler(_: RequireAdmin) -> &'static str {
            "ok"
        }

        let response = Router::new()
            .route("/", get(handler))
            .layer(Extension(MaybeUser(Some(test_user(Role::Moderator)))))
            .oneshot(Request::builder().uri("/").body(axum::body::Body::empty()).unwrap())
            .await
            .expect("request should succeed");

        assert_eq!(response.status(), axum::http::StatusCode::FORBIDDEN);
    }

    #[tokio::test]
    async fn require_admin_allows_admin() {
        async fn handler(_: RequireAdmin) -> &'static str {
            "ok"
        }

        let response = Router::new()
            .route("/", get(handler))
            .layer(Extension(MaybeUser(Some(test_user(Role::Admin)))))
            .oneshot(Request::builder().uri("/").body(axum::body::Body::empty()).unwrap())
            .await
            .expect("request should succeed");

        assert_eq!(response.status(), axum::http::StatusCode::OK);
    }

    fn test_user(role: Role) -> CurrentUser {
        CurrentUser {
            session_id: Uuid::new_v4(),
            user: User {
                id: 1,
                username: "tester".to_string(),
                password_hash: "hash".to_string(),
                display_name: "Tester".to_string(),
                bio: String::new(),
                role,
                created_at: Utc::now(),
            },
        }
    }
}
