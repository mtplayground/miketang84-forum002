use sqlx::{migrate::Migrator, postgres::PgPoolOptions, PgPool};
use tracing::info;

use crate::config::Config;

pub(crate) static MIGRATOR: Migrator = sqlx::migrate!("./migrations");

#[derive(Clone)]
pub struct Db {
    pool: PgPool,
}

impl Db {
    pub async fn connect(config: &Config) -> Result<Self, sqlx::Error> {
        Self::connect_with_url(&config.database_url).await
    }

    pub async fn connect_with_url(database_url: &str) -> Result<Self, sqlx::Error> {
        let pool = PgPoolOptions::new()
            .max_connections(5)
            .connect(database_url)
            .await?;

        Ok(Self { pool })
    }

    pub async fn run_migrations(&self) -> Result<(), sqlx::migrate::MigrateError> {
        MIGRATOR.run(&self.pool).await?;
        info!("database migrations applied");

        Ok(())
    }

    pub fn pool(&self) -> &PgPool {
        &self.pool
    }

    #[cfg(test)]
    pub fn from_pool(pool: PgPool) -> Self {
        Self { pool }
    }
}
