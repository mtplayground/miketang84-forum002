use std::{collections::HashMap, env, error::Error, fmt};

use miketang84_forum002::{
    category_store::{CategoryStore, CreateCategoryInput, UpdateCategoryInput},
    db::Db,
    models::{
        category::Category,
        user::{Role, User},
    },
    password::{hash_password, PasswordError},
    thread_store::{CreateThreadInput, ThreadStore},
};
use sqlx::PgPool;
use tracing::info;

const SAMPLE_CATEGORIES: &[CategorySeed] = &[
    CategorySeed {
        name: "Announcements",
        slug: "announcements",
        description: "Project news, maintenance windows, and release notes.",
        position: 0,
    },
    CategorySeed {
        name: "General Discussion",
        slug: "general-discussion",
        description: "Open discussion for introductions, questions, and community chatter.",
        position: 1,
    },
];

const SAMPLE_THREADS: &[ThreadSeed] = &[
    ThreadSeed {
        category_slug: "announcements",
        title: "Welcome to the forum",
        slug: "welcome-to-the-forum",
        body: "This forum is ready for its first conversations.\n\nUse announcements for important updates from the team.",
        reply_body: Some("Reply here once you have the app running locally and want to confirm everything looks healthy."),
    },
    ThreadSeed {
        category_slug: "general-discussion",
        title: "Introduce yourself",
        slug: "introduce-yourself",
        body: "Start here with a short introduction.\n\nWhat are you building with the forum?",
        reply_body: Some("A quick hello and a note about what you want to discuss is enough to get started."),
    },
];

#[tokio::main]
async fn main() -> Result<(), SeedError> {
    init_tracing();
    load_env();

    let config = SeedConfig::from_env()?;
    let db = Db::connect_with_url(&config.database_url).await?;
    db.run_migrations().await?;

    let admin = upsert_admin(db.pool(), &config).await?;
    let categories = ensure_categories(db.pool()).await?;
    let thread_store = ThreadStore::new(db.pool());

    for sample_thread in SAMPLE_THREADS {
        let category = categories.get(sample_thread.category_slug).ok_or_else(|| {
            SeedError::Data(format!(
                "sample category {} was not created before thread seeding",
                sample_thread.category_slug
            ))
        })?;
        let thread_id = ensure_thread(&thread_store, db.pool(), category.id, admin.id, sample_thread).await?;

        if let Some(reply_body) = sample_thread.reply_body {
            ensure_sample_reply(&thread_store, db.pool(), thread_id, admin.id, reply_body).await?;
        }
    }

    info!(
        admin_username = %admin.username,
        admin_id = admin.id,
        category_count = categories.len(),
        thread_count = SAMPLE_THREADS.len(),
        "seed data applied"
    );

    println!(
        "Seed complete. Admin user '{}' is ready and sample forum content has been created.",
        admin.username
    );

    Ok(())
}

#[derive(Debug)]
struct SeedConfig {
    database_url: String,
    admin_username: String,
    admin_password: String,
    admin_display_name: String,
    admin_bio: String,
}

impl SeedConfig {
    fn from_env() -> Result<Self, SeedError> {
        Ok(Self {
            database_url: required_var("DATABASE_URL")?,
            admin_username: required_var("SEED_ADMIN_USERNAME")?.trim().to_lowercase(),
            admin_password: required_var("SEED_ADMIN_PASSWORD")?,
            admin_display_name: env::var("SEED_ADMIN_DISPLAY_NAME")
                .unwrap_or_else(|_| "Forum Admin".to_string())
                .trim()
                .to_string(),
            admin_bio: env::var("SEED_ADMIN_BIO")
                .unwrap_or_else(|_| "Bootstrapped administrative account.".to_string())
                .trim()
                .to_string(),
        })
    }
}

#[derive(Debug)]
struct CategorySeed {
    name: &'static str,
    slug: &'static str,
    description: &'static str,
    position: i32,
}

#[derive(Debug)]
struct ThreadSeed {
    category_slug: &'static str,
    title: &'static str,
    slug: &'static str,
    body: &'static str,
    reply_body: Option<&'static str>,
}

#[derive(Debug)]
enum SeedError {
    MissingVar {
        name: &'static str,
        source: env::VarError,
    },
    InvalidAdminUsername,
    InvalidAdminPassword,
    Database(sqlx::Error),
    Migration(sqlx::migrate::MigrateError),
    Password(PasswordError),
    Data(String),
}

impl fmt::Display for SeedError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MissingVar { name, .. } => write!(f, "missing required environment variable {name}"),
            Self::InvalidAdminUsername => {
                write!(f, "SEED_ADMIN_USERNAME must be 3-32 chars of lowercase letters, numbers, or underscores")
            }
            Self::InvalidAdminPassword => {
                write!(f, "SEED_ADMIN_PASSWORD must be at least 8 characters long")
            }
            Self::Database(err) => write!(f, "database operation failed: {err}"),
            Self::Migration(err) => write!(f, "migration failed: {err}"),
            Self::Password(err) => write!(f, "password hashing failed: {err}"),
            Self::Data(message) => write!(f, "{message}"),
        }
    }
}

