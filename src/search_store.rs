use chrono::{DateTime, Utc};
use serde::Serialize;
use sqlx::PgPool;

#[derive(Debug, Clone, Serialize, sqlx::FromRow)]
pub struct SearchResultItem {
    pub result_kind: String,
    pub thread_id: i64,
    pub thread_slug: String,
    pub thread_title: String,
    pub post_id: Option<i64>,
    pub body: Option<String>,
    pub created_at: DateTime<Utc>,
    pub rank: f64,
}

#[derive(Debug, Clone, Serialize)]
pub struct SearchResultsPage {
    pub results: Vec<SearchResultItem>,
    pub total_results: i64,
    pub current_page: i64,
    pub total_pages: i64,
    pub per_page: i64,
}

#[derive(Clone)]
pub struct SearchStore {
    pool: PgPool,
}

impl SearchStore {
    pub fn new(pool: &PgPool) -> Self {
        Self { pool: pool.clone() }
    }

    pub async fn search(
        &self,
        query_text: &str,
        requested_page: i64,
        per_page: i64,
    ) -> Result<SearchResultsPage, sqlx::Error> {
        let total_results = sqlx::query_scalar::<_, i64>(
            r#"
            WITH query AS (
                SELECT plainto_tsquery('english', $1) AS q
            )
            SELECT COUNT(*)
            FROM (
                SELECT t.id
                FROM threads t
                CROSS JOIN query
                WHERE t.deleted_at IS NULL
                  AND t.search_tsv @@ query.q

                UNION ALL

                SELECT p.id
                FROM posts p
                INNER JOIN threads t ON t.id = p.thread_id
                CROSS JOIN query
                WHERE p.deleted_at IS NULL
                  AND t.deleted_at IS NULL
                  AND p.search_tsv @@ query.q
            ) AS search_hits
            "#,
        )
        .bind(query_text)
        .fetch_one(&self.pool)
        .await?;

        let total_pages = if total_results == 0 {
            1
        } else {
            ((total_results - 1) / per_page) + 1
        };
        let current_page = requested_page.clamp(1, total_pages);
        let offset = (current_page - 1) * per_page;

        let results = sqlx::query_as::<_, SearchResultItem>(
            r#"
            WITH query AS (
                SELECT plainto_tsquery('english', $1) AS q
            )
            SELECT
                result_kind,
                thread_id,
                thread_slug,
                thread_title,
                post_id,
                body,
                created_at,
                rank
            FROM (
                SELECT
                    'thread'::text AS result_kind,
                    t.id AS thread_id,
                    t.slug AS thread_slug,
                    t.title AS thread_title,
                    NULL::BIGINT AS post_id,
                    NULL::TEXT AS body,
                    t.created_at,
                    ts_rank(t.search_tsv, query.q)::double precision AS rank
                FROM threads t
                CROSS JOIN query
                WHERE t.deleted_at IS NULL
                  AND t.search_tsv @@ query.q

                UNION ALL

                SELECT
                    'post'::text AS result_kind,
                    t.id AS thread_id,
                    t.slug AS thread_slug,
                    t.title AS thread_title,
                    p.id AS post_id,
                    p.body,
                    p.created_at,
                    ts_rank(p.search_tsv, query.q)::double precision AS rank
                FROM posts p
                INNER JOIN threads t ON t.id = p.thread_id
                CROSS JOIN query
                WHERE p.deleted_at IS NULL
                  AND t.deleted_at IS NULL
                  AND p.search_tsv @@ query.q
            ) AS search_results
            ORDER BY rank DESC, created_at DESC, thread_id DESC, post_id DESC NULLS FIRST
            LIMIT $2
            OFFSET $3
            "#,
        )
        .bind(query_text)
        .bind(per_page)
        .bind(offset)
        .fetch_all(&self.pool)
        .await?;

        Ok(SearchResultsPage {
            results,
            total_results,
            current_page,
            total_pages,
            per_page,
        })
    }
}
