use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::FromRow;

#[allow(dead_code)]
#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct Category {
    pub id: i64,
    pub name: String,
    pub slug: String,
    pub description: String,
    pub position: i32,
    pub created_at: DateTime<Utc>,
}
