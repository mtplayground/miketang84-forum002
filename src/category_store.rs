use sqlx::PgPool;

use crate::models::category::Category;

#[derive(Debug, Clone)]
pub struct CreateCategoryInput {
    pub name: String,
    pub slug: String,
    pub description: String,
    pub position: i32,
}

#[derive(Debug, Clone)]
pub struct UpdateCategoryInput {
    pub name: String,
    pub slug: String,
    pub description: String,
    pub position: i32,
}

#[derive(Clone)]
pub struct CategoryStore {
    pool: PgPool,
}

#[allow(dead_code)]
impl CategoryStore {
    pub fn new(pool: &PgPool) -> Self {
        Self { pool: pool.clone() }
    }

    pub async fn list(&self) -> Result<Vec<Category>, sqlx::Error> {
        sqlx::query_as::<_, Category>(
            r#"
            SELECT id, name, slug, description, position, created_at
            FROM categories
            ORDER BY position ASC, created_at ASC, id ASC
            "#,
        )
        .fetch_all(&self.pool)
        .await
    }

    pub async fn get_by_slug(&self, slug: &str) -> Result<Option<Category>, sqlx::Error> {
        sqlx::query_as::<_, Category>(
            r#"
            SELECT id, name, slug, description, position, created_at
            FROM categories
            WHERE slug = $1
            "#,
        )
        .bind(slug)
        .fetch_optional(&self.pool)
        .await
    }

    pub async fn create(&self, input: &CreateCategoryInput) -> Result<Category, sqlx::Error> {
        sqlx::query_as::<_, Category>(
            r#"
            INSERT INTO categories (name, slug, description, position)
            VALUES ($1, $2, $3, $4)
            RETURNING id, name, slug, description, position, created_at
            "#,
        )
        .bind(&input.name)
        .bind(&input.slug)
        .bind(&input.description)
        .bind(input.position)
        .fetch_one(&self.pool)
        .await
    }

    pub async fn update(
        &self,
        id: i64,
        input: &UpdateCategoryInput,
    ) -> Result<Option<Category>, sqlx::Error> {
        sqlx::query_as::<_, Category>(
            r#"
            UPDATE categories
            SET name = $2,
                slug = $3,
                description = $4,
                position = $5
            WHERE id = $1
            RETURNING id, name, slug, description, position, created_at
            "#,
        )
        .bind(id)
        .bind(&input.name)
        .bind(&input.slug)
        .bind(&input.description)
        .bind(input.position)
        .fetch_optional(&self.pool)
        .await
    }

    pub async fn delete(&self, id: i64) -> Result<bool, sqlx::Error> {
        let result = sqlx::query(
            r#"
            DELETE FROM categories
            WHERE id = $1
            "#,
        )
        .bind(id)
        .execute(&self.pool)
        .await?;

        Ok(result.rows_affected() > 0)
    }
}
