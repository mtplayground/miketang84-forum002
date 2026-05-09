# miketang84-forum002

Rust forum application scaffolded with Axum and Tokio.

## Requirements

- Rust toolchain
- PostgreSQL connection string provided through `DATABASE_URL`

## Run locally

```bash
export DATABASE_URL=$(cat /workspace/.database_url)
export SESSION_SECRET=dev-only-session-secret
export EDIT_WINDOW_MINUTES=15
cargo build
BIND_ADDR=0.0.0.0:8080 cargo run
```

The server exposes:

- `GET /`
- `GET /health`

Environment variables:

- `DATABASE_URL`: PostgreSQL connection string
- `BIND_ADDR`: bind address for the HTTP server, for example `0.0.0.0:8080`
- `SESSION_SECRET`: secret used for session signing/encryption
- `EDIT_WINDOW_MINUTES`: post edit window length in minutes
- `RUST_LOG`: tracing filter, optional
- `SEED_ADMIN_USERNAME`: admin username used by `cargo run --bin seed`
- `SEED_ADMIN_PASSWORD`: admin password used by `cargo run --bin seed`
- `SEED_ADMIN_DISPLAY_NAME`: optional display name for the seeded admin user
- `SEED_ADMIN_BIO`: optional bio for the seeded admin user

Copy [`.env.example`](/workspace/.env.example) to `.env` for local development if preferred.

## Seed sample data

To bootstrap a local or fresh database with an admin account, starter categories, and sample threads:

```bash
export DATABASE_URL=$(cat /workspace/.database_url)
export SEED_ADMIN_USERNAME=admin
export SEED_ADMIN_PASSWORD=change-me-now
cargo run --bin seed
```

The seed binary will:

- run pending migrations
- upsert the admin account and force its role to `admin`
- create or update the starter categories
- create sample threads and one sample reply per thread if they do not already exist

`SEED_ADMIN_DISPLAY_NAME` and `SEED_ADMIN_BIO` are optional; defaults are used when omitted.

## Run with Docker Compose

The repository includes a multi-stage [Dockerfile](/workspace/Dockerfile) using `cargo-chef` for build caching and a local [docker-compose.yml](/workspace/docker-compose.yml) for the app plus PostgreSQL.

```bash
docker compose up --build
```

This starts:

- `postgres` on `localhost:5432` with a named `postgres-data` volume
- `app` on `http://localhost:8080`

The compose file provides local development defaults for:

- `DATABASE_URL=postgresql://forum:forum@postgres:5432/forum002`
- `BIND_ADDR=0.0.0.0:8080`
- `SESSION_SECRET=local-development-secret`
- `EDIT_WINDOW_MINUTES=15`

To stop the stack:

```bash
docker compose down
```

To remove the PostgreSQL volume as well:

```bash
docker compose down --volumes
```
