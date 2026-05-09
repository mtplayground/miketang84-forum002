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
    pub deleted_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Serialize, sqlx::FromRow)]
pub struct PostDetail {
    pub id: i64,
    pub thread_id: i64,
    pub author_id: i64,
    pub author_username: String,
    pub body: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub thread_title: String,
    pub thread_slug: String,
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

#[derive(Debug, Clone, Serialize)]
pub struct ReplyCreateResult {
    pub post_id: i64,
    pub total_posts: i64,
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
                p.updated_at,
                p.deleted_at
            FROM posts p
            INNER JOIN users u ON u.id = p.author_id
            WHERE p.thread_id = $1
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

    pub async fn create_reply(
        &self,
        thread_id: i64,
        author_id: i64,
        body: &str,
    ) -> Result<ReplyCreateResult, sqlx::Error> {
        let mut tx = self.pool.begin().await?;

        let (post_id, created_at) = sqlx::query_as::<_, (i64, DateTime<Utc>)>(
            r#"
            INSERT INTO posts (thread_id, author_id, body)
            VALUES ($1, $2, $3)
            RETURNING id, created_at
            "#,
        )
        .bind(thread_id)
        .bind(author_id)
        .bind(body)
        .fetch_one(&mut *tx)
        .await?;

        sqlx::query(
            r#"
            UPDATE threads
            SET last_activity_at = $2
            WHERE id = $1
            "#,
        )
        .bind(thread_id)
        .bind(created_at)
        .execute(&mut *tx)
        .await?;

        let total_posts = sqlx::query_scalar::<_, i64>(
            r#"
            SELECT COUNT(*)
            FROM posts
            WHERE thread_id = $1
            "#,
        )
        .bind(thread_id)
        .fetch_one(&mut *tx)
        .await?;

        tx.commit().await?;

        Ok(ReplyCreateResult {
            post_id,
            total_posts,
        })
    }

    pub async fn get_post_detail(&self, post_id: i64) -> Result<Option<PostDetail>, sqlx::Error> {
        sqlx::query_as::<_, PostDetail>(
            r#"
            SELECT
                p.id,
                p.thread_id,
                p.author_id,
                u.username AS author_username,
                p.body,
                p.created_at,
                p.updated_at,
                t.title AS thread_title,
                t.slug AS thread_slug
            FROM posts p
            INNER JOIN users u ON u.id = p.author_id
            INNER JOIN threads t ON t.id = p.thread_id
            WHERE p.id = $1
              AND p.deleted_at IS NULL
            "#,
        )
        .bind(post_id)
        .fetch_optional(&self.pool)
        .await
    }

    pub async fn update_post_body(&self, post_id: i64, body: &str) -> Result<bool, sqlx::Error> {
        let result = sqlx::query(
            r#"
            UPDATE posts
            SET body = $2,
                updated_at = NOW()
            WHERE id = $1
              AND deleted_at IS NULL
            "#,
        )
        .bind(post_id)
        .bind(body)
        .execute(&self.pool)
        .await?;

        Ok(result.rows_affected() > 0)
    }

    pub async fn page_for_post(&self, post_id: i64, per_page: i64) -> Result<Option<i64>, sqlx::Error> {
        sqlx::query_scalar::<_, i64>(
            r#"
            SELECT ((COUNT(*) - 1) / $2) + 1
            FROM posts target
            INNER JOIN posts p ON p.thread_id = target.thread_id
            WHERE target.id = $1
              AND (
                  p.created_at < target.created_at
                  OR (p.created_at = target.created_at AND p.id <= target.id)
              )
            "#,
        )
        .bind(post_id)
        .bind(per_page)
        .fetch_optional(&self.pool)
        .await
    }

    pub async fn soft_delete_post(&self, post_id: i64) -> Result<bool, sqlx::Error> {
        let result = sqlx::query(
            r#"
            UPDATE posts
            SET deleted_at = NOW()
            WHERE id = $1
              AND deleted_at IS NULL
            "#,
        )
        .bind(post_id)
        .execute(&self.pool)
        .await?;

        Ok(result.rows_affected() > 0)
    }

    pub async fn set_locked(&self, thread_id: i64, is_locked: bool) -> Result<bool, sqlx::Error> {
        let result = sqlx::query(
            r#"
            UPDATE threads
            SET is_locked = $2
            WHERE id = $1
            "#,
        )
        .bind(thread_id)
        .bind(is_locked)
        .execute(&self.pool)
        .await?;

        Ok(result.rows_affected() > 0)
    }

    pub async fn set_pinned(&self, thread_id: i64, is_pinned: bool) -> Result<bool, sqlx::Error> {
        let result = sqlx::query(
            r#"
            UPDATE threads
            SET is_pinned = $2
            WHERE id = $1
            "#,
        )
        .bind(thread_id)
        .bind(is_pinned)
        .execute(&self.pool)
        .await?;

        Ok(result.rows_affected() > 0)
    }
}
