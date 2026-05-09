use askama::Template;
use axum::response::Html;
use chrono::{DateTime, Utc};

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
    pub categories: Vec<HomeCategoryCard>,
    pub csrf_token: Option<String>,
}

#[derive(Template)]
#[template(path = "admin_categories.html")]
pub struct AdminCategoriesTemplate {
    pub categories: Vec<AdminCategoryRow>,
    pub create_form: AdminCategoryFormValues,
    pub error_message: Option<String>,
    pub csrf_token: Option<String>,
}

#[derive(Template)]
#[template(path = "category.html")]
pub struct CategoryTemplate {
    pub category: CategoryHeader,
    pub threads: Vec<CategoryThreadRow>,
    pub total_threads: i64,
    pub current_page: i64,
    pub total_pages: i64,
    pub prev_page: Option<i64>,
    pub next_page: Option<i64>,
    pub csrf_token: Option<String>,
}

#[derive(Template)]
#[template(path = "new_thread.html")]
pub struct NewThreadTemplate {
    pub category: CategoryHeader,
    pub form: NewThreadFormValues,
    pub error_message: Option<String>,
    pub csrf_token: Option<String>,
}

#[derive(Template)]
#[template(path = "edit_post.html")]
pub struct EditPostTemplate {
    pub post: EditPostContext,
    pub form: EditPostFormValues,
    pub error_message: Option<String>,
    pub csrf_token: Option<String>,
}

#[derive(Template)]
#[template(path = "thread.html")]
pub struct ThreadTemplate {
    pub thread: ThreadHeader,
    pub posts: Vec<ThreadPostRow>,
    pub total_posts: i64,
    pub current_page: i64,
    pub total_pages: i64,
    pub prev_page: Option<i64>,
    pub next_page: Option<i64>,
    pub can_reply: bool,
    pub reply_form_action: String,
    pub reply_error_message: Option<String>,
    pub reply_body: String,
    pub csrf_token: Option<String>,
}

#[derive(Debug, Clone)]
pub struct HomeCategoryCard {
    pub name: String,
    pub slug: String,
    pub description: String,
    pub position: i32,
    pub thread_count: i64,
    pub most_recent_thread: Option<HomeRecentThread>,
}

#[derive(Debug, Clone)]
pub struct HomeRecentThread {
    pub title: String,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone)]
pub struct AdminCategoryRow {
    pub id: i64,
    pub name: String,
    pub slug: String,
    pub description: String,
    pub position: i32,
}

#[derive(Debug, Clone, Default)]
pub struct AdminCategoryFormValues {
    pub name: String,
    pub slug: String,
    pub description: String,
    pub position: i32,
}

#[derive(Debug, Clone)]
pub struct CategoryHeader {
    pub name: String,
    pub slug: String,
    pub description: String,
}

#[derive(Debug, Clone)]
pub struct CategoryThreadRow {
    pub id: i64,
    pub title: String,
    pub slug: String,
    pub author_username: String,
    pub reply_count: i64,
    pub last_activity_at: DateTime<Utc>,
    pub is_pinned: bool,
    pub is_locked: bool,
}

#[derive(Debug, Clone, Default)]
pub struct NewThreadFormValues {
    pub title: String,
    pub body: String,
}

#[derive(Debug, Clone)]
pub struct ThreadHeader {
    pub id: i64,
    pub title: String,
    pub slug: String,
    pub category_name: String,
    pub category_slug: String,
    pub author_username: String,
    pub created_at: DateTime<Utc>,
    pub last_activity_at: DateTime<Utc>,
    pub is_pinned: bool,
    pub is_locked: bool,
}

#[derive(Debug, Clone)]
pub struct ThreadPostRow {
    pub id: i64,
    pub author_username: String,
    pub body: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub edit_url: Option<String>,
    pub delete_action: Option<String>,
    pub is_deleted: bool,
}

#[derive(Debug, Clone)]
pub struct EditPostContext {
    pub id: i64,
    pub thread_id: i64,
    pub thread_title: String,
    pub thread_slug: String,
    pub author_username: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Default)]
pub struct EditPostFormValues {
    pub body: String,
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
