use chrono::{DateTime, Utc};
use serde::Serialize;
use sqlx::PgPool;

#[derive(Debug, Clone, Serialize, sqlx::FromRow)]
pub struct PublicProfileSummary {
    pub id: i64,
    pub username: String,
    pub display_name: String,
    pub bio: String,
    pub created_at: DateTime<Utc>,
    pub post_count: i64,
}

#[derive(Debug, Clone, Serialize, sqlx::FromRow)]
pub struct PublicProfilePost {
    pub post_id: i64,
    pub thread_id: i64,
    pub thread_slug: String,
    pub thread_title: String,
    pub body: String,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, sqlx::FromRow)]
pub struct EditableProfile {
    pub id: i64,
    pub username: String,
    pub display_name: String,
    pub bio: String,
}

#[derive(Clone)]
pub struct ProfileStore {
    pool: PgPool,
}

#[allow(dead_code)]
impl ProfileStore {
    pub fn new(pool: &PgPool) -> Self {
        Self { pool: pool.clone() }
    }

    pub async fn get_public_profile(
        &self,
        username: &str,
    ) -> Result<Option<PublicProfileSummary>, sqlx::Error> {
        sqlx::query_as::<_, PublicProfileSummary>(
            r#"
            SELECT
                u.id,
                u.username,
                u.display_name,
                u.bio,
                u.created_at,
                COUNT(p.id) AS post_count
            FROM users u
            LEFT JOIN posts p ON p.author_id = u.id AND p.deleted_at IS NULL
            WHERE u.username = $1
            GROUP BY u.id
            "#,
        )
        .bind(username)
        .fetch_optional(&self.pool)
        .await
    }

    pub async fn recent_posts(
        &self,
        user_id: i64,
        limit: i64,
    ) -> Result<Vec<PublicProfilePost>, sqlx::Error> {
        sqlx::query_as::<_, PublicProfilePost>(
            r#"
            SELECT
                p.id AS post_id,
                t.id AS thread_id,
                t.slug AS thread_slug,
                t.title AS thread_title,
                p.body,
                p.created_at
            FROM posts p
            INNER JOIN threads t ON t.id = p.thread_id
            WHERE p.author_id = $1
              AND p.deleted_at IS NULL
            ORDER BY p.created_at DESC, p.id DESC
            LIMIT $2
            "#,
        )
        .bind(user_id)
        .bind(limit)
        .fetch_all(&self.pool)
        .await
    }

    pub async fn get_editable_profile(
        &self,
        user_id: i64,
    ) -> Result<Option<EditableProfile>, sqlx::Error> {
        sqlx::query_as::<_, EditableProfile>(
            r#"
            SELECT
                id,
                username,
                display_name,
                bio
            FROM users
            WHERE id = $1
            "#,
        )
        .bind(user_id)
        .fetch_optional(&self.pool)
        .await
    }

    pub async fn update_profile(
        &self,
        user_id: i64,
        display_name: &str,
        bio: &str,
    ) -> Result<bool, sqlx::Error> {
        let rows_affected = sqlx::query(
            r#"
            UPDATE users
            SET display_name = $2,
                bio = $3
            WHERE id = $1
            "#,
        )
        .bind(user_id)
        .bind(display_name)
        .bind(bio)
        .execute(&self.pool)
        .await?
        .rows_affected();

        Ok(rows_affected > 0)
    }
}
