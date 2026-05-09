use askama::Template;
use axum::{
    http::StatusCode,
    response::{Html, IntoResponse, Response},
};
use thiserror::Error;
use tracing::error;

use crate::{config::ConfigError, password::PasswordError, templates::ErrorTemplate};

#[derive(Debug, Error)]
pub enum AppError {
    #[error("requested resource was not found")]
    NotFound {
        title: &'static str,
        message: &'static str,
    },
    #[error("request was forbidden")]
    Forbidden {
        title: &'static str,
        message: &'static str,
    },
    #[error("requested resource is no longer available")]
    Gone {
        title: &'static str,
        message: &'static str,
    },
    #[error("failed to load application configuration")]
    Config(#[from] ConfigError),
    #[error("failed to connect to the database")]
    Database(#[from] sqlx::Error),
    #[error("failed to apply database migrations")]
    Migration(#[from] sqlx::migrate::MigrateError),
    #[error("failed to render template")]
    Template(#[from] askama::Error),
    #[error("failed to hash or verify a password")]
    Password(#[from] PasswordError),
    #[error("request handling failed")]
    Io(#[from] std::io::Error),
}

impl AppError {
    pub fn not_found(title: &'static str, message: &'static str) -> Self {
        Self::NotFound { title, message }
    }

    pub fn forbidden(title: &'static str, message: &'static str) -> Self {
        Self::Forbidden { title, message }
    }

    pub fn gone(title: &'static str, message: &'static str) -> Self {
        Self::Gone { title, message }
    }

    fn status_code(&self) -> StatusCode {
        match self {
            Self::NotFound { .. } => StatusCode::NOT_FOUND,
            Self::Forbidden { .. } => StatusCode::FORBIDDEN,
            Self::Gone { .. } => StatusCode::GONE,
            Self::Template(_) | Self::Password(_) => StatusCode::INTERNAL_SERVER_ERROR,
            Self::Config(_) | Self::Database(_) | Self::Migration(_) | Self::Io(_) => {
                StatusCode::INTERNAL_SERVER_ERROR
            }
        }
    }

    fn title(&self) -> &'static str {
        match self {
            Self::NotFound { title, .. } => title,
            Self::Forbidden { title, .. } => title,
            Self::Gone { title, .. } => title,
            Self::Template(_) => "Template Error",
            Self::Config(_) => "Configuration Error",
            Self::Database(_) => "Database Error",
            Self::Migration(_) => "Migration Error",
            Self::Password(_) => "Password Error",
            Self::Io(_) => "Server Error",
        }
    }

    fn user_message(&self) -> &'static str {
        match self {
            Self::NotFound { message, .. } => message,
            Self::Forbidden { message, .. } => message,
            Self::Gone { message, .. } => message,
            Self::Template(_) => "The page could not be rendered.",
            Self::Config(_) => "The server configuration is invalid.",
            Self::Database(_) => "The database is currently unavailable.",
            Self::Migration(_) => "The database schema could not be prepared.",
            Self::Password(_) => "The password could not be processed.",
            Self::Io(_) => "The server could not complete the request.",
        }
    }
}

pub fn render_error_page(status: StatusCode, title: &str, message: &str) -> Response {
    let template = ErrorTemplate {
        status_code: status.as_u16(),
        title,
        message,
        is_authenticated: false,
        csrf_token: None,
    };

    match template.render() {
        Ok(html) => (status, Html(html)).into_response(),
        Err(render_err) => {
            error!(error = %render_err, "failed to render error template");
            (status, message.to_string()).into_response()
        }
    }
}

impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        let status = self.status_code();
        error!(error = %self, "request failed");
        render_error_page(status, self.title(), self.user_message())
    }
}