impl Error for SeedError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::MissingVar { source, .. } => Some(source),
            Self::Database(err) => Some(err),
            Self::Migration(err) => Some(err),
            Self::Password(err) => Some(err),
            Self::InvalidAdminUsername | Self::InvalidAdminPassword | Self::Data(_) => None,
        }
    }
}

impl From<sqlx::Error> for SeedError {
    fn from(value: sqlx::Error) -> Self {
        Self::Database(value)
    }
}

impl From<sqlx::migrate::MigrateError> for SeedError {
    fn from(value: sqlx::migrate::MigrateError) -> Self {
        Self::Migration(value)
    }
}

impl From<PasswordError> for SeedError {
    fn from(value: PasswordError) -> Self {
        Self::Password(value)
    }
}

async fn upsert_admin(pool: &PgPool, config: &SeedConfig) -> Result<User, SeedError> {
    validate_admin_credentials(config)?;
    let password_hash = hash_password(&config.admin_password)?;

    let user = sqlx::query_as::<_, User>(
        r#"
        INSERT INTO users (username, password_hash, display_name, bio, role)
        VALUES ($1, $2, $3, $4, $5)
        ON CONFLICT (username)
        DO UPDATE SET
            password_hash = EXCLUDED.password_hash,
            display_name = EXCLUDED.display_name,
            bio = EXCLUDED.bio,
            role = EXCLUDED.role
        RETURNING id, username, password_hash, display_name, bio, role, created_at
        "#,
    )
    .bind(&config.admin_username)
    .bind(&password_hash)
    .bind(&config.admin_display_name)
    .bind(&config.admin_bio)
    .bind(Role::Admin)
    .fetch_one(pool)
    .await?;

    Ok(user)
}

async fn ensure_categories(pool: &PgPool) -> Result<HashMap<&'static str, Category>, SeedError> {
    let store = CategoryStore::new(pool);
    let mut categories = HashMap::with_capacity(SAMPLE_CATEGORIES.len());

    for seed in SAMPLE_CATEGORIES {
        let category = if let Some(existing) = store.get_by_slug(seed.slug).await? {
            store
                .update(
                    existing.id,
                    &UpdateCategoryInput {
                        name: seed.name.to_string(),
                        slug: seed.slug.to_string(),
                        description: seed.description.to_string(),
                        position: seed.position,
                    },
                )
                .await?
                .ok_or_else(|| SeedError::Data(format!("category {} disappeared during update", seed.slug)))?
        } else {
            store
                .create(&CreateCategoryInput {
                    name: seed.name.to_string(),
                    slug: seed.slug.to_string(),
                    description: seed.description.to_string(),
                    position: seed.position,
                })
                .await?
        };

        categories.insert(seed.slug, category);
    }

    Ok(categories)
}

async fn ensure_thread(
    thread_store: &ThreadStore,
    pool: &PgPool,
    category_id: i64,
    author_id: i64,
    seed: &ThreadSeed,
) -> Result<i64, SeedError> {
    let existing_thread_id = sqlx::query_scalar::<_, i64>(
        r#"
        SELECT id
        FROM threads
        WHERE category_id = $1
          AND slug = $2
          AND deleted_at IS NULL
        ORDER BY id ASC
        LIMIT 1
        "#,
    )
    .bind(category_id)
    .bind(seed.slug)
    .fetch_optional(pool)
    .await?;

    if let Some(thread_id) = existing_thread_id {
        return Ok(thread_id);
    }

    let thread = thread_store
        .create_thread_with_initial_post(&CreateThreadInput {
            category_id,
            author_id,
            title: seed.title.to_string(),
            slug: seed.slug.to_string(),
            body: seed.body.to_string(),
        })
        .await?;

    Ok(thread.id)
}

async fn ensure_sample_reply(
    thread_store: &ThreadStore,
    pool: &PgPool,
    thread_id: i64,
    author_id: i64,
    reply_body: &str,
) -> Result<(), SeedError> {
    let existing_posts = sqlx::query_scalar::<_, i64>(
        r#"
        SELECT COUNT(*)
        FROM posts
        WHERE thread_id = $1
        "#,
    )
    .bind(thread_id)
    .fetch_one(pool)
    .await?;

    if existing_posts < 2 {
        thread_store.create_reply(thread_id, author_id, reply_body).await?;
    }

    Ok(())
}

fn validate_admin_credentials(config: &SeedConfig) -> Result<(), SeedError> {
    let username = config.admin_username.as_str();

    if !(3..=32).contains(&username.len())
        || !username
            .chars()
            .all(|ch| ch.is_ascii_lowercase() || ch.is_ascii_digit() || ch == '_')
    {
        return Err(SeedError::InvalidAdminUsername);
    }

    if config.admin_password.len() < 8 {
        return Err(SeedError::InvalidAdminPassword);
    }

    Ok(())
}

fn load_env() {
    dotenvy::dotenv().ok();
    dotenvy::from_filename_override(".env.production").ok();
}

fn required_var(name: &'static str) -> Result<String, SeedError> {
    env::var(name).map_err(|source| SeedError::MissingVar { name, source })
}

fn init_tracing() {
    tracing_subscriber::fmt::init();
}
