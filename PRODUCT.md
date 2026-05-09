# PRODUCT

## What this project is

`miketang84-forum002` is a server-rendered Rust forum application built with Axum, Askama, sqlx, PostgreSQL, and Tailwind CSS. It is a classic threaded discussion app with user accounts, moderation tools, search, and a seed/bootstrap workflow for new environments.

## What it currently does

- User registration, login, logout, signed session cookies, and CSRF protection
- Anonymous navigation exposes `Register` / `Login`, while authenticated sessions get `Logout`
- Role-aware access control for `user`, `moderator`, and `admin`
- Category listing and admin category management
- Thread creation, paginated thread/category views, replies, and public user profiles
- Self-service post edit/delete within a configured time window
- Moderator actions for lock/unlock, pin/unpin, post delete, and thread delete
- Admin user management with role assignment and self-demotion protection
- PostgreSQL full-text search across thread titles and post bodies
- Seed binary for admin bootstrap plus starter categories/threads
- Fresh deployments are expected to be bootstrapped by running the seed binary once
- Docker Compose flow for app + PostgreSQL local development

## Product and data conventions

- PostgreSQL is the only persistent store.
- Migrations are embedded and run automatically at startup and during seeding.
- Sessions are database-backed rather than in-memory.
- Deleted posts are soft-deleted and rendered as placeholders.
- Deleted threads are hidden from listings and show a removed page on direct access.
- Search uses PostgreSQL `tsvector` + GIN indexes.

## Architectural shape

- `src/main.rs`: HTTP routes, handlers, and application assembly
- `src/auth.rs`: session middleware, CSRF checks, and auth/role extractors
- `src/*_store.rs`: database access by domain (`category`, `thread`, `session`, `profile`, `search`)
- `src/templates.rs` + `templates/`: Askama view models and HTML templates
- `migrations/`: schema history for users, sessions, categories, threads, posts, soft-delete, CSRF, and search
- `src/bin/seed.rs`: admin bootstrap and sample data seeding

## Runtime contract

- Required app env vars: `DATABASE_URL`, `BIND_ADDR`, `SESSION_SECRET`, `EDIT_WINDOW_MINUTES`
- Optional operational env vars: `RUST_LOG`, `SEED_ADMIN_*`
- Default HTTP bind is expected to be `0.0.0.0:8080`
- Static assets are served from `/static`
- Health check endpoint is `/health`

## Conventions for future work

- Prefer typed errors and friendly rendered error pages over raw status responses.
- Keep server-rendered HTML as the primary UI model; avoid adding client-side state unless it clearly pays for itself.
- Reuse the existing store/template split instead of putting SQL directly in handlers.
- Preserve PostgreSQL-first assumptions; do not add alternate persistence layers.
