use askama::Template;
use axum::{
    http::StatusCode,
    response::{Html, IntoResponse, Response},
};
use tracing::error;

pub struct HtmlTemplate<T>(pub T);

impl<T> IntoResponse for HtmlTemplate<T>
where
    T: Template,
{
    fn into_response(self) -> Response {
        match self.0.render() {
            Ok(html) => Html(html).into_response(),
            Err(err) => {
                error!(error = %err, "failed to render template");
                StatusCode::INTERNAL_SERVER_ERROR.into_response()
            }
        }
    }
}

#[derive(Template)]
#[template(path = "home.html")]
pub struct HomeTemplate;
