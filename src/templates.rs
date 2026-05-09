use askama::Template;
use axum::response::Html;
use chrono::{DateTime, Utc};

use crate::error::AppError;

pub mod filters {
    pub fn format_body(value: &str) -> askama::Result<String> {
        Ok(escape_html(value).replace('\n', "<br>"))
    }

    fn escape_html(value: &str) -> String {
        let mut escaped = String::with_capacity(value.len());

        for ch in value.chars() {
            match ch {
                '&' => escaped.push_str("&amp;"),
                '<' => escaped.push_str("&lt;"),
                '>' => escaped.push_str("&gt;"),
                '"' => escaped.push_str("&quot;"),
                '\'' => escaped.push_str("&#x27;"),
                _ => escaped.push(ch),
            }
        }

        escaped
    }
}

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
#[template(path = "profile.html")]
pub struct ProfileTemplate {
    pub profile: ProfileHeader,
    pub recent_posts: Vec<ProfilePostRow>,
    pub can_edit: bool,
    pub edit_profile_url: String,
    pub csrf_token: Option<String>,
}

#[derive(Template)]
#[template(path = "edit_profile.html")]
pub struct EditProfileTemplate {
    pub profile: EditProfileContext,
    pub form: EditProfileFormValues,
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
    pub can_moderate: bool,
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

#[derive(Debug, Clone)]
pub struct ProfileHeader {
    pub username: String,
    pub display_name: String,
    pub bio: String,
    pub created_at: DateTime<Utc>,
    pub post_count: i64,
}

#[derive(Debug, Clone)]
pub struct ProfilePostRow {
    pub post_id: i64,
    pub thread_id: i64,
    pub thread_slug: String,
    pub thread_title: String,
    pub body: String,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone)]
pub struct EditProfileContext {
    pub username: String,
}

#[derive(Debug, Clone, Default)]
pub struct EditProfileFormValues {
    pub display_name: String,
    pub bio: String,
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

#[cfg(test)]
mod tests {
    use super::filters::format_body;

    #[test]
    fn format_body_escapes_html() {
        let formatted = format_body(r#"<script>alert("x")</script>"#).expect("formatting should succeed");

        assert_eq!(
            formatted,
            "&lt;script&gt;alert(&quot;x&quot;)&lt;/script&gt;"
        );
    }

    #[test]
    fn format_body_preserves_line_breaks() {
        let formatted = format_body("first line\nsecond line\nthird line")
            .expect("formatting should succeed");

        assert_eq!(formatted, "first line<br>second line<br>third line");
    }
}
