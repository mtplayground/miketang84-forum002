use chrono::{DateTime, Utc};
use serde::Serialize;
use sqlx::PgPool;

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

#[derive(Debug, Clone, Serialize)]
pub struct ThreadListPage {
    pub threads: Vec<ThreadListItem>,
    pub total_threads: i64,
    pub current_page: i64,
    pub total_pages: i64,
    pub per_page: i64,
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
}
