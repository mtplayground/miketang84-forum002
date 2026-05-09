use chrono::{DateTime, Utc};
use sqlx::PgPool;
use uuid::Uuid;

use crate::models::session::Session;

#[derive(Clone)]
pub struct SessionStore {
    pool: PgPool,
}

#[allow(dead_code)]
impl SessionStore {
    pub fn new(pool: &PgPool) -> Self {
        Self { pool: pool.clone() }
    }

    pub async fn create(
        &self,
        user_id: Option<i64>,
        expires_at: DateTime<Utc>,
    ) -> Result<Session, sqlx::Error> {
        let id = Uuid::new_v4();
        let csrf_token = Uuid::new_v4().to_string();

        sqlx::query_as::<_, Session>(
            r#"
            INSERT INTO sessions (id, user_id, csrf_token, expires_at)
            VALUES ($1, $2, $3, $4)
            RETURNING id, user_id, csrf_token, expires_at, created_at
            "#,
        )
        .bind(id)
        .bind(user_id)
        .bind(csrf_token)
        .bind(expires_at)
        .fetch_one(&self.pool)
        .await
    }

    pub async fn load(&self, id: Uuid) -> Result<Option<Session>, sqlx::Error> {
        sqlx::query_as::<_, Session>(
            r#"
            SELECT id, user_id, csrf_token, expires_at, created_at
            FROM sessions
            WHERE id = $1
            "#,
        )
        .bind(id)
        .fetch_optional(&self.pool)
        .await
    }

    pub async fn delete(&self, id: Uuid) -> Result<bool, sqlx::Error> {
        let result = sqlx::query(
            r#"
            DELETE FROM sessions
            WHERE id = $1
            "#,
        )
        .bind(id)
        .execute(&self.pool)
        .await?;

        Ok(result.rows_affected() > 0)
    }
}
