use chrono::{DateTime, Utc};
use serde::Serialize;
use sqlx::PgPool;

use crate::models::thread::Thread;

#[derive(Debug, Clone, Serialize, sqlx::FromRow)]
pub struct ThreadListItem {
    pub id: i64,
    pub category_id: i64,
    pub author_id: i64,
    pub title: String,
    pub slug: String,
    pub is_locked: bool,
    pub is_pinned: bool,
    pub created_at: DateTime<Utc>,
    pub last_activity_at: DateTime<Utc>,
    pub author_username: String,
    pub reply_count: i64,
}

#[derive(Debug, Clone, Serialize, sqlx::FromRow)]
pub struct ThreadDetail {
    pub id: i64,
    pub category_id: i64,
    pub category_name: String,
    pub category_slug: String,
    pub author_id: i64,
    pub author_username: String,
    pub title: String,
    pub slug: String,
    pub is_locked: bool,
    pub is_pinned: bool,
    pub created_at: DateTime<Utc>,
    pub last_activity_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, sqlx::FromRow)]
pub struct ThreadPostItem {
    pub id: i64,
    pub thread_id: i64,
    pub author_id: i64,
    pub author_username: String,
    pub body: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ThreadListPage {
    pub threads: Vec<ThreadListItem>,
    pub total_threads: i64,
    pub current_page: i64,
    pub total_pages: i64,
    pub per_page: i64,
}

#[derive(Debug, Clone, Serialize)]
pub struct ThreadPostsPage {
    pub posts: Vec<ThreadPostItem>,
    pub total_posts: i64,
    pub current_page: i64,
    pub total_pages: i64,
    pub per_page: i64,
}

#[derive(Debug, Clone)]
pub struct CreateThreadInput {
    pub category_id: i64,
    pub author_id: i64,
    pub title: String,
    pub slug: String,
    pub body: String,
}

#[derive(Clone)]
pub struct ThreadStore {
    pool: PgPool,
}

#[allow(dead_code)]
impl ThreadStore {
    pub fn new(pool: &PgPool) -> Self {
        Self { pool: pool.clone() }
    }

    pub async fn list_by_category(
        &self,
        category_id: i64,
        requested_page: i64,
        per_page: i64,
    ) -> Result<ThreadListPage, sqlx::Error> {
        let total_threads = sqlx::query_scalar::<_, i64>(
            r#"
            SELECT COUNT(*)
            FROM threads
            WHERE category_id = $1
            "#,
        )
        .bind(category_id)
        .fetch_one(&self.pool)
        .await?;

        let total_pages = if total_threads == 0 {
            1
        } else {
            ((total_threads - 1) / per_page) + 1
        };
        let current_page = requested_page.clamp(1, total_pages);
        let offset = (current_page - 1) * per_page;

        let threads = sqlx::query_as::<_, ThreadListItem>(
            r#"
            SELECT
                t.id,
                t.category_id,
                t.author_id,
                t.title,
                t.slug,
                t.is_locked,
                t.is_pinned,
                t.created_at,
                t.last_activity_at,
                u.username AS author_username,
                GREATEST(COUNT(p.id) - 1, 0) AS reply_count
            FROM threads t
            INNER JOIN users u ON u.id = t.author_id
            LEFT JOIN posts p ON p.thread_id = t.id AND p.deleted_at IS NULL
            WHERE t.category_id = $1
            GROUP BY t.id, u.username
            ORDER BY t.is_pinned DESC, t.last_activity_at DESC, t.id DESC
            LIMIT $2
            OFFSET $3
            "#,
        )
        .bind(category_id)
        .bind(per_page)
        .bind(offset)
        .fetch_all(&self.pool)
        .await?;

        Ok(ThreadListPage {
            threads,
            total_threads,
            current_page,
            total_pages,
            per_page,
        })
    }

    pub async fn create_thread_with_initial_post(
        &self,
        input: &CreateThreadInput,
    ) -> Result<Thread, sqlx::Error> {
        let mut tx = self.pool.begin().await?;

        let thread = sqlx::query_as::<_, Thread>(
            r#"
            INSERT INTO threads (category_id, author_id, title, slug)
            VALUES ($1, $2, $3, $4)
            RETURNING
                id,
                category_id,
                author_id,
                title,
                slug,
                is_locked,
                is_pinned,
                created_at,
                last_activity_at
            "#,
        )
        .bind(input.category_id)
        .bind(input.author_id)
        .bind(&input.title)
        .bind(&input.slug)
        .fetch_one(&mut *tx)
        .await?;

        sqlx::query(
            r#"
            INSERT INTO posts (thread_id, author_id, body)
            VALUES ($1, $2, $3)
            "#,
        )
        .bind(thread.id)
        .bind(input.author_id)
        .bind(&input.body)
        .execute(&mut *tx)
        .await?;

        tx.commit().await?;

        Ok(thread)
    }

    pub async fn get_thread_detail(&self, id: i64) -> Result<Option<ThreadDetail>, sqlx::Error> {
        sqlx::query_as::<_, ThreadDetail>(
            r#"
            SELECT
                t.id,
                t.category_id,
                c.name AS category_name,
                c.slug AS category_slug,
                t.author_id,
                u.username AS author_username,
                t.title,
                t.slug,
                t.is_locked,
                t.is_pinned,
                t.created_at,
                t.last_activity_at
            FROM threads t
            INNER JOIN categories c ON c.id = t.category_id
            INNER JOIN users u ON u.id = t.author_id
            WHERE t.id = $1
            "#,
        )
        .bind(id)
        .fetch_optional(&self.pool)
        .await
    }

    pub async fn list_posts_for_thread(
        &self,
        thread_id: i64,
        requested_page: i64,
        per_page: i64,
    ) -> Result<ThreadPostsPage, sqlx::Error> {
        let total_posts = sqlx::query_scalar::<_, i64>(
            r#"
            SELECT COUNT(*)
            FROM posts
            WHERE thread_id = $1
              AND deleted_at IS NULL
            "#,
        )
        .bind(thread_id)
        .fetch_one(&self.pool)
        .await?;

        let total_pages = if total_posts == 0 {
            1
        } else {
            ((total_posts - 1) / per_page) + 1
        };
        let current_page = requested_page.clamp(1, total_pages);
        let offset = (current_page - 1) * per_page;

        let posts = sqlx::query_as::<_, ThreadPostItem>(
            r#"
            SELECT
                p.id,
                p.thread_id,
                p.author_id,
                u.username AS author_username,
                p.body,
                p.created_at,
                p.updated_at
            FROM posts p
            INNER JOIN users u ON u.id = p.author_id
            WHERE p.thread_id = $1
              AND p.deleted_at IS NULL
            ORDER BY p.created_at ASC, p.id ASC
            LIMIT $2
            OFFSET $3
            "#,
        )
        .bind(thread_id)
        .bind(per_page)
        .bind(offset)
        .fetch_all(&self.pool)
        .await?;

        Ok(ThreadPostsPage {
            posts,
            total_posts,
            current_page,
            total_pages,
            per_page,
        })
    }
}
