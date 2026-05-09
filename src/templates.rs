use askama::Template;
use axum::response::Html;

use crate::error::AppError;

pub fn render<T>(template: T) -> Result<Html<String>, AppError>
where
    T: Template,
{
    Ok(Html(template.render()?))
}

#[derive(Template)]
#[template(path = "home.html")]
pub struct HomeTemplate {
    pub csrf_token: Option<String>,
}

#[derive(Template)]
#[template(path = "register.html")]
pub struct RegisterTemplate {
    pub username: String,
    pub display_name: String,
    pub bio: String,
    pub error_message: Option<String>,
    pub csrf_token: Option<String>,
}

#[derive(Template)]
#[template(path = "login.html")]
pub struct LoginTemplate {
    pub username: String,
    pub error_message: Option<String>,
    pub csrf_token: Option<String>,
}

#[derive(Template)]
#[template(path = "error.html")]
pub struct ErrorTemplate<'a> {
    pub status_code: u16,
    pub title: &'a str,
    pub message: &'a str,
    pub csrf_token: Option<String>,
}
