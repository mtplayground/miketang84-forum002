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
    fn status_code(&self) -> StatusCode {
        match self {
            Self::Template(_) | Self::Password(_) => StatusCode::INTERNAL_SERVER_ERROR,
            Self::Config(_) | Self::Database(_) | Self::Migration(_) | Self::Io(_) => {
                StatusCode::INTERNAL_SERVER_ERROR
            }
        }
    }

    fn title(&self) -> &'static str {
        match self {
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
            Self::Template(_) => "The page could not be rendered.",
            Self::Config(_) => "The server configuration is invalid.",
            Self::Database(_) => "The database is currently unavailable.",
            Self::Migration(_) => "The database schema could not be prepared.",
            Self::Password(_) => "The password could not be processed.",
            Self::Io(_) => "The server could not complete the request.",
        }
    }
}

impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        let status = self.status_code();
        error!(error = %self, "request failed");

        let template = ErrorTemplate {
            status_code: status.as_u16(),
            title: self.title(),
            message: self.user_message(),
            csrf_token: None,
        };

        match template.render() {
            Ok(html) => (status, Html(html)).into_response(),
            Err(render_err) => {
                error!(error = %render_err, "failed to render error template");
                (status, self.to_string()).into_response()
            }
        }
    }
}
